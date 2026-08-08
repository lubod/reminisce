//! Coordinator service: peer registry, QUIC relay, and reverse tunnel.
//!
//! Runs on a VPS. Storage nodes and home servers register here so they can find
//! each other across NATs. Also proxies bidirectional QUIC streams between peers
//! that cannot connect directly, and maintains a reverse tunnel so Android clients
//! can reach the home server from outside the LAN.

mod handler;
mod peers;
mod tunnel;
mod types;

use std::collections::HashMap;
use std::path::PathBuf;
use std::net::SocketAddr;
use std::sync::{Arc, RwLock};
use clap::Parser;
use tokio::sync::Semaphore;
use tracing::{info, warn};
use np2p::crypto::{NodeIdentity, verify_signature};
use np2p::network::transport::Node;
use np2p::network::protocol::{Message, Protocol};

use crate::handler::{RateLimiter, handle_stream};
use crate::peers::{PeerMap, load_persisted_peers, save_persisted_peers};
use crate::tunnel::{load_tls_acceptor, start_tcp_tunnel_listener};
use crate::types::{TunnelMap, ChannelMap};

/// Maximum concurrent QUIC connections the coordinator will serve. Bounds the
/// unbounded `tokio::spawn` per connection (memory-DoS protection).
const MAX_CONNECTIONS: usize = 512;

#[derive(Parser)]
#[command(about = "Reminisce P2P coordinator — runs on VPS")]
struct Args {
    /// QUIC address for P2P registration, relay, and tunnel registration
    #[arg(short, long, default_value = "0.0.0.0:5055")]
    listen: SocketAddr,

    /// TCP port that Android clients connect to for tunneled home-server access
    #[arg(long, default_value_t = 8443)]
    tunnel_port: u16,

    /// TLS certificate file (PEM) for the tunnel TCP port — get from Let's Encrypt
    #[arg(long)]
    tls_cert: Option<PathBuf>,

    /// TLS private key file (PEM) for the tunnel TCP port
    #[arg(long)]
    tls_key: Option<PathBuf>,

    #[arg(long, default_value = "/data")]
    data_dir: PathBuf,

    /// Seconds before a peer that stopped re-registering is removed
    #[arg(long, default_value_t = 60)]
    peer_ttl_secs: u64,

    /// Hex-encoded Ed25519 public key (node_id) of the home server allowed to register the tunnel.
    /// If not set, tunnel registration is REFUSED unless --allow-any-tunnel is given.
    /// Get this from the home server startup log.
    #[arg(long)]
    allowed_tunnel_node_id: Option<String>,

    /// Explicitly allow ANY node to register as the tunnel backend (insecure — a rogue
    /// peer could hijack Android→home traffic). Only set this on trusted/private setups.
    #[arg(long)]
    allow_any_tunnel: bool,
}

