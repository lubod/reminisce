use std::net::SocketAddr;
use std::path::Path;
use std::sync::Arc;
use clap::Parser;
use np2p::network::P2PService;
use reminisce::config::Config;
use reminisce::db::create_pool;
use reminisce::p2p_restore::{load_cli_identity, parse_node_override, restore_file_to_path};

#[derive(Parser)]
#[command(name = "p2p_restore", about = "Restore a file from Reminisce P2P backup")]
struct Args {
    #[arg(long, help = "Path to config.yaml (provides database_url, api_secret_key, p2p_data_dir, p2p_identity_kdf)")]
    config: Option<String>,

    #[arg(long, help = "PostgreSQL connection URL (overrides config database_url)")]
    db_url: Option<String>,

    #[arg(long, help = "Fallback storage node address (e.g. 192.168.1.155:5050) applied to shard node_ids missing a registered address")]
    pi_addr: Option<String>,

    #[arg(long = "node", help = "Storage node override 'node_id=host:port' (repeatable)", value_parser = parse_node_override)]
    nodes: Vec<(String, SocketAddr)>,

    #[arg(long, help = "File hash to restore (hex string)")]
    hash: String,

    #[arg(long, help = "Master API secret key (api_secret_key) used to unwrap the per-file encryption key (overrides config)")]
    api_secret: Option<String>,

    #[arg(long, help = "Directory containing node.key (overrides config p2p_data_dir)")]
    p2p_data_dir: Option<String>,

    #[arg(long, help = "Identity KDF override ('argon2id' or 'legacy'; defaults to config p2p_identity_kdf)")]
    identity_kdf: Option<String>,

    #[arg(long, default_value = ".", help = "Output directory for restored file")]
    output: String,
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    tracing_subscriber::fmt::init();

    let args = Args::parse();
    let cfg = args.config.as_deref()
        .map(Config::from_file)
        .transpose()
        .map_err(|e| format!("failed to load config: {}", e))?;

    let db_url = args.db_url
        .or_else(|| cfg.as_ref().and_then(|c| c.database_url.clone()))
        .ok_or("missing PostgreSQL connection URL: pass --db-url or --config")?;
    let api_secret = args.api_secret
        .or_else(|| cfg.as_ref().and_then(|c| c.api_secret_key.clone()))
        .unwrap_or_default();
    let p2p_data_dir = args.p2p_data_dir
        .or_else(|| cfg.as_ref().map(|c| c.p2p_data_dir.clone()))
        .unwrap_or_else(|| "p2p_data".to_string());
    let identity_kdf = args.identity_kdf
        .or_else(|| cfg.as_ref().and_then(|c| c.p2p_identity_kdf.clone()));

    let pool = create_pool(&db_url).map_err(|e| e.to_string())?;
    let identity = load_cli_identity(&p2p_data_dir, &api_secret, identity_kdf.as_deref())?;
    let p2p_service = Arc::new(
        P2PService::new("0.0.0.0:0".parse()?, identity).await?
    );

    // Seed node addresses from p2p_nodes (with optional --pi-addr fallback and --node overrides).
    let pi_fallback: Option<SocketAddr> = args.pi_addr.as_deref()
        .map(|s| s.parse::<SocketAddr>())
        .transpose()?;
    {
        let client = pool.get().await?;
        let rows = client.query(
            "SELECT DISTINCT s.node_id, n.public_addr
             FROM p2p_shards s
             LEFT JOIN p2p_nodes n ON n.node_id = s.node_id
             WHERE s.file_hash = $1",
            &[&args.hash],
        ).await?;
        for row in &rows {
            let node_id: String = row.get(0);
            let public_addr: Option<String> = row.get(1);
            if let Some(addr) = public_addr.and_then(|a| a.parse::<SocketAddr>().ok()).or(pi_fallback) {
                p2p_service.registry.upsert(node_id, addr);
            }
        }
    }
    for (node_id, addr) in &args.nodes {
        p2p_service.registry.upsert(node_id.clone(), *addr);
    }

    let out_dir = Path::new(&args.output);
    std::fs::create_dir_all(out_dir)?;
    let prefix: String = args.hash.chars().take(16).filter(|c| c.is_ascii_alphanumeric()).collect();
    let stage_path = out_dir.join(format!(".{}.{}.restore_tmp", if prefix.is_empty() { "file" } else { &prefix }, std::process::id()));

    let meta = match restore_file_to_path(&pool, &p2p_service, &args.hash, &api_secret, None, &stage_path).await {
        Ok(m) => m,
        Err(e) => {
            let _ = std::fs::remove_file(&stage_path);
            return Err(e);
        }
    };

    let safe_name = Path::new(&meta.filename)
        .file_name()
        .and_then(|s| s.to_str())
        .filter(|s| !s.is_empty() && *s != "." && *s != "..")
        .map(str::to_string)
        .unwrap_or_else(|| format!("{}.bin", if prefix.is_empty() { "restored" } else { &prefix }));
    let out_path = out_dir.join(&safe_name);
    if let Err(e) = std::fs::rename(&stage_path, &out_path) {
        let _ = std::fs::remove_file(&stage_path);
        return Err(e.into());
    }

    println!("Restored {} → {}", args.hash, out_path.display());
    println!("  media type : {}", meta.media_type);
    println!("  size       : {} bytes", meta.size_bytes);

    Ok(())
}
