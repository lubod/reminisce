//! Periodic database backup to P2P storage nodes.
//!
//! On a fixed interval (default 24h) the worker runs `pg_dump -Fc`, treats the
//! dump as a high-priority system object — encrypts it (ChaCha20-Poly1305),
//! erasure-codes it into 3/5 Reed-Solomon shards, and pushes the shards to the
//! active P2P storage nodes via QUIC. A rolling manifest (`db_backups`) keeps
//! the last N snapshot hashes; older snapshots are pruned — their shards are
//! deleted from the remote nodes and their manifest rows removed.
//!
//! Key management: a fresh random ChaCha20 key encrypts each dump. The key is
//! stored encrypted-with-master-key both in `db_backups` and in the on-disk
//! `p2p_keys.escrow` file (outside the database) so it survives a DB loss.

use crate::config::Config;
use crate::metrics::{
    DB_BACKUP_DURATION_SECONDS, DB_BACKUP_FAILURES_TOTAL, DB_BACKUP_PRUNED_TOTAL,
    DB_BACKUP_SIZE_BYTES, DB_BACKUP_SNAPSHOTS_KEPT, DB_BACKUP_SUCCESS_TOTAL,
};
use crate::p2p_upload::{self, SEGMENT_THRESHOLD, SHARD_COUNT};
use deadpool_postgres::Pool;
use log::{error, info, warn};
use np2p::network::{Message, P2PService};
use np2p::storage::StorageEngine;
use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;
use tokio::io::AsyncReadExt;

/// Entry point. Fixed-interval loop (not adaptive backoff): run a backup, then
/// sleep for the configured interval, respecting the shutdown token.
pub async fn db_backup_loop(
    pool: Pool,
    config: Config,
    p2p_service: Arc<P2PService>,
    shutdown_token: tokio_util::sync::CancellationToken,
) {
    info!("DB Backup Worker started (interval={}s, retention={})",
        config.workers.db_backup_interval_secs, config.workers.db_backup_retention_count);

    // Give LAN/coordinator discovery time to register storage nodes before first run.
    tokio::time::sleep(Duration::from_secs(30)).await;

    let interval = Duration::from_secs(config.workers.db_backup_interval_secs.max(60));
    loop {
        if let Err(e) = backup_cycle(&pool, &config, &p2p_service).await {
            error!("DB backup cycle failed: {}", e);
            DB_BACKUP_FAILURES_TOTAL.inc();
        }

        tokio::select! {
            _ = shutdown_token.cancelled() => {
                info!("DB Backup Worker shutting down");
                break;
            }
            _ = tokio::time::sleep(interval) => {}
        }
    }
}

/// One full backup + retention cycle.
async fn backup_cycle(
    pool: &Pool,
    config: &Config,
    p2p_service: &Arc<P2PService>,
) -> Result<bool, String> {
    if !config.workers.db_backup_enabled {
        return Ok(false);
    }
    if config.database_url.is_none() {
        return Ok(false);
    }

    let nodes: Vec<(String, SocketAddr)> = p2p_service
        .registry
        .all()
        .into_iter()
        .map(|p| (p.node_id, p.addr))
        .collect();
    if nodes.is_empty() {
        return Ok(false);
    }
    if nodes.len() < 3 {
        warn!("Only {} P2P nodes discovered. 3+ nodes recommended for 3/5 EC redundancy.", nodes.len());
    }

    let start = std::time::Instant::now();

    // 1. Dump the database (custom format) to a temp file.
    let dump_path = dump_database(config).await?;
    // Ensure the temp file is removed even on failure.
    let result = backup_dump_file(pool, config, p2p_service, &nodes, &dump_path).await;
    let _ = tokio::fs::remove_file(&dump_path).await;
    // Remove the enclosing 0700 dir dump_database created (best-effort).
    if let Some(parent) = dump_path.parent() {
        let _ = tokio::fs::remove_dir(parent).await;
    }
    let did_work = result?;

    if did_work {
        DB_BACKUP_SUCCESS_TOTAL.inc();
        DB_BACKUP_DURATION_SECONDS.observe(start.elapsed().as_secs_f64());
    }

    // 2. Retention: prune old snapshots (remote shards + manifest rows + manifest files).
    let retention = config.workers.db_backup_retention_count.max(1);
    match prune_old_snapshots(pool, config, p2p_service, retention).await {
        Ok(pruned) if pruned > 0 => {
            info!("DB backup retention: pruned {} old snapshot(s)", pruned);
            DB_BACKUP_PRUNED_TOTAL.inc_by(pruned);
        }
        Ok(_) => {}
        Err(e) => warn!("DB backup retention pruning failed: {}", e),
    }

    // 3. Update kept-snapshots gauge.
    if let Ok(client) = pool.get().await {
        if let Ok(row) = client.query_one("SELECT COUNT(*) FROM db_backups", &[]).await {
            let count: i64 = row.get(0);
            DB_BACKUP_SNAPSHOTS_KEPT.set(count);
        }
    }

    Ok(did_work)
}

