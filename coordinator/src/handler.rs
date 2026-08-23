//! Per-stream handling: registration, relay, and the per-IP rate limiter.

use std::collections::HashMap;
use std::net::{IpAddr, SocketAddr};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use tracing::{info, warn};
use np2p::network::transport::Node;
use np2p::network::protocol::{Message, Protocol};

use crate::peers::{PeerMap, PeerEntry, save_persisted_peers, current_peer_list};
use crate::types::ChannelMap;

/// Maximum size of a single relayed payload.
pub const MAX_RELAY_PAYLOAD: usize = 128 * 1024 * 1024;

/// Fixed-window per-IP rate limiter for registration / discovery / relay requests.
/// Prevents an attacker from flooding the coordinator with unauthenticated messages.
pub struct RateLimiter {
    state: Arc<Mutex<HashMap<IpAddr, (u32, Instant)>>>,
}

impl RateLimiter {
    const WINDOW: Duration = Duration::from_secs(1);
    const MAX_PER_WINDOW: u32 = 20;

    pub fn new() -> Self {
        Self {
            state: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    /// Returns true if `ip` is within its per-window request budget.
    fn allow(&self, ip: IpAddr) -> bool {
        let now = Instant::now();
        let mut map = self.state.lock().unwrap_or_else(|e| e.into_inner());
        // Opportunistic prune so the map cannot grow without bound.
        if map.len() > 100_000 {
            map.retain(|_, (_, last)| now.duration_since(*last) < Duration::from_secs(60));
        }
        let entry = map.entry(ip).or_insert((0, now));
        if now.duration_since(entry.1) >= Self::WINDOW {
            *entry = (0, now);
        }
        entry.0 += 1;
        entry.0 <= Self::MAX_PER_WINDOW
    }
}

#[allow(clippy::too_many_arguments)]
pub async fn handle_stream(
    msg: Message,
    mut send: quinn::SendStream,
    _recv: quinn::RecvStream,
    conn: quinn::Connection,
    remote_ip: IpAddr,
    peers: PeerMap,
    peer_ttl_secs: u64,
    node: Node,
    channels: ChannelMap,
    data_dir: PathBuf,
    rate_limiter: Arc<RateLimiter>,
    allowed_nodes: std::sync::Arc<Option<Vec<String>>>,
) {
    // Guard every registration / discovery / relay message with a per-IP rate limit.
    if !rate_limiter.allow(remote_ip) {
        let _ = Protocol::send(&mut send, &Message::Error {
            code: 429,
            message: "Rate limit exceeded".into(),
        }).await;
        return;
    }

    let response = match msg {
        Message::RegisterNode { node_id, quic_port, namespace } => {
            // Verify that the node_id matches the peer certificate's public key (C2)
            let mut verified = false;
            if let Some(peer_identity) = conn.peer_identity() {
                if let Some(certs) = peer_identity.downcast_ref::<Vec<rustls_pki_types::CertificateDer<'static>>>() {
                    if let Some(cert) = certs.first() {
                        let cert_bytes: &[u8] = cert.as_ref();
                        if let Some(pubkey) = np2p::crypto::extract_public_key(cert_bytes) {
                            let pubkey_hex = hex::encode(pubkey);
                            if pubkey_hex == node_id {
                                verified = true;
                            } else {
                                warn!("[COORD] Node ID mismatch: claimed {}, certificate has {}", node_id, pubkey_hex);
                            }
                        }
                    }
                }
            }

            let mut rejection: Option<Message> = None;
            if !verified {
                warn!("[COORD] Rejecting RegisterNode from {} - identity verification failed", remote_ip);
                rejection = Some(Message::Error { code: 401, message: "Identity verification failed".into() });
            } else if let Some(list) = allowed_nodes.as_ref() {
                if !list.iter().any(|id| id == &node_id) {
                    warn!("[COORD] Rejecting RegisterNode from {}: node not in admission allow-list", remote_ip);
                    rejection = Some(Message::Error { code: 403, message: "Node not allowed".into() });
                }
            }

            if let Some(resp) = rejection {
                resp
            } else {
                info!("[COORD] Register: node_id={} ns={} ip={} quic_port={}", node_id, namespace, remote_ip, quic_port);
                peers.write().unwrap_or_else(|e| e.into_inner()).insert(
                    (namespace.clone(), node_id.clone()),
                    PeerEntry { node_id, ip: remote_ip, quic_port, last_seen: Instant::now() },
                );
                save_persisted_peers(&peers, &data_dir);
                Message::PeerList { peers: current_peer_list(&peers, &namespace, peer_ttl_secs) }
            }
        }

        Message::GetPeers { namespace } => {
            info!("[COORD] GetPeers ns={} from {}", namespace, remote_ip);
            Message::PeerList { peers: current_peer_list(&peers, &namespace, peer_ttl_secs) }
        }

        Message::RelayRequest { target_node_id, payload } => {
            if payload.len() > MAX_RELAY_PAYLOAD {
                let _ = Protocol::send(&mut send, &Message::Error { code: 400, message: "Relay payload too large".into() }).await;
                return;
            }
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
    if payload.len() > MAX_RELAY_PAYLOAD {
        return;
    }

    // Try channel first (works even if target is behind NAT)
    let channel_conn = {
        let map = channels.read().unwrap_or_else(|e| e.into_inner());
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
        // Bound the relay wait: a stalled/unresponsive channel must release the
        // requester instead of hanging the relay task indefinitely.
        let mut len_buf = [0u8; 4];
        if tokio::time::timeout(std::time::Duration::from_secs(30), tr.read_exact(&mut len_buf)).await.is_err() { return; }
        let resp_len = u32::from_be_bytes(len_buf) as usize;
        if resp_len > MAX_RELAY_PAYLOAD { return; }
        let mut resp_payload = vec![0u8; resp_len];
        if tokio::time::timeout(std::time::Duration::from_secs(30), tr.read_exact(&mut resp_payload)).await.is_err() { return; }
        let _ = Protocol::send(send, &Message::RelayResponse { payload: resp_payload }).await;
        let _ = send.finish();
        return;
    }

    // Fall back to direct connection
    let target_addr = {
        let map = peers.read().unwrap_or_else(|e| e.into_inner());
        map.values()
            .find(|e| e.node_id == target_node_id && e.last_seen.elapsed().as_secs() < peer_ttl_secs)
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
        node.connect(target_addr, target_node_id),
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

    // Bound the whole post-connect exchange (open_bi + write + read-back) so a
    // stalled/unresponsive target releases the requester with a 504 instead of
    // hanging the relay task indefinitely.
    let len = payload.len() as u32;
    let exchange = async {
        let (mut ts, mut tr) = conn.open_bi().await.map_err(|e| e.to_string())?;
        ts.write_all(&len.to_be_bytes()).await.map_err(|e| e.to_string())?;
        ts.write_all(&payload).await.map_err(|e| e.to_string())?;
        let _ = ts.finish();

        let mut len_buf = [0u8; 4];
        tr.read_exact(&mut len_buf).await.map_err(|e| e.to_string())?;
        let resp_len = u32::from_be_bytes(len_buf) as usize;
        if resp_len > MAX_RELAY_PAYLOAD {
            return Err("target response exceeds relay limit".to_string());
        }
        let mut resp_payload = vec![0u8; resp_len];
        tr.read_exact(&mut resp_payload).await.map_err(|e| e.to_string())?;
        Ok(resp_payload)
    };

    match tokio::time::timeout(std::time::Duration::from_secs(30), exchange).await {
        Ok(Ok(resp_payload)) => {
            let _ = Protocol::send(send, &Message::RelayResponse { payload: resp_payload }).await;
            let _ = send.finish();
        }
        Ok(Err(e)) => {
            warn!("[RELAY] Exchange with {} failed: {}", target_addr, e);
        }
        Err(_) => {
            warn!("[RELAY] Exchange with {} timed out after 30s", target_addr);
            let _ = Protocol::send(send, &Message::Error {
                code: 504,
                message: format!("Relay to {} timed out", target_node_id),
            }).await;
            let _ = send.finish();
        }
    }
}
