use std::net::SocketAddr;
use serde::{Deserialize, Serialize};
use tokio::net::UdpSocket;
use tracing::{info, warn};
use crate::network::peer_registry::PeerRegistry;

pub const DEFAULT_DISCOVERY_PORT: u16 = 5060;
const BROADCAST_INTERVAL_SECS: u64 = 10;
const PEER_TTL_SECS: u64 = 90;

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct DiscoveryAnnouncement {
    pub node_id: String,
    pub quic_port: u16,
}

/// Storage nodes call this: broadcasts presence to LAN every 10s.
/// Does NOT listen — only announces.
pub fn start_broadcaster(node_id: String, quic_port: u16, discovery_port: u16) {
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

        let announcement = DiscoveryAnnouncement {
            node_id: node_id.clone(),
            quic_port,
        };
        let payload = match serde_json::to_vec(&announcement) {
            Ok(p) => p,
            Err(e) => {
                warn!("[DISCOVERY] Serialization error: {}", e);
                return;
            }
        };

        info!(
            "[DISCOVERY] Broadcaster started — node_id={} quic_port={} discovery_port={}",
            node_id, quic_port, discovery_port
        );

        loop {
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
                    let Ok(ann) = serde_json::from_slice::<DiscoveryAnnouncement>(&buf[..len])
                    else {
                        continue;
                    };
                    if ann.node_id == our_node_id {
                        continue; // skip our own broadcasts
                    }
                    let quic_addr = SocketAddr::new(peer_addr.ip(), ann.quic_port);
                    info!("[DISCOVERY] Peer found: {} at {}", ann.node_id, quic_addr);
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
