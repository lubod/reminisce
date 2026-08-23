//! Disaster recovery CLI for Reminisce.
//!
//! Handles the main data-loss scenarios against the P2P backup mesh:
//!
//!   * `list` — show available DB snapshots (from on-disk manifests).
//!   * `db` — restore the database from a P2P snapshot (full DB loss).
//!   * `media` — restore media files from P2P (DB intact): one file, all missing
//!     files, or the entire library.
//!   * `full` — full recovery: restore the DB snapshot, then the media files.
//!
//! The `db` command reconstructs + decrypts the dump, writes it to a file, and can
//! run `pg_restore`. The restore client authenticates to storage nodes with the home
//! server's P2P identity (`<p2p_data_dir>/node.key`) — required for the storage nodes
//! to accept RetrieveShard tokens (they pin `allowed_owner_id` to the home server).

use clap::{Parser, Subcommand};
use deadpool_postgres::Pool;
use np2p::crypto::NodeIdentity;
use np2p::network::{Message, P2PService};
use reminisce::config::Config;
use reminisce::db_restore::{self, DbBackupManifest};
use reminisce::p2p_restore::restore_file;
use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::sync::Arc;

#[derive(Parser)]
#[command(name = "disaster_recovery", about = "Restore Reminisce DB snapshots and media from P2P backup")]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Parser)]
struct CommonOpts {
    #[arg(long, help = "Path to config.yaml (provides api_secret_key, p2p_data_dir, dirs, database_url)")]
    config: String,

    #[arg(long = "node", help = "Storage node override 'node_id=host:port' (repeatable)", value_parser = parse_node)]
    nodes: Vec<(String, SocketAddr)>,
}

#[derive(Subcommand)]
enum Command {
    /// List available DB snapshots from on-disk manifests.
    List {
        #[command(flatten)]
        common: CommonOpts,
    },
    /// Restore the database from a P2P snapshot (full DB loss scenario).
    Db {
        #[command(flatten)]
        common: CommonOpts,
        #[arg(long, help = "Snapshot backup_hash to restore (default: latest)")]
        backup_hash: Option<String>,
        #[arg(long, default_value = "./db_restore.dump", help = "Output path for the reconstructed dump")]
        out: String,
        #[arg(long, help = "Run pg_restore into the target database after reconstructing the dump")]
        pg_restore: bool,
        #[arg(long, help = "Target database URL for pg_restore (e.g. postgres://user:pass@host:5432/reminisce_db)")]
        target_db_url: Option<String>,
        #[arg(long, help = "Pass --clean --if-exists to pg_restore (drops existing objects first)")]
        clean: bool,
        #[arg(long, help = "Fetch the restore manifest from the P2P mesh (use after a full disk loss; skips local manifests)")]
        from_mesh: bool,
    },
    /// Restore media files from P2P (database intact scenario).
    Media {
        #[command(flatten)]
        common: CommonOpts,
        #[arg(long, help = "Restore a single file by hash")]
        hash: Option<String>,
        #[arg(long, help = "Restore every file whose copy on disk is missing")]
        all_missing: bool,
        #[arg(long, help = "Restore every file in the library (even if present on disk)")]
        all: bool,
        #[arg(long, help = "Override the database URL (default: from config)")]
        db_url: Option<String>,
        #[arg(long, help = "Max number of files to restore (0 = no limit)")]
        limit: Option<usize>,
    },
    /// Full recovery: restore the DB snapshot, then the media files.
    Full {
        #[command(flatten)]
        common: CommonOpts,
        #[arg(long, help = "Snapshot backup_hash to restore (default: latest)")]
        backup_hash: Option<String>,
        #[arg(long, default_value = "./db_restore.dump", help = "Output path for the reconstructed dump")]
        out: String,
        #[arg(long, help = "Run pg_restore into the target database after reconstructing the dump")]
        pg_restore: bool,
        #[arg(long, help = "Target database URL for pg_restore")]
        target_db_url: Option<String>,
        #[arg(long, help = "Pass --clean --if-exists to pg_restore")]
        clean: bool,
        #[arg(long, help = "Fetch the restore manifest from the P2P mesh (use after a full disk loss)")]
        from_mesh: bool,
        #[arg(long, help = "Also restore media files that are missing on disk (default: true)")]
        restore_media: Option<bool>,
    },
}