/// Dump the plaintext dump to shards, unless an identical snapshot already exists.
/// Returns Ok(true) if a new snapshot was stored.
async fn backup_dump_file(
    pool: &Pool,
    config: &Config,
    p2p_service: &Arc<P2PService>,
    nodes: &[(String, SocketAddr)],
    dump_path: &Path,
) -> Result<bool, String> {
    let metadata = tokio::fs::metadata(dump_path).await.map_err(|e| e.to_string())?;
    let file_size = metadata.len() as usize;
    if file_size == 0 {
        return Err("pg_dump produced an empty file".to_string());
    }

    // 2. Content hash of the plaintext dump (streamed, memory-efficient).
    let backup_hash = blake3_file_hex(dump_path).await?;

    // 3. Dedup: skip if an identical snapshot is already stored.
    {
        let client = pool.get().await.map_err(|e| e.to_string())?;
        let exists = client
            .query_opt("SELECT 1 FROM db_backups WHERE backup_hash = $1", &[&backup_hash])
            .await
            .map_err(|e| e.to_string())?
            .is_some();
        if exists {
            info!("DB unchanged since last snapshot (hash {}…) — skipping", &backup_hash[..16]);
            return Ok(false);
        }
    }

    DB_BACKUP_SIZE_BYTES.observe(file_size as f64);

    // 4. Encrypt + shard + upload.
    let encryption_key: [u8; 32] = rand::random();
    let data_shards = config.p2p_data_shards;
    let parity_shards = config.p2p_parity_shards;

    let (uploaded, enc_size, segment_count, segment_enc_sizes) = if file_size > SEGMENT_THRESHOLD {
        let (u, sizes) = p2p_upload::upload_segmented(p2p_service, nodes, &backup_hash, dump_path, &encryption_key, data_shards, parity_shards).await?;
        (u, 0i64, sizes.len() as i32, Some(sizes))
    } else {
        let data = tokio::fs::read(dump_path).await.map_err(|e| e.to_string())?;
        let (shards, enc_size) = StorageEngine::process_for_backup(&data, &encryption_key, &encryption_key, data_shards, parity_shards)
            .map_err(|e| e.to_string())?;
        let u = p2p_upload::upload_inmemory(p2p_service, nodes, &backup_hash, shards).await?;
        (u, enc_size as i64, 1i32, None)
    };

    if uploaded.len() < data_shards {
        return Err(format!(
            "Only {}/{} shards stored for DB snapshot. Minimum {} required for reconstruction.",
            uploaded.len(), SHARD_COUNT, data_shards
        ));
    }

    // 5. Persist manifest + shard placement (transaction).
    let api_secret = config.get_api_key().map_err(|e| e.to_string())?;
    let encrypted_key = crate::utils::encrypt_key(&encryption_key, api_secret)?;

    let mut client = pool.get().await.map_err(|e| e.to_string())?;
    let trans = client.transaction().await.map_err(|e| e.to_string())?;
    trans.execute(
        "INSERT INTO db_backups (backup_hash, size_bytes, encrypted_size, data_shards, parity_shards, encryption_key, segment_count, segment_enc_sizes)
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8)
         ON CONFLICT (backup_hash) DO NOTHING",
        &[
            &backup_hash,
            &(file_size as i64),
            &enc_size,
            &(data_shards as i32),
            &(parity_shards as i32),
            &encrypted_key,
            &segment_count,
            &segment_enc_sizes,
        ],
    ).await.map_err(|e| e.to_string())?;

    for shard in &uploaded {
        trans.execute(
            "INSERT INTO p2p_nodes (node_id, public_addr, is_active) VALUES ($1, $2, TRUE)
             ON CONFLICT (node_id) DO UPDATE SET public_addr = $2, is_active = TRUE, last_seen = NOW()",
            &[&shard.node_id, &shard.addr],
        ).await.map_err(|e| e.to_string())?;
        trans.execute(
            "INSERT INTO db_backup_shards (backup_hash, shard_index, node_id, shard_hash)
             VALUES ($1, $2, $3, $4)
             ON CONFLICT (backup_hash, shard_index) DO UPDATE SET node_id = $3, shard_hash = $4",
            &[&backup_hash, &(shard.idx as i32), &shard.node_id, &shard.shard_hash_hex],
        ).await.map_err(|e| e.to_string())?;
    }
    trans.commit().await.map_err(|e| e.to_string())?;

    // 6. Append the (encrypted) key to the on-disk escrow file so it survives DB loss.
    append_key_to_escrow(config, &backup_hash, &encrypted_key);

    // 7. Write a self-contained restore manifest to disk (outside the DB) so the
    // snapshot can be restored even if the database itself is lost.
    let manifest = crate::db_restore::DbBackupManifest {
        backup_hash: backup_hash.clone(),
        created_at: chrono::Utc::now().to_rfc3339(),
        size_bytes: file_size as i64,
        encrypted_size: enc_size,
        data_shards: data_shards as i32,
        parity_shards: parity_shards as i32,
        segment_count,
        segment_enc_sizes: segment_enc_sizes.clone(),
        encryption_key_hex: hex::encode(&encrypted_key),
        shards: uploaded.iter().map(|s| crate::db_restore::ManifestShard {
            index: s.idx,
            node_id: s.node_id.clone(),
            addr: s.addr.clone(),
            shard_hash: s.shard_hash_hex.clone(),
        }).collect(),
    };
    let manifest_dir = crate::db_restore::manifest_dir(&config.p2p_data_dir);
    if let Err(e) = crate::db_restore::write_manifest(&manifest_dir, &manifest) {
        warn!("Failed to write DB backup manifest to disk: {}", e);
    }

    // 8. Publish the manifest (encrypted with the api_secret-derived mesh key) to all
    // reachable nodes as a pinned object, so the restore map survives a full
    // home-server disk loss. Best-effort — local manifest + escrow already exist.
    if let Ok(api_secret) = config.get_api_key() {
        publish_manifest_to_mesh(p2p_service, nodes, &manifest, api_secret).await;
    }

    info!("DB snapshot {} stored: {} bytes, {} shards across {} node(s)",
        &backup_hash[..16], file_size, uploaded.len(), nodes.len());
    Ok(true)
}

