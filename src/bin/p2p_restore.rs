use np2p::crypto::NodeIdentity;
use np2p::network::P2PService;
use reminisce::p2p_restore::restore_file;
use reminisce::db::create_pool;
use std::sync::Arc;
use clap::Parser;

#[derive(Parser)]
#[command(name = "p2p_restore", about = "Restore a file from Reminisce P2P backup")]
struct Args {
    #[arg(long, help = "PostgreSQL connection URL (e.g. postgres://user:pass@host/db)")]
    db_url: String,

    #[arg(long, help = "Storage node address (e.g. 192.168.1.155:5050)")]
    pi_addr: String,

    #[arg(long, help = "File hash to restore (hex string)")]
    hash: String,

    #[arg(long, default_value = ".", help = "Output directory for restored file")]
    output: String,
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    tracing_subscriber::fmt::init();

    let args = Args::parse();

    let pool = create_pool(&args.db_url).map_err(|e| e.to_string())?;

    let identity = NodeIdentity::generate();
    let p2p_service = Arc::new(
        P2PService::new("0.0.0.0:0".parse()?, identity).await?
    );

    // Pre-seed the registry: map all node_ids for this file to the given pi_addr.
    // send_message uses the registry for direct connections.
    let pi_addr: std::net::SocketAddr = args.pi_addr.parse()?;
    {
        let client = pool.get().await?;
        let rows = client.query(
            "SELECT DISTINCT node_id FROM p2p_shards WHERE file_hash = $1",
            &[&args.hash],
        ).await?;
        for row in &rows {
            let node_id: String = row.get(0);
            p2p_service.registry.upsert(node_id, pi_addr);
        }
    }

    let restored = restore_file(&pool, &p2p_service, &args.hash).await?;

    let out_dir = std::path::Path::new(&args.output);
    std::fs::create_dir_all(out_dir)?;
    let out_path = out_dir.join(&restored.filename);
    std::fs::write(&out_path, &restored.data)?;

    println!("Restored {} → {}", args.hash, out_path.display());
    println!("  media type : {}", restored.media_type);
    println!("  size       : {} bytes", restored.data.len());

    Ok(())
}
