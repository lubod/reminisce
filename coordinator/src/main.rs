use std::collections::HashMap;
use std::io::BufReader;
use std::net::{IpAddr, SocketAddr};
use std::path::PathBuf;
use std::sync::{Arc, RwLock};
use std::time::Instant;
use clap::Parser;
use tokio::io::{AsyncRead, AsyncWrite};
use tokio_rustls::TlsAcceptor;
use tracing::{info, warn};
use np2p::crypto::{NodeIdentity, verify_signature};
use np2p::network::transport::Node;
use np2p::network::protocol::{Message, Protocol};

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
    /// If not set, any node can register (insecure). Get this from the home server startup log.
    #[arg(long)]
    allowed_tunnel_node_id: Option<String>,
}

// ── Peer registry ────────────────────────────────────────────────────────────

struct PeerEntry {
    node_id: String,
    ip: IpAddr,
    quic_port: u16,
    last_seen: Instant,
}

type PeerMap = Arc<RwLock<HashMap<String, PeerEntry>>>;

fn current_peer_list(peers: &PeerMap, ttl: u64) -> Vec<(String, String)> {
    peers
        .read()
        .unwrap()
        .values()
        .filter(|p| p.last_seen.elapsed().as_secs() < ttl)
        .map(|p| (p.node_id.clone(), format!("{}:{}", p.ip, p.quic_port)))
        .collect()
}

// ── Tunnel registry ───────────────────────────────────────────────────────────

/// Maps node_id → QUIC connection kept alive by the home server.
type TunnelMap = Arc<RwLock<HashMap<String, quinn::Connection>>>;

/// Maps node_id → persistent reverse QUIC connection from a storage node behind NAT.
type ChannelMap = Arc<RwLock<HashMap<String, quinn::Connection>>>;

// ── Per-stream handler ────────────────────────────────────────────────────────

/// Handle one QUIC stream. `msg` is already read by the caller.
async fn handle_stream(
    msg: Message,
    mut send: quinn::SendStream,
    mut recv: quinn::RecvStream,
    remote_ip: IpAddr,
    peers: PeerMap,
    peer_ttl_secs: u64,
    node: Node,
    channels: ChannelMap,
) {
    let response = match msg {
        Message::RegisterNode { node_id, quic_port } => {
            info!("[COORD] Register: node_id={} ip={} quic_port={}", node_id, remote_ip, quic_port);
            peers.write().unwrap().insert(
                node_id.clone(),
                PeerEntry { node_id, ip: remote_ip, quic_port, last_seen: Instant::now() },
            );
            Message::PeerList { peers: current_peer_list(&peers, peer_ttl_secs) }
        }

        Message::GetPeers => {
            info!("[COORD] GetPeers from {}", remote_ip);
            Message::PeerList { peers: current_peer_list(&peers, peer_ttl_secs) }
        }

        Message::RelayRequest { target_node_id, payload } => {
            relay(&mut send, &peers, peer_ttl_secs, &node, &target_node_id, payload, &channels).await;
            return;
        }

        _ => Message::Error { code: 400, message: "Unexpected message".into() },
    };

    let _ = Protocol::send(&mut send, &response).await;
    let _ = send.finish();
}

// ── Relay ─────────────────────────────────────────────────────────────────────