#[tokio::main(flavor = "multi_thread")]
async fn main() -> anyhow::Result<()> {
    let _ = rustls::crypto::ring::default_provider().install_default();
    tracing_subscriber::fmt::init();

    let args = Args::parse();
    std::fs::create_dir_all(&args.data_dir)?;

    let identity_path = args.data_dir.join("coordinator.key");
    let identity = if identity_path.exists() {
        let bytes = std::fs::read(&identity_path)?;
        NodeIdentity::from_secret_bytes(&bytes)?
    } else {
        let id = NodeIdentity::generate();
        std::fs::write(&identity_path, id.signing_key.to_bytes())?;
        id
    };

    info!("Coordinator node_id: {}", hex::encode(identity.node_id()));

    let node = Node::new(args.listen, identity)?;
    info!("Coordinator QUIC on {}", args.listen);

    let peers_map = load_persisted_peers(&args.data_dir);
    let peers: PeerMap = Arc::new(RwLock::new(peers_map));
    let tunnels: TunnelMap = Arc::new(RwLock::new(HashMap::new()));
    let channels: ChannelMap = Arc::new(RwLock::new(HashMap::new()));

    // Background peer cleanup
    {
        let peers = peers.clone();
        let ttl = args.peer_ttl_secs;
        let data_dir = args.data_dir.clone();
        tokio::spawn(async move {
            loop {
                tokio::time::sleep(std::time::Duration::from_secs(30)).await;
                let mut map = peers.write().unwrap_or_else(|e| e.into_inner());
                let before = map.len();
                map.retain(|_key, p| p.last_seen.elapsed().as_secs() < ttl);
                let removed = before - map.len();
                if removed > 0 {
                    info!("[COORD] Cleaned {} stale peers, {} active", removed, map.len());
                }
                drop(map);
                if removed > 0 {
                    save_persisted_peers(&peers, &data_dir);
                }
            }
        });
    }

    // TCP tunnel listener for Android clients (optionally TLS-terminated)
    let tls_acceptor = match (args.tls_cert.as_ref(), args.tls_key.as_ref()) {
        (Some(cert), Some(key)) => match load_tls_acceptor(cert, key) {
            Ok(a) => { info!("[TUNNEL] TLS enabled with cert {:?}", cert); Some(a) }
            Err(e) => { warn!("[TUNNEL] Failed to load TLS cert: {} — falling back to plain TCP", e); None }
        },
        _ => { info!("[TUNNEL] No TLS cert provided — tunnel will use plain TCP"); None }
    };
    start_tcp_tunnel_listener(args.tunnel_port, tunnels.clone(), tls_acceptor, args.allowed_tunnel_node_id.clone());

    let allowed_tunnel_node_id = args.allowed_tunnel_node_id.clone();
    let allow_any_tunnel = args.allow_any_tunnel;
    if allowed_tunnel_node_id.is_none() && !allow_any_tunnel {
        warn!("[TUNNEL] --allowed-tunnel-node-id not set and --allow-any-tunnel not given — tunnel registration will be REFUSED");
    }

    let rate_limiter = Arc::new(RateLimiter::new());
    let conn_permits = Arc::new(Semaphore::new(MAX_CONNECTIONS));

    // QUIC accept loop
    loop {
        if let Some(incoming) = node.accept().await {
            let peers = peers.clone();
            let tunnels = tunnels.clone();
            let channels = channels.clone();
            let ttl = args.peer_ttl_secs;
            let node_for_task = node.clone();
            let allowed_node_id = allowed_tunnel_node_id.clone();
            let allow_any = allow_any_tunnel;
            let data_dir_owned = args.data_dir.clone();
            let limiter = rate_limiter.clone();
            let permits = conn_permits.clone();
            let permit = match permits.try_acquire_owned() {
                Ok(p) => p,
                Err(_) => {
                    warn!("[COORD] At max concurrent connections ({}) — dropping incoming connection", MAX_CONNECTIONS);
                    continue;
                }
            };

            tokio::spawn(async move {
                let _permit = permit; // held for the connection's lifetime
                let conn = match incoming.await {
                    Ok(c) => c,
                    Err(e) => { warn!("[COORD] Incoming connection failed: {}", e); return; }
                };
                let remote_ip = conn.remote_address().ip();

                // Read first stream + first message to determine connection type
                let Ok((mut first_send, mut first_recv)) = conn.accept_bi().await else { return };
                let Ok(first_msg) = Protocol::receive(&mut first_recv).await else { return };

                if let Message::TunnelRegister { ref node_id } = first_msg {
                    // ── Tunnel connection: challenge-response authentication ────
                    let node_id = node_id.clone();

                    // Check node_id is in the allowed list (if configured)
                    if let Some(ref allowed) = allowed_node_id {
                        if &node_id != allowed {
                            warn!("[TUNNEL] Rejected {} from {} — not in allowed list", node_id, remote_ip);
                            let _ = Protocol::send(&mut first_send, &Message::Error {
                                code: 403,
                                message: "Node ID not allowed".into(),
                            }).await;
                            return;
                        }
                    } else if !allow_any {
                        // No allowlist and no explicit --allow-any-tunnel: refuse. Without
                        // this, a rogue peer could register as the tunnel backend and
                        // hijack Android→home traffic.
                        warn!("[TUNNEL] Rejected {} from {} — tunnel backend not configured (set --allowed-tunnel-node-id or --allow-any-tunnel)", node_id, remote_ip);
                        let _ = Protocol::send(&mut first_send, &Message::Error {
                            code: 403,
                            message: "Tunnel registration disabled — set --allowed-tunnel-node-id".into(),
                        }).await;
                        return;
                    }

                    // Issue a cryptographically random 32-byte nonce challenge
                    let nonce: Vec<u8> = rand::random::<[u8; 32]>().to_vec();

                    if Protocol::send(&mut first_send, &Message::TunnelChallenge { nonce: nonce.clone() }).await.is_err() {
                        return;
                    }

                    // Verify the signature
                    let signature = match Protocol::receive(&mut first_recv).await {
                        Ok(Message::TunnelChallengeResponse { signature }) => signature,
                        _ => {
                            warn!("[TUNNEL] Expected ChallengeResponse from {}", remote_ip);
                            return;
                        }
                    };

                    let node_id_bytes = match hex::decode(&node_id) {
                        Ok(b) => b,
                        Err(_) => { warn!("[TUNNEL] Invalid node_id hex from {}", remote_ip); return; }
                    };

                    if !verify_signature(&node_id_bytes, &nonce, &signature) {
                        warn!("[TUNNEL] Signature verification failed for {} from {}", node_id, remote_ip);
                        let _ = Protocol::send(&mut first_send, &Message::Error {
                            code: 401,
                            message: "Signature verification failed".into(),
                        }).await;
                        return;
                    }

                    let _ = Protocol::send(&mut first_send, &Message::TunnelAccepted).await;
                    let _ = first_send.finish();
                    tunnels.write().unwrap_or_else(|e| e.into_inner()).insert(node_id.clone(), conn.clone());
                    info!("[TUNNEL] Registered: {} from {}", node_id, remote_ip);
                    conn.closed().await;
                    // Only remove if this is still the same connection we registered.
                    // A reconnect may have replaced it already; removing a fresh entry
                    // would leave TunnelMap empty and break the next Android request.
                    {
                        let mut map = tunnels.write().unwrap_or_else(|e| e.into_inner());
                        if map.get(&node_id).map(|c| c.stable_id() == conn.stable_id()).unwrap_or(false) {
                            map.remove(&node_id);
                        }
                    }
                    info!("[TUNNEL] Disconnected: {}", node_id);
                } else if let Message::NodeChannelRegister { ref node_id } = first_msg {
                    // ── Channel connection: challenge-response authentication ───
                    let node_id = node_id.clone();

                    let nonce: Vec<u8> = rand::random::<[u8; 32]>().to_vec();
                    if Protocol::send(&mut first_send, &Message::NodeChannelChallenge { nonce: nonce.clone() }).await.is_err() {
                        return;
                    }

                    let signature = match Protocol::receive(&mut first_recv).await {
                        Ok(Message::NodeChannelChallengeResponse { signature }) => signature,
                        _ => { warn!("[CHANNEL] Expected ChallengeResponse from {}", remote_ip); return; }
                    };

                    let node_id_bytes = match hex::decode(&node_id) {
                        Ok(b) => b,
                        Err(_) => { warn!("[CHANNEL] Invalid node_id hex from {}", remote_ip); return; }
                    };

                    if !verify_signature(&node_id_bytes, &nonce, &signature) {
                        warn!("[CHANNEL] Signature verification failed for {} from {}", node_id, remote_ip);
                        let _ = Protocol::send(&mut first_send, &Message::Error {
                            code: 401,
                            message: "Signature verification failed".into(),
                        }).await;
                        return;
                    }

                    let _ = Protocol::send(&mut first_send, &Message::NodeChannelAccepted).await;
                    let _ = first_send.finish();
                    channels.write().unwrap_or_else(|e| e.into_inner()).insert(node_id.clone(), conn.clone());
                    info!("[CHANNEL] Registered: {} from {}", node_id, remote_ip);
                    conn.closed().await;
                    // Only remove OUR registration: a reconnecting node may have
                    // already inserted a fresh entry (different connection), and the
                    // stale disconnect task must not wipe it (see tunnel path).
                    let mut map = channels.write().unwrap_or_else(|e| e.into_inner());
                    if map.get(&node_id).map(|c| c.stable_id() == conn.stable_id()).unwrap_or(false) {
                        map.remove(&node_id);
                    }
                    drop(map);
                    info!("[CHANNEL] Disconnected: {}", node_id);
                } else {
                    // ── Normal P2P connection ─────────────────────────────────
                    // Handle first message, then loop for more streams
                    let limiter_first = limiter.clone();
                    tokio::spawn(handle_stream(
                        first_msg, first_send, first_recv, conn.clone(),
                        remote_ip, peers.clone(), ttl, node_for_task.clone(), channels.clone(),
                        data_dir_owned.clone(), limiter_first,
                    ));

                    while let Ok((send, mut recv)) = conn.accept_bi().await {
                        let Ok(msg) = Protocol::receive(&mut recv).await else { continue };
                        let peers = peers.clone();
                        let node = node_for_task.clone();
                        let channels = channels.clone();
                        let conn_clone = conn.clone();
                        let data_dir = data_dir_owned.clone();
                        let limiter = limiter.clone();
                        tokio::spawn(handle_stream(msg, send, recv, conn_clone, remote_ip, peers, ttl, node, channels, data_dir, limiter));
                    }
                }
            });
        }
    }
}