fn parse_node(s: &str) -> Result<(String, SocketAddr), String> {
    let (id, addr) = s.split_once('=').ok_or("expected node_id=host:port")?;
    let addr = addr.parse::<SocketAddr>().map_err(|e| format!("invalid addr '{}': {}", addr, e))?;
    Ok((id.to_string(), addr))
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    tracing_subscriber::fmt::init();
    let cli = Cli::parse();

    match cli.command {
        Command::List { common } => cmd_list(&common).await,
        Command::Db { common, backup_hash, out, pg_restore, target_db_url, clean, from_mesh } => {
            cmd_db(&common, backup_hash, &out, pg_restore, target_db_url, clean, from_mesh).await.map(|_| ())
        }
        Command::Media { common, hash, all_missing, all, db_url, limit } => {
            cmd_media(&common, hash, all_missing, all, db_url, limit).await
        }
        Command::Full { common, backup_hash, out, pg_restore, target_db_url, clean, from_mesh, restore_media } => {
            cmd_full(&common, backup_hash, &out, pg_restore, target_db_url, clean, from_mesh, restore_media.unwrap_or(true)).await
        }
    }
}

// ── Shared helpers ────────────────────────────────────────────────────────────

fn load_config(common: &CommonOpts) -> Result<Config, Box<dyn std::error::Error + Send + Sync>> {
    Config::from_file(&common.config).map_err(|e| format!("failed to load config {}: {}", common.config, e).into())
}

/// Load the home server's P2P identity. Prefers `<p2p_data_dir>/node.key`; if it's
/// absent (full disk loss) and `p2p_deterministic_identity` was used, the identity is
/// re-derived from the api_secret so storage nodes still accept our tokens.
fn load_identity(
    p2p_data_dir: &str,
    api_secret: &str,
) -> Result<NodeIdentity, Box<dyn std::error::Error + Send + Sync>> {
    let path = PathBuf::from(p2p_data_dir).join("node.key");
    if path.exists() {
        let bytes = std::fs::read(&path)
            .map_err(|e| format!("cannot read P2P identity {}: {}", path.display(), e))?;
        return Ok(NodeIdentity::from_secret_bytes(&bytes)?);
    }
    eprintln!("node.key not found at {} — deriving P2P identity from api_secret (deterministic)", path.display());
    Ok(NodeIdentity::from_secret(api_secret))
}

async fn build_p2p(
    identity: NodeIdentity,
    seed: &[(String, SocketAddr)],
    overrides: &[(String, SocketAddr)],
) -> Result<Arc<P2PService>, Box<dyn std::error::Error + Send + Sync>> {
    let svc = Arc::new(P2PService::new("0.0.0.0:0".parse()?, identity).await?);
    for (id, addr) in seed.iter().chain(overrides.iter()) {
        svc.registry.upsert(id.clone(), *addr);
    }
    Ok(svc)
}

type FetchFuture = std::pin::Pin<Box<dyn std::future::Future<Output = Option<Vec<u8>>> + Send>>;
type ShardFetcher = dyn Fn(String, [u8; 32]) -> FetchFuture + Send;

/// Fetch closure shared by all restore paths (retrieve; hash-verify is done by caller).
fn make_fetcher(svc: Arc<P2PService>) -> Box<ShardFetcher> {
    Box::new(move |node_id, shard_hash| {
        let svc = svc.clone();
        Box::pin(async move {
            let token = svc.identity().create_shard_token(np2p::crypto::ShardOp::Retrieve, &shard_hash);
            match svc.send_message(&node_id, &Message::RetrieveShardRequest { shard_hash, token }).await {
                Ok(Message::RetrieveShardResponse { data, .. }) => data,
                _ => None,
            }
        })
    })
}

// ── list ──────────────────────────────────────────────────────────────────────