async fn relay(
    send: &mut quinn::SendStream,
    peers: &PeerMap,
    peer_ttl_secs: u64,
    node: &Node,
    target_node_id: &str,
    payload: Vec<u8>,
    channels: &ChannelMap,
) {
    // Try channel first (works even if target is behind NAT)
    let channel_conn = {
        let map = channels.read().unwrap();
        map.get(target_node_id).cloned()
    };

    if let Some(conn) = channel_conn {
        info!("[RELAY] {} → via channel", target_node_id);
        let (mut ts, mut tr) = match conn.open_bi().await {
            Ok(s) => s,
            Err(e) => {
                let _ = Protocol::send(send, &Message::Error { code: 503, message: e.to_string() }).await;
                return;
            }
        };
        let len = payload.len() as u32;
        if ts.write_all(&len.to_be_bytes()).await.is_err() || ts.write_all(&payload).await.is_err() {
            return;
        }
        let _ = ts.finish();
        let mut len_buf = [0u8; 4];
        if tr.read_exact(&mut len_buf).await.is_err() { return; }
        let resp_len = u32::from_be_bytes(len_buf) as usize;
        if resp_len > 100 * 1024 * 1024 { return; }
        let mut resp_payload = vec![0u8; resp_len];
        if tr.read_exact(&mut resp_payload).await.is_err() { return; }
        let _ = Protocol::send(send, &Message::RelayResponse { payload: resp_payload }).await;
        let _ = send.finish();
        return;
    }

    // Fall back to direct connection
    let target_addr = {
        let map = peers.read().unwrap();
        map.get(target_node_id)
            .filter(|e| e.last_seen.elapsed().as_secs() < peer_ttl_secs)
            .map(|e| SocketAddr::new(e.ip, e.quic_port))
    };
    let target_addr = match target_addr {
        Some(a) => a,
        None => {
            warn!("[RELAY] Target not found: {}", target_node_id);
            let _ = Protocol::send(send, &Message::Error {
                code: 404,
                message: format!("Target peer not found: {}", target_node_id),
            }).await;
            return;
        }
    };

    info!("[RELAY] {} → {}", target_node_id, target_addr);

    let conn = match tokio::time::timeout(
        std::time::Duration::from_secs(10),
        node.connect(target_addr),
    ).await {
        Ok(Ok(c)) => c,
        Ok(Err(e)) => {
            warn!("[RELAY] Connect to {} failed: {}", target_addr, e);
            let _ = Protocol::send(send, &Message::Error { code: 503, message: e.to_string() }).await;
            return;
        }
        Err(_) => {
            warn!("[RELAY] Connect to {} timed out", target_addr);
            let _ = Protocol::send(send, &Message::Error { code: 504, message: "Timed out".into() }).await;
            return;
        }
    };

    let (mut ts, mut tr) = match conn.open_bi().await {
        Ok(s) => s,
        Err(e) => {
            let _ = Protocol::send(send, &Message::Error { code: 503, message: e.to_string() }).await;
            return;
        }
    };

    let len = payload.len() as u32;
    if ts.write_all(&len.to_be_bytes()).await.is_err() || ts.write_all(&payload).await.is_err() {
        return;
    }
    let _ = ts.finish();

    let mut len_buf = [0u8; 4];
    if tr.read_exact(&mut len_buf).await.is_err() { return; }
    let resp_len = u32::from_be_bytes(len_buf) as usize;
    if resp_len > 100 * 1024 * 1024 { return; }
    let mut resp_payload = vec![0u8; resp_len];
    if tr.read_exact(&mut resp_payload).await.is_err() { return; }

    let _ = Protocol::send(send, &Message::RelayResponse { payload: resp_payload }).await;
    let _ = send.finish();
}

// ── TCP tunnel listener ───────────────────────────────────────────────────────

fn load_tls_acceptor(cert: &PathBuf, key: &PathBuf) -> anyhow::Result<TlsAcceptor> {
    let cert_file = std::fs::File::open(cert)?;
    let key_file = std::fs::File::open(key)?;

    let certs: Vec<_> = rustls_pemfile::certs(&mut BufReader::new(cert_file))
        .collect::<Result<_, _>>()?;
    let private_key = rustls_pemfile::private_key(&mut BufReader::new(key_file))?
        .ok_or_else(|| anyhow::anyhow!("No private key found in {:?}", key))?;

    let config = rustls::ServerConfig::builder()
        .with_no_client_auth()
        .with_single_cert(certs, private_key)?;

    Ok(TlsAcceptor::from(Arc::new(config)))
}

async fn pipe<R, W>(
    mut client_read: R,
    mut client_write: W,
    mut quic_recv: quinn::RecvStream,
    mut quic_send: quinn::SendStream,
) where
    R: AsyncRead + Unpin,
    W: AsyncWrite + Unpin,
{
    let to_home = tokio::io::copy(&mut client_read, &mut quic_send);
    let to_client = tokio::io::copy(&mut quic_recv, &mut client_write);
    tokio::select! { _ = to_home => {}, _ = to_client => {} }
}