/// Run `pg_dump -Fc` for the configured database into a temp file (async).
async fn dump_database(config: &Config) -> Result<PathBuf, String> {
    let database_url = config.database_url.as_ref().ok_or("Database URL not configured")?;
    // Plaintext full-DB dump must not be world-readable while it exists: create
    // it under a 0600 file in a private temp dir instead of /tmp root.
    let private_dir = std::env::temp_dir().join(format!("reminisce_db_backup_{}", std::process::id()));
    tokio::fs::create_dir_all(&private_dir)
        .await
        .map_err(|e| format!("Failed to create private backup dir: {}", e))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = tokio::fs::set_permissions(&private_dir, std::fs::Permissions::from_mode(0o700)).await;
    }
    let output_path = private_dir.join(format!("reminisce_db_backup_{}.dump", chrono::Utc::now().timestamp()));

    let output = tokio::process::Command::new("pg_dump")
        .arg("--format=custom")
        .arg("--file")
        .arg(&output_path)
        .arg(database_url)
        .output()
        .await
        .map_err(|e| {
            if e.kind() == std::io::ErrorKind::NotFound {
                "pg_dump not found on PATH".to_string()
            } else {
                format!("Failed to run pg_dump: {}", e)
            }
        })?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        let _ = tokio::fs::remove_file(&output_path).await;
        return Err(format!("pg_dump failed: {}", stderr));
    }
    Ok(output_path)
}