async fn cmd_list(common: &CommonOpts) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let config = load_config(common)?;
    let dir = db_restore::manifest_dir(&config.p2p_data_dir);
    let manifests = db_restore::list_manifests(&dir);

    if manifests.is_empty() {
        println!("No DB snapshots found in {}", dir.display());
        return Ok(());
    }

    println!("Available DB snapshots in {} (newest first):\n", dir.display());
    for m in &manifests {
        println!("  {}  {}  {} bytes  {} shards  segments={}",
            &m.backup_hash[..16.min(m.backup_hash.len())],
            m.created_at,
            m.size_bytes,
            m.shards.len(),
            m.segment_count);
    }
    println!("\nRestore with: disaster_recovery db --config {} --backup-hash <hash>", common.config);
    Ok(())
}

// ── db ────────────────────────────────────────────────────────────────────────

/// Reconstruct + decrypt a DB snapshot. Returns (manifest, plaintext dump bytes).
/// The manifest comes from local disk, or — when absent or `--from-mesh` is set —
/// is retrieved from the P2P mesh itself (true full-disk-loss recovery).
async fn restore_db_common(
    common: &CommonOpts,
    backup_hash: Option<String>,
    from_mesh: bool,
) -> Result<(DbBackupManifest, Vec<u8>), Box<dyn std::error::Error + Send + Sync>> {
    let config = load_config(common)?;
    let api_secret = config.get_api_key()?.to_string();
    let dir = db_restore::manifest_dir(&config.p2p_data_dir);

    // Obtain the manifest: local disk first (unless --from-mesh), else the mesh.
    let local = if from_mesh {
        None
    } else {
        match &backup_hash {
            Some(h) => Some(db_restore::read_manifest(&dir.join(format!("{}.json", h)))?),
            None => db_restore::list_manifests(&dir).into_iter().next(),
        }
    };
    let manifest = match local {
        Some(m) => m,
        None => fetch_manifest_from_mesh(&config, common, backup_hash.as_deref(), &api_secret).await?,
    };

    println!("Restoring DB snapshot {} (created {}, {} bytes, {} shards)",
        &manifest.backup_hash[..16.min(manifest.backup_hash.len())],
        manifest.created_at, manifest.size_bytes, manifest.shards.len());

    // Seed the registry from the manifest's recorded node addresses.
    let seed: Vec<(String, SocketAddr)> = manifest.shards.iter()
        .filter_map(|s| s.addr.parse::<SocketAddr>().ok().map(|a| (s.node_id.clone(), a)))
        .collect();
    let identity = load_identity(&config.p2p_data_dir, &api_secret)?;
    let svc = build_p2p(identity, &seed, &common.nodes).await?;

    println!("Fetching shards and reconstructing…");
    let dump = db_restore::restore_db_snapshot(&manifest, &api_secret, make_fetcher(svc)).await?;
    println!("Reconstructed + decrypted dump: {} bytes", dump.len());
    Ok((manifest, dump))
}

/// Discover storage nodes (LAN broadcast + --node overrides) and retrieve the
/// encrypted snapshot manifest from the mesh. Used when there is no on-disk manifest
/// (full home-server disk loss).
async fn fetch_manifest_from_mesh(
    config: &Config,
    common: &CommonOpts,
    backup_hash: Option<&str>,
    api_secret: &str,
) -> Result<DbBackupManifest, Box<dyn std::error::Error + Send + Sync>> {
    println!("No local manifest — discovering storage nodes and fetching manifest from mesh…");
    let identity = load_identity(&config.p2p_data_dir, api_secret)?;
    let svc = build_p2p(identity, &[], &common.nodes).await?;

    // Listen for LAN broadcast announcements from storage nodes for a few seconds.
    let our_id = hex::encode(svc.identity().node_id());
    np2p::network::discovery::start_listener(svc.registry.clone(), config.p2p_discovery_port, our_id);
    tokio::time::sleep(std::time::Duration::from_secs(4)).await;

    let name = backup_hash.map(db_restore::mesh_manifest_name)
        .unwrap_or_else(|| db_restore::MESH_LATEST_MANIFEST.to_string());
    let name_hash: [u8; 32] = blake3::hash(name.as_bytes()).into();

    let peers = svc.registry.all();
    if peers.is_empty() {
        return Err("no storage nodes discovered on the LAN — use --node node_id=host:port to specify one".into());
    }

    for peer in &peers {
        let token = svc.identity().create_shard_token(np2p::crypto::ShardOp::Retrieve, &name_hash);
        let msg = Message::GetPinnedObject { name: name.clone(), token };
        if let Ok(Message::PinnedObjectResponse { data: Some(data) }) = svc.send_message(&peer.node_id, &msg).await {
            let json = db_restore::decrypt_from_mesh(&data, api_secret)?;
            let manifest: DbBackupManifest = serde_json::from_slice(&json)
                .map_err(|e| format!("failed to parse mesh manifest: {}", e))?;
            println!("Retrieved manifest '{}' from mesh node {}", name, peer.node_id);
            return Ok(manifest);
        }
    }
    Err(format!("manifest '{}' not found on any of the {} discovered node(s)", name, peers.len()).into())
}