/// Listen on a TCP port; pipe each connection to the home server via QUIC tunnel.
/// If `tls_acceptor` is provided, terminates TLS — use with a Let's Encrypt cert.
fn start_tcp_tunnel_listener(tunnel_port: u16, tunnels: TunnelMap, tls_acceptor: Option<TlsAcceptor>) {
    let tls_acceptor = tls_acceptor.map(Arc::new);

    tokio::spawn(async move {
        let addr: SocketAddr = format!("0.0.0.0:{}", tunnel_port).parse().unwrap();
        let listener = match tokio::net::TcpListener::bind(addr).await {
            Ok(l) => l,
            Err(e) => { warn!("[TUNNEL] Failed to bind TCP port {}: {}", tunnel_port, e); return; }
        };

        let tls_label = if tls_acceptor.is_some() { "HTTPS" } else { "HTTP" };
        info!("[TUNNEL] {} listener on :{} (Android → home server)", tls_label, tunnel_port);

        loop {
            let Ok((tcp_stream, client_addr)) = listener.accept().await else { continue };
            let tunnels = tunnels.clone();
            let tls = tls_acceptor.clone();

            tokio::spawn(async move {
                let tunnel_conn = {
                    let map = tunnels.read().unwrap();
                    map.values().next().cloned()
                };
                let tunnel_conn = match tunnel_conn {
                    Some(c) => c,
                    None => { warn!("[TUNNEL] No home server registered — dropping {}", client_addr); return; }
                };
                let (mut qs, mut qr) = match tunnel_conn.open_bi().await {
                    Ok(s) => s,
                    Err(e) => { warn!("[TUNNEL] open_bi failed: {}", e); return; }
                };

                if let Some(acceptor) = tls {
                    match acceptor.accept(tcp_stream).await {
                        Ok(tls_stream) => {
                            let (r, w) = tokio::io::split(tls_stream);
                            pipe(r, w, qr, qs).await;
                        }
                        Err(e) => warn!("[TUNNEL] TLS handshake failed from {}: {}", client_addr, e),
                    }
                } else {
                    let (r, w) = tcp_stream.into_split();
                    pipe(r, w, qr, qs).await;
                }
            });
        }
    });
}

// ── Main ──────────────────────────────────────────────────────────────────────

#[tokio::main]
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

    let peers: PeerMap = Arc::new(RwLock::new(HashMap::new()));
    let tunnels: TunnelMap = Arc::new(RwLock::new(HashMap::new()));
    let channels: ChannelMap = Arc::new(RwLock::new(HashMap::new()));

    // Background peer cleanup
    {
        let peers = peers.clone();
        let ttl = args.peer_ttl_secs;
        tokio::spawn(async move {
            loop {
                tokio::time::sleep(std::time::Duration::from_secs(30)).await;
                let mut map = peers.write().unwrap();
                let before = map.len();
                map.retain(|_, p| p.last_seen.elapsed().as_secs() < ttl);
                let removed = before - map.len();
                if removed > 0 {
                    info!("[COORD] Cleaned {} stale peers, {} active", removed, map.len());
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
    start_tcp_tunnel_listener(args.tunnel_port, tunnels.clone(), tls_acceptor);

    let allowed_tunnel_node_id = args.allowed_tunnel_node_id.clone();
    if allowed_tunnel_node_id.is_none() {
        warn!("[TUNNEL] --allowed-tunnel-node-id not set — any node can register as tunnel backend!");
    }

    // QUIC accept loop
    loop {
        if let Some(incoming) = node.accept().await {
            let peers = peers.clone();
            let tunnels = tunnels.clone();
            let channels = channels.clone();
            let ttl = args.peer_ttl_secs;
            let node_for_task = node.clone();
            let allowed_node_id = allowed_tunnel_node_id.clone();

            tokio::spawn(async move {
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
                    tunnels.write().unwrap().insert(node_id.clone(), conn.clone());
                    info!("[TUNNEL] Registered: {} from {}", node_id, remote_ip);
                    conn.closed().await;
                    tunnels.write().unwrap().remove(&node_id);
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
                    channels.write().unwrap().insert(node_id.clone(), conn.clone());
                    info!("[CHANNEL] Registered: {} from {}", node_id, remote_ip);
                    conn.closed().await;
                    channels.write().unwrap().remove(&node_id);
                    info!("[CHANNEL] Disconnected: {}", node_id);
                } else {
                    // ── Normal P2P connection ─────────────────────────────────
                    // Handle first message, then loop for more streams
                    tokio::spawn(handle_stream(
                        first_msg, first_send, first_recv,
                        remote_ip, peers.clone(), ttl, node_for_task.clone(), channels.clone(),
                    ));

                    loop {
                        match conn.accept_bi().await {
                            Ok((send, mut recv)) => {
                                let Ok(msg) = Protocol::receive(&mut recv).await else { continue };
                                let peers = peers.clone();
                                let node = node_for_task.clone();
                                let channels = channels.clone();
                                tokio::spawn(handle_stream(msg, send, recv, remote_ip, peers, ttl, node, channels));
                            }
                            Err(_) => break,
                        }
                    }
                }
            });
        }
    }
}