/// Stream a file through BLAKE3 and return the hex digest (memory-efficient).
async fn blake3_file_hex(path: &Path) -> Result<String, String> {
    let mut file = tokio::fs::File::open(path).await.map_err(|e| e.to_string())?;
    let mut hasher = blake3::Hasher::new();
    let mut buf = vec![0u8; 1024 * 1024];
    loop {
        let n = file.read(&mut buf).await.map_err(|e| e.to_string())?;
        if n == 0 { break; }
        hasher.update(&buf[..n]);
    }
    Ok(hasher.finalize().to_hex().to_string())
}

/// Publish the snapshot manifest to every reachable node as an encrypted pinned
/// object (both the `latest` pointer and a per-snapshot name), so the restore map
/// survives a full home-server disk loss. Best-effort per node.
async fn publish_manifest_to_mesh(
    p2p_service: &Arc<P2PService>,
    nodes: &[(String, SocketAddr)],
    manifest: &crate::db_restore::DbBackupManifest,
    api_secret: &str,
) {
    let json = match serde_json::to_vec(manifest) {
        Ok(j) => j,
        Err(e) => { warn!("Mesh manifest serialize failed: {}", e); return; }
    };
    let encrypted = match crate::db_restore::encrypt_for_mesh(&json, api_secret, manifest.backup_hash.as_bytes()) {
        Ok(d) => d,
        Err(e) => { warn!("Mesh manifest encrypt failed: {}", e); return; }
    };

    let names = [
        crate::db_restore::MESH_LATEST_MANIFEST.to_string(),
        crate::db_restore::mesh_manifest_name(&manifest.backup_hash),
    ];

    let mut stored = 0usize;
    for (node_id, addr) in nodes {
        for name in &names {
            let name_hash: [u8; 32] = blake3::hash(name.as_bytes()).into();
            let token = p2p_service.identity().create_shard_token(np2p::crypto::ShardOp::Store, &name_hash);
            let msg = Message::StorePinnedObject { name: name.clone(), data: encrypted.clone(), token };
            match p2p_service.send_message(node_id, &msg).await {
                Ok(Message::StorePinnedResponse { success: true }) => { stored += 1; }
                Ok(other) => warn!("Mesh manifest store to {} for '{}' unexpected: {:?}", node_id, name, other),
                Err(e) => warn!("Mesh manifest store to {} for '{}' failed: {}", node_id, name, e),
            }
        }
        let _ = addr;
    }
    info!("Published DB manifest to mesh ({} copies across {} node(s))", stored, nodes.len());
}

/// Append the encrypted backup key to the on-disk escrow file (outside the DB).
fn append_key_to_escrow(config: &Config, backup_hash: &str, encrypted_key: &[u8]) {    let base_dir = config.get_images_dir();
    let abs_base = std::fs::canonicalize(base_dir).unwrap_or_else(|_| PathBuf::from(base_dir));
    let escrow_path = abs_base.parent().unwrap_or(&abs_base).join("p2p_keys.escrow");

    let mut options = std::fs::OpenOptions::new();
    options.create(true).append(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }

    if let Ok(mut file) = options.open(&escrow_path) {
        use std::io::Write;
        let line = format!("dbbackup:{},{}\n", backup_hash, hex::encode(encrypted_key));
        let _ = file.write_all(line.as_bytes());
    } else {
        warn!("Could not open p2p_keys.escrow to record DB backup key");
    }
}