async fn cmd_db(
    common: &CommonOpts,
    backup_hash: Option<String>,
    out: &str,
    pg_restore: bool,
    target_db_url: Option<String>,
    clean: bool,
    from_mesh: bool,
) -> Result<DbBackupManifest, Box<dyn std::error::Error + Send + Sync>> {
    let (manifest, dump) = restore_db_common(common, backup_hash, from_mesh).await?;

    let out_path = Path::new(out);
    if let Some(parent) = out_path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(out_path, &dump)?;
    println!("Wrote dump to {}", out_path.display());

    if pg_restore {
        let url = target_db_url.ok_or("--target-db-url is required with --pg-restore")?;
        run_pg_restore(&url, out_path, clean).await?;
    } else {
        println!("\nNext: restore into a database with, e.g.:");
        println!("  pg_restore -d <target_db_url> {}", out_path.display());
        println!("  (or re-run with --pg-restore --target-db-url <url> [--clean])");
    }
    Ok(manifest)
}

async fn run_pg_restore(target_db_url: &str, dump_path: &Path, clean: bool) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    println!("Running pg_restore into {}…", target_db_url);
    let mut cmd = tokio::process::Command::new("pg_restore");
    if clean {
        cmd.arg("--clean").arg("--if-exists");
    }
    cmd.arg("--dbname").arg(target_db_url).arg(dump_path);

    let output = cmd.output().await.map_err(|e| {
        if e.kind() == std::io::ErrorKind::NotFound {
            "pg_restore not found on PATH".to_string()
        } else {
            format!("failed to run pg_restore: {}", e)
        }
    })?;

    // pg_restore returns non-zero if any warning/error occurred; surface stderr but
    // only hard-fail if the dump wasn't processed at all.
    let stderr = String::from_utf8_lossy(&output.stderr);
    if !output.status.success() {
        eprintln!("pg_restore exited with {}:\n{}", output.status, stderr);
        return Err(format!("pg_restore failed (status {})", output.status).into());
    }
    if !stderr.is_empty() {
        eprintln!("pg_restore warnings:\n{}", stderr);
    }
    println!("pg_restore completed");
    Ok(())
}

// ── media ─────────────────────────────────────────────────────────────────────

