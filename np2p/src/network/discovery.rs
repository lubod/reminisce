use std::net::SocketAddr;
use serde::{Deserialize, Serialize};
use tokio::net::UdpSocket;
use tracing::{info, warn};
use crate::network::peer_registry::PeerRegistry;
use crate::crypto::{NodeIdentity, verify_signature};
use std::sync::Arc;

pub const DEFAULT_DISCOVERY_PORT: u16 = 5066;
const BROADCAST_INTERVAL_SECS: u64 = 10;
const PEER_TTL_SECS: u64 = 90;

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct DiscoveryAnnouncement {
    pub node_id: String,
    pub quic_port: u16,
    pub timestamp: u64,
    pub signature: String,
}

/// Storage nodes call this: broadcasts presence to LAN every 10s.
/// Does NOT listen — only announces.
pub fn start_broadcaster(identity: Arc<NodeIdentity>, quic_port: u16, discovery_port: u16) {
    tokio::spawn(async move {
        let socket = match UdpSocket::bind("0.0.0.0:0").await {
            Ok(s) => s,
            Err(e) => {
                warn!("[DISCOVERY] Failed to bind broadcast socket: {}", e);
                return;
            }
        };
        if let Err(e) = socket.set_broadcast(true) {
            warn!("[DISCOVERY] set_broadcast failed: {}", e);
            return;
        }

        let broadcast_addr: SocketAddr = format!("255.255.255.255:{}", discovery_port)
            .parse()
            .unwrap();

        let node_id_hex = hex::encode(identity.node_id());
        info!(
            "[DISCOVERY] Broadcaster started — node_id={} quic_port={} discovery_port={}",
            node_id_hex, quic_port, discovery_port
        );

        loop {
            let timestamp = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_secs();

            let mut msg_to_sign = Vec::new();
            msg_to_sign.extend_from_slice(&identity.node_id());
            msg_to_sign.extend_from_slice(&quic_port.to_be_bytes());
            msg_to_sign.extend_from_slice(&timestamp.to_be_bytes());

            let signature_bytes = identity.sign(&msg_to_sign);
            let signature = hex::encode(&signature_bytes);

            let announcement = DiscoveryAnnouncement {
                node_id: node_id_hex.clone(),
                quic_port,
                timestamp,
                signature,
            };

            let payload = match serde_json::to_vec(&announcement) {
                Ok(p) => p,
                Err(e) => {
                    warn!("[DISCOVERY] Serialization error: {}", e);
                    tokio::time::sleep(std::time::Duration::from_secs(BROADCAST_INTERVAL_SECS)).await;
                    continue;
                }
            };

            match socket.send_to(&payload, broadcast_addr).await {
                Ok(_) => info!("[DISCOVERY] Announced presence"),
                Err(e) => warn!("[DISCOVERY] Broadcast send failed: {}", e),
            }
            tokio::time::sleep(std::time::Duration::from_secs(BROADCAST_INTERVAL_SECS)).await;
        }
    });
}

/// Main server calls this: listens for broadcasts and populates the registry.
pub fn start_listener(registry: PeerRegistry, discovery_port: u16, our_node_id: String) {
    tokio::spawn(async move {
        let bind_addr = format!("0.0.0.0:{}", discovery_port);
        let socket = match UdpSocket::bind(&bind_addr).await {
            Ok(s) => s,
            Err(e) => {
                warn!("[DISCOVERY] Failed to bind listener on {}: {}", bind_addr, e);
                return;
            }
        };
        if let Err(e) = socket.set_broadcast(true) {
            warn!("[DISCOVERY] set_broadcast failed: {}", e);
        }

        info!("[DISCOVERY] Listener started on {}", bind_addr);

        let mut buf = [0u8; 1024];
        loop {
            match socket.recv_from(&mut buf).await {
                Ok((len, peer_addr)) => {
                    let Ok(ann) = serde_json::from_slice::<DiscoveryAnnouncement>(&buf[..len]) else {
                        continue;
                    };
                    if ann.node_id == our_node_id {
                        continue; // skip our own broadcasts
                    }

                    // Prevent peer spoofing / replay attacks
                    let current_secs = std::time::SystemTime::now()
                        .duration_since(std::time::UNIX_EPOCH)
                        .unwrap_or_default()
                        .as_secs();

                    if ann.timestamp > current_secs + 10 || current_secs.saturating_sub(ann.timestamp) > 60 {
                        warn!("[DISCOVERY] Rejected announcement from {}: expired or future timestamp", peer_addr);
                        continue;
                    }

                    let Ok(node_id_bytes) = hex::decode(&ann.node_id) else { continue };
                    let Ok(sig_bytes) = hex::decode(&ann.signature) else { continue };

                    let mut msg_to_verify = Vec::new();
                    msg_to_verify.extend_from_slice(&node_id_bytes);
                    msg_to_verify.extend_from_slice(&ann.quic_port.to_be_bytes());
                    msg_to_verify.extend_from_slice(&ann.timestamp.to_be_bytes());

                    if !verify_signature(&node_id_bytes, &msg_to_verify, &sig_bytes) {
                        warn!("[DISCOVERY] Rejected announcement from {}: invalid signature", peer_addr);
                        continue;
                    }

                    let quic_addr = SocketAddr::new(peer_addr.ip(), ann.quic_port);
                    info!("[DISCOVERY] Authenticated Peer found: {} at {}", ann.node_id, quic_addr);
                    registry.upsert(ann.node_id, quic_addr);
                }
                Err(e) => warn!("[DISCOVERY] Recv error: {}", e),
            }
        }
    });
}

/// Periodically removes peers not seen within PEER_TTL_SECS.
pub fn start_cleanup(registry: PeerRegistry) {
    tokio::spawn(async move {
        loop {
            tokio::time::sleep(std::time::Duration::from_secs(PEER_TTL_SECS)).await;
            registry.remove_stale(PEER_TTL_SECS);
        }
    });
}