/// Delete snapshots older than the newest `retention`. Removes shards from the
/// remote nodes, then the manifest rows (cascading to db_backup_shards) and the
/// on-disk manifest files.
async fn prune_old_snapshots(
    pool: &Pool,
    config: &Config,
    p2p_service: &Arc<P2PService>,
    retention: i64,
) -> Result<u64, String> {
    let client = pool.get().await.map_err(|e| e.to_string())?;
    let old = client
        .query("SELECT backup_hash FROM db_backups ORDER BY created_at DESC OFFSET $1", &[&retention])
        .await
        .map_err(|e| e.to_string())?;

    let mut pruned = 0u64;
    for row in old {
        let backup_hash: String = row.get(0);

        // Delete each shard from its remote node (best-effort).
        let shard_rows = client
            .query("SELECT shard_hash, node_id FROM db_backup_shards WHERE backup_hash = $1", &[&backup_hash])
            .await
            .map_err(|e| e.to_string())?;
        for srow in shard_rows {
            let shard_hash: String = srow.get(0);
            let node_id: String = srow.get(1);
            delete_shard_remote(pool, p2p_service, &node_id, &shard_hash).await;
        }

        // Remove the manifest row (cascades to db_backup_shards) and the on-disk manifest file.
        client
            .execute("DELETE FROM db_backups WHERE backup_hash = $1", &[&backup_hash])
            .await
            .map_err(|e| e.to_string())?;
        crate::db_restore::delete_manifest(&crate::db_restore::manifest_dir(&config.p2p_data_dir), &backup_hash);
        pruned += 1;
    }
    Ok(pruned)
}

/// Send a DeleteShardRequest to the node holding a shard (best-effort).
async fn delete_shard_remote(
    pool: &Pool,
    p2p_service: &Arc<P2PService>,
    node_id: &str,
    shard_hash_hex: &str,
) {
    let addr = match crate::shard_rebalance_worker::lookup_node_addr(pool, p2p_service, node_id).await {
        Some(a) => a,
        None => {
            warn!("Retention: cannot resolve addr for node {} — shard {} left on node", node_id, &shard_hash_hex[..16.min(shard_hash_hex.len())]);
            return;
        }
    };

    let result = crate::p2p_upload::delete_shard_remote(p2p_service, addr, node_id, shard_hash_hex).await;
    match result {
        Ok(true) => info!("Retention: deleted shard {} from node {}", &shard_hash_hex[..16.min(shard_hash_hex.len())], node_id),
        Ok(false) => warn!("Retention: node {} refused to delete shard {}", node_id, &shard_hash_hex[..16.min(shard_hash_hex.len())]),
        Err(e) => warn!("Retention: failed to delete shard {} from node {}: {}", &shard_hash_hex[..16.min(shard_hash_hex.len())], node_id, e),
    }
}


