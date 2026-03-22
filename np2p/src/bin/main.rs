use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::Arc;
use tracing::{info, warn};
use tracing_subscriber::EnvFilter;
use clap::Parser;
use np2p::crypto::NodeIdentity;
use np2p::network::{P2PService, ConnectionHandler};
use np2p::network::discovery;
use np2p::network::coordinator;
use np2p::storage::DiskStorage;

#[derive(Parser, Debug)]
#[command(author, version, about = "np2p storage node daemon", long_about = None)]
struct Args {
    /// Address to listen on for P2P (QUIC) connections
    #[arg(short, long, default_value = "0.0.0.0:5050")]
    listen: SocketAddr,

    /// Directory to store shards and identity
    #[arg(short, long, default_value = "/data")]
    data_dir: PathBuf,

    /// Secret key in hex (optional, generates new if missing)
    #[arg(short, long)]
    secret_key: Option<String>,

    /// UDP port to broadcast discovery announcements on (LAN discovery)
    #[arg(long, default_value_t = discovery::DEFAULT_DISCOVERY_PORT)]
    discovery_port: u16,

    /// Coordinator QUIC address for cross-network discovery and relay (e.g. 1.2.3.4:5055)
    #[arg(long)]
    coordinator_addr: Option<SocketAddr>,

    /// Namespace for coordinator peer isolation (e.g. "production", "dev")
    #[arg(long, default_value = "default")]
    namespace: String,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
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

    let node_id_hex = hex::encode(identity.node_id());
    info!("Node ID: {}", node_id_hex);
    std::fs::write(args.data_dir.join("node_id.txt"), &node_id_hex)?;

    let storage_path = args.data_dir.join("shards");
    let storage = DiskStorage::new(storage_path).await?;

    let mut service = P2PService::new(args.listen, identity.clone()).await?;
    let quic_port = service.node().local_addr()?.port();
    info!("np2p daemon listening on {}", service.node().local_addr()?);

    // LAN broadcast — announces our QUIC port to all nodes on the same subnet
    discovery::start_broadcaster(node_id_hex.clone(), quic_port, args.discovery_port);
    info!("Broadcasting on discovery port {}", args.discovery_port);

    // Coordinator — for nodes on different networks
    if let Some(addr) = args.coordinator_addr {
        service.coordinator_addr = Some(addr);
        coordinator::start_coordinator_client(
            addr,
            service.node().clone(),
            node_id_hex.clone(),
            Some(quic_port),
            service.registry.clone(),
            args.namespace.clone(),
        );

        // Reverse channel — so coordinator can relay messages to us even when behind NAT
        np2p::network::channel::start_channel_client(
            addr,
            service.node().clone(),
            identity.clone(),
            storage.clone(),
        );
    } else {
        info!("No coordinator configured — LAN discovery only");
    }

    let service = Arc::new(service);
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
