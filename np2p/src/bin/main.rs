use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::Arc;
use tracing::{info, warn};
use tracing_subscriber::EnvFilter;
use clap::Parser;
use np2p::crypto::NodeIdentity;
use np2p::network::{P2PService, ConnectionHandler};
use np2p::storage::DiskStorage;

#[derive(Parser, Debug)]
#[command(author, version, about = "np2p storage node daemon", long_about = None)]
struct Args {
    /// Address to listen on for P2P connections
    #[arg(short, long, default_value = "0.0.0.0:5050")]
    listen: SocketAddr,

    /// Directory to store shards and identity
    #[arg(short, long, default_value = "/data")]
    data_dir: PathBuf,

    /// Secret key in hex (optional, generates new if missing)
    #[arg(short, long)]
    secret_key: Option<String>,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    // Required for rustls 0.23+
    let _ = rustls::crypto::ring::default_provider().install_default();

    tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::from_default_env().add_directive(tracing::Level::INFO.into()))
        .init();

    let args = Args::parse();

    if !args.data_dir.exists() {
        std::fs::create_dir_all(&args.data_dir)?;
    }

    let identity_path = args.data_dir.join("node.key");
    let identity = if let Some(key_hex) = args.secret_key {
        let bytes = hex::decode(key_hex)?;
        NodeIdentity::from_secret_bytes(&bytes)?
    } else if identity_path.exists() {
        info!("Loading identity from {:?}", identity_path);
        let bytes = std::fs::read(&identity_path)?;
        NodeIdentity::from_secret_bytes(&bytes)?
    } else {
        info!("Generating new identity...");
        let id = NodeIdentity::generate();
        std::fs::write(&identity_path, id.signing_key.to_bytes())?;
        info!("Identity saved to {:?}", identity_path);
        id
    };

    info!("Node ID: {}", hex::encode(identity.node_id()));
    std::fs::write(args.data_dir.join("node_id.txt"), hex::encode(identity.node_id()))?;

    let storage_path = args.data_dir.join("shards");
    let storage = DiskStorage::new(storage_path).await?;

    let service = Arc::new(P2PService::new(args.listen, identity.clone()).await?);
    info!("np2p daemon listening on {}", service.node().local_addr()?);

    // Accept connections loop
    let identity_arc = Arc::new(identity);
    loop {
        if let Some(incoming) = service.node().accept().await {
            let storage_clone = storage.clone();
            let identity_clone = identity_arc.clone();
            tokio::spawn(async move {
                match incoming.await {
                    Ok(conn) => {
                        let handler = ConnectionHandler::new(conn, storage_clone, identity_clone);
                        handler.run().await;
                    }
                    Err(e) => warn!("Incoming connection failed: {}", e),
                }
            });
        }
    }
}