#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::Config;
    use std::io::Write;

    /// Build a Config from a minimal YAML (serde defaults fill the rest).
    fn mini_config(images_dir: Option<String>) -> Config {
        let mut c: Config = serde_yaml::from_str("api_secret_key: \"test-secret-for-unit-tests-0123456789abcdef01234567\"\n")
            .expect("minimal config");
        c.images_dir = images_dir;
        c
    }

    #[actix_web::test]
    async fn blake3_file_hex_hashes_temp_file() {
        let dir = std::env::temp_dir().join(format!("reminisce_bk_{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let p = dir.join("data.bin");
        let mut f = std::fs::File::create(&p).unwrap();
        f.write_all(b"hello backup coverage").unwrap();
        drop(f);

        let hex = blake3_file_hex(&p).await.expect("hash ok");
        assert_eq!(hex.len(), 64);
        assert_eq!(hex, blake3::hash(b"hello backup coverage").to_hex().to_string());

        let missing = dir.join("nope.bin");
        assert!(blake3_file_hex(&missing).await.is_err(), "missing file errors");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[actix_web::test]
    async fn dump_database_requires_configured_url() {
        let cfg = mini_config(None);
        let res = dump_database(&cfg).await;
        assert!(res.is_err(), "empty database_url -> Err");
    }

    #[actix_web::test]
    async fn append_key_to_escrow_writes_line_with_0600() {        let base = std::env::temp_dir()
            .join(format!("reminisce_escrow_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&base);
        let media = base.join("media");
        std::fs::create_dir_all(&media).unwrap();

        let cfg = mini_config(Some(media.to_string_lossy().to_string()));
        append_key_to_escrow(&cfg, "hash001", &[0x01, 0x02, 0x03]);

        // escrow lives at <base>/p2p_keys.escrow (parent of the images dir).
        let escrow = base.join("p2p_keys.escrow");
        let content = std::fs::read_to_string(&escrow).expect("escrow written");
        assert!(content.contains("dbbackup:hash001,010203"), "escrow line: {}", content);
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(&escrow).unwrap().permissions().mode() & 0o777;
            assert_eq!(mode, 0o600, "escrow file is 0600");
        }
        let _ = std::fs::remove_dir_all(&base);
    }

    /// Detect whether the dev Postgres connection env is configured, so DB-backed
    /// unit tests skip cleanly when the gate runs without a live dev DB.
    fn test_db_env_available() -> bool {
        std::env::var("TEST_DATABASE_URL").is_ok()
            || std::env::var("PGHOST").is_ok()
            || std::env::var("PGPORT").is_ok()
    }

    #[actix_web::test]
    async fn backup_cycle_short_circuits_when_disabled_or_no_url() {
        if !test_db_env_available() {
            eprintln!("backup_cycle: skipping (no dev PG env)");
            return;
        }
        // Disabled worker: no DB, no peers needed — returns false immediately.
        let (pool, _db) = crate::test_utils::setup_test_database_with_instance().await;
        let mut cfg = mini_config(None);
        cfg.workers.db_backup_enabled = false;
        let p2p = Arc::new(
            np2p::network::P2PService::new("127.0.0.1:0".parse().unwrap(), np2p::crypto::NodeIdentity::generate())
                .await.unwrap()
        );
        assert!(
            !backup_cycle(&pool, &cfg, &p2p).await.unwrap(),
            "disabled backup worker should short-circuit to Ok(false)"
        );

        // Enabled but no configured DB URL: also short-circuits without error.
        let mut cfg = mini_config(None);
        cfg.workers.db_backup_enabled = true;
        cfg.database_url = None;
        assert!(
            !backup_cycle(&pool, &cfg, &p2p).await.unwrap(),
            "no database_url should short-circuit to Ok(false)"
        );

        // Enabled + URL but zero discovered peers: short-circuits to Ok(false).
        let mut cfg = mini_config(None);
        cfg.workers.db_backup_enabled = true;
        cfg.database_url = Some("postgres://postgres:postgres@localhost:25432/reminisce_db".to_string());
        assert!(
            !backup_cycle(&pool, &cfg, &p2p).await.unwrap(),
            "no peers should short-circuit to Ok(false)"
        );
    }

    #[actix_web::test]
    #[serial_test::serial]
    async fn prune_old_snapshots_respects_retention() {
        if !test_db_env_available() {
            eprintln!("prune_old_snapshots: skipping (no dev PG env)");
            return;
        }

        let (pool, _db) = crate::test_utils::setup_test_database_with_instance().await;
        let client = pool.get().await.unwrap();

        // Minimal config so manifest_dir points into a temp dir.
        let base = std::env::temp_dir().join(format!("reminisce_prune_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&base);
        let media = base.join("media");
        std::fs::create_dir_all(&media).unwrap();
        let cfg = mini_config(Some(media.to_string_lossy().to_string()));

        // Insert 3 snapshots with distinct created_at (override via direct SQL).
        let key = [0x42u8; 32];
        for (i, h) in ["prune_snap_a", "prune_snap_b", "prune_snap_c"].iter().enumerate() {
            client.execute(
                "INSERT INTO db_backups (backup_hash, created_at, size_bytes, encrypted_size, encryption_key, segment_count)
                 VALUES ($1, NOW() - ($2 || ' hours')::interval, 1, 1, $3, 1)",
                &[&h, &i.to_string(), &key.as_slice()],
            ).await.unwrap();
        }

        // No live nodes, so delete_shard_remote is best-effort no-op; only the
        // manifest row + file removal path runs.
        let p2p = Arc::new(
            np2p::network::P2PService::new("127.0.0.1:0".parse().unwrap(), np2p::crypto::NodeIdentity::generate())
                .await.unwrap()
        );

        let pruned = prune_old_snapshots(&pool, &cfg, &p2p, 1).await.expect("prune ok");
        assert_eq!(pruned, 2, "keeping 1 newest should prune 2 oldest");

        let remaining: i64 = client.query_one("SELECT COUNT(*) FROM db_backups", &[]).await.unwrap().get(0);
        assert_eq!(remaining, 1, "only the newest snapshot remains");

        let _ = std::fs::remove_dir_all(&base);
    }
}