async fn cmd_media(
    common: &CommonOpts,
    hash: Option<String>,
    all_missing: bool,
    all: bool,
    db_url: Option<String>,
    limit: Option<usize>,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let config = load_config(common)?;
    let api_secret = config.get_api_key()?.to_string();
    let db_url = db_url.or_else(|| config.database_url.clone())
        .ok_or("database URL required (config.database_url or --db-url)")?;

    let pool = reminisce::db::create_pool(&db_url).map_err(|e| e.to_string())?;

    // Seed the registry from the p2p_nodes table (node_id → public_addr).
    let mut seed: Vec<(String, SocketAddr)> = Vec::new();
    {
        let client = pool.get().await?;
        let rows = client.query("SELECT node_id, public_addr FROM p2p_nodes WHERE is_active = TRUE", &[]).await?;
        for row in &rows {
            let node_id: String = row.get(0);
            let addr: Option<String> = row.get(1);
            if let Some(a) = addr {
                if let Ok(parsed) = a.parse::<SocketAddr>() {
                    seed.push((node_id, parsed));
                }
            }
        }
    }

    let identity = load_identity(&config.p2p_data_dir, &api_secret)?;
    let svc = build_p2p(identity, &seed, &common.nodes).await?;

    // Determine which files to restore.
    let targets: Vec<(String, String)> = if let Some(h) = hash {
        vec![(h, String::new())] // ext resolved from DB by restore_file
    } else if all || all_missing {
        collect_media_candidates(&pool, &config, !all).await?
    } else {
        return Err("specify --hash <hash>, --all-missing, or --all".into());
    };

    let cap = limit.unwrap_or(0);
    let mut restored = 0usize;
    let mut failed = 0usize;

    for (file_hash, _ext) in targets.iter() {
        if cap > 0 && restored >= cap { break; }
        match restore_file(&pool, &svc, file_hash, &api_secret, None).await {
            Ok(restored_file) => {
                match write_media_to_disk(&config, file_hash, &restored_file) {
                    Ok(path) => {
                        println!("Restored {} → {}", &file_hash[..16.min(file_hash.len())], path.display());
                        restored += 1;
                    }
                    Err(e) => { eprintln!("Write failed for {}: {}", file_hash, e); failed += 1; }
                }
            }
            Err(e) => { eprintln!("Restore failed for {}: {}", &file_hash[..16.min(file_hash.len())], e); failed += 1; }
        }
    }

    println!("\nMedia restore complete: {} restored, {} failed", restored, failed);
    Ok(())
}

/// Collect (hash, ext) for all non-deleted media. If `only_missing`, filter to files
/// whose on-disk copy is gone.
async fn collect_media_candidates(
    pool: &Pool,
    config: &Config,
    only_missing: bool,
) -> Result<Vec<(String, String)>, Box<dyn std::error::Error + Send + Sync>> {
    let client = pool.get().await?;
    let mut out = Vec::new();

    for (table, base_dir) in [("images", config.get_images_dir()), ("videos", config.get_videos_dir())] {
        let query = format!("SELECT hash, ext FROM {} WHERE deleted_at IS NULL AND p2p_synced_at IS NOT NULL", table);
        let rows = client.query(&query, &[]).await?;
        for row in &rows {
            let hash: String = row.get(0);
            let ext: String = row.get(1);
            if only_missing && media_path(base_dir, &hash, &ext).exists() {
                continue;
            }
            out.push((hash, ext));
        }
    }
    Ok(out)
}

/// The on-disk storage path for a media file: `<base>/<hash[0..2]>/<hash>.<ext>`.
fn media_path(base_dir: &str, hash: &str, ext: &str) -> PathBuf {
    PathBuf::from(base_dir).join(&hash[0..2]).join(format!("{}.{}", hash, ext))
}

fn write_media_to_disk(
    config: &Config,
    hash: &str,
    restored: &reminisce::p2p_restore::RestoredFile,
) -> Result<PathBuf, Box<dyn std::error::Error + Send + Sync>> {
    let base_dir = if restored.media_type == "video" { config.get_videos_dir() } else { config.get_images_dir() };
    let ext = restored.filename.rsplit('.').next().unwrap_or("bin");
    let path = media_path(base_dir, hash, ext);
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(&path, &restored.data)?;
    Ok(path)
}

// ── full ──────────────────────────────────────────────────────────────────────

#[allow(clippy::too_many_arguments)]
async fn cmd_full(
    common: &CommonOpts,
    backup_hash: Option<String>,
    out: &str,
    pg_restore: bool,
    target_db_url: Option<String>,
    clean: bool,
    from_mesh: bool,
    restore_media: bool,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    println!("=== Step 1/2: restore database snapshot ===");
    cmd_db(common, backup_hash, out, pg_restore, target_db_url, clean, from_mesh).await?;

    if restore_media {
        println!("\n=== Step 2/2: restore missing media files ===");
        cmd_media(common, None, true, false, None, None).await?;
    }

    println!("\nFull disaster recovery complete.");
    Ok(())
}
