use std::net::{IpAddr, SocketAddr};
use serde::{Deserialize, Serialize};
use tokio::net::UdpSocket;
use tracing::{info, warn};
use crate::network::peer_registry::PeerRegistry;
use crate::crypto::{NodeIdentity, verify_signature};
use std::sync::Arc;

pub const DEFAULT_DISCOVERY_PORT: u16 = 5066;
const BROADCAST_INTERVAL_SECS: u64 = 10;

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct DiscoveryAnnouncement {
    pub node_id: String,
    pub quic_port: u16,
    pub timestamp: u64,
    pub signature: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub advertise_ip: Option<IpAddr>,
}

/// Builds the canonical byte payload signed/verified for a `DiscoveryAnnouncement`.
/// When `advertise_ip` is `None`, the output is identical to the legacy format.
pub fn build_announcement_signing_payload(
    node_id_bytes: &[u8],
    quic_port: u16,
    timestamp: u64,
    advertise_ip: Option<IpAddr>,
) -> Vec<u8> {
    let mut msg = Vec::new();
    msg.extend_from_slice(node_id_bytes);
    msg.extend_from_slice(&quic_port.to_be_bytes());
    msg.extend_from_slice(&timestamp.to_be_bytes());
    if let Some(ip) = advertise_ip {
        msg.extend_from_slice(ip.to_string().as_bytes());
    }
    msg
}

/// Normalizes a unicast announcement target (`ip`, `host`, or `host:port`),
/// appending `:{default_port}` when no port is specified.
pub fn normalize_announce_target(raw: &str, default_port: u16) -> String {
    let trimmed = raw.trim();
    if trimmed.parse::<SocketAddr>().is_ok() {
        return trimmed.to_string();
    }
    if let Ok(ip) = trimmed.parse::<IpAddr>() {
        return match ip {
            IpAddr::V4(v4) => format!("{}:{}", v4, default_port),
            IpAddr::V6(v6) => format!("[{}]:{}", v6, default_port),
        };
    }
    if trimmed.starts_with('[') && trimmed.ends_with(']') {
        return format!("{}:{}", trimmed, default_port);
    }
    if trimmed.contains(':') {
        trimmed.to_string()
    } else {
        format!("{}:{}", trimmed, default_port)
    }
}

/// Determines the local source IP the OS routing table selects when sending UDP to `target`.
/// Used on overlay networks like Tailscale so a storage node behind Docker NAT advertises its
/// reachable overlay IP rather than letting intermediate NAT rewrite the source address.
pub fn detect_local_ip_for_target(target: SocketAddr) -> Option<IpAddr> {
    let bind_addr = if target.is_ipv6() { "[::]:0" } else { "0.0.0.0:0" };
    let sock = std::net::UdpSocket::bind(bind_addr).ok()?;
    sock.connect(target).ok()?;
    let ip = sock.local_addr().ok()?.ip();
    if ip.is_unspecified() || ip.is_loopback() {
        None
    } else {
        Some(ip)
    }
}

fn build_signed_announcement(
    identity: &NodeIdentity,
    node_id_hex: &str,
    quic_port: u16,
    timestamp: u64,
    advertise_ip: Option<IpAddr>,
) -> DiscoveryAnnouncement {
    let msg_to_sign =
        build_announcement_signing_payload(&identity.node_id(), quic_port, timestamp, advertise_ip);
    let signature_bytes = identity.sign(&msg_to_sign);
    DiscoveryAnnouncement {
        node_id: node_id_hex.to_string(),
        quic_port,
        timestamp,
        signature: hex::encode(&signature_bytes),
        advertise_ip,
    }
}

/// Storage nodes call this: broadcasts presence to LAN every 10s.
/// Does NOT listen — only announces.
pub fn start_broadcaster(identity: Arc<NodeIdentity>, quic_port: u16, discovery_port: u16) {
    start_broadcaster_with_targets(identity, quic_port, discovery_port, Vec::new(), None);
}

/// Storage nodes call this: broadcasts presence to LAN and sends signed unicast
/// ("write-in") announcements to configured targets (e.g. home server over Tailscale) every 10s.
pub fn start_broadcaster_with_targets(
    identity: Arc<NodeIdentity>,
    quic_port: u16,
    discovery_port: u16,
    announce_targets: Vec<String>,
    advertise_ip: Option<IpAddr>,
) {
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
        let socket_v6 = UdpSocket::bind("[::]:0").await.ok();

        let broadcast_addr: SocketAddr = format!("255.255.255.255:{}", discovery_port)
            .parse()
            .unwrap();

        let node_id_hex = hex::encode(identity.node_id());
        info!(
            "[DISCOVERY] Broadcaster started — node_id={} quic_port={} discovery_port={} unicast_targets={:?}",
            node_id_hex, quic_port, discovery_port, announce_targets
        );

        loop {
            let timestamp = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_secs();

            let broadcast_ann = build_signed_announcement(
                &identity,
                &node_id_hex,
                quic_port,
                timestamp,
                advertise_ip,
            );

            match serde_json::to_vec(&broadcast_ann) {
                Ok(payload) => match socket.send_to(&payload, broadcast_addr).await {
                    Ok(_) => info!("[DISCOVERY] Announced presence (LAN broadcast)"),
                    Err(e) => warn!("[DISCOVERY] Broadcast send failed: {}", e),
                },
                Err(e) => {
                    warn!("[DISCOVERY] Serialization error: {}", e);
                }
            }

            for raw_target in &announce_targets {
                let target_host_port = normalize_announce_target(raw_target, discovery_port);
                match tokio::net::lookup_host(&target_host_port).await {
                    Ok(addrs) => {
                        let mut sent_any = false;
                        for target_addr in addrs {
                            sent_any = true;
                            let effective_ip =
                                advertise_ip.or_else(|| detect_local_ip_for_target(target_addr));
                            let unicast_ann = build_signed_announcement(
                                &identity,
                                &node_id_hex,
                                quic_port,
                                timestamp,
                                effective_ip,
                            );
                            let Ok(payload) = serde_json::to_vec(&unicast_ann) else {
                                continue;
                            };
                            let send_res = if target_addr.is_ipv6() {
                                if let Some(ref s6) = socket_v6 {
                                    s6.send_to(&payload, target_addr).await
                                } else {
                                    warn!(
                                        "[DISCOVERY] Cannot send unicast to IPv6 {}: no IPv6 socket",
                                        target_addr
                                    );
                                    continue;
                                }
                            } else {
                                socket.send_to(&payload, target_addr).await
                            };
                            match send_res {
                                Ok(_) => info!(
                                    "[DISCOVERY] Write-in announced presence to {} (advertise_ip={:?})",
                                    target_addr, effective_ip
                                ),
                                Err(e) => warn!(
                                    "[DISCOVERY] Write-in send to {} failed: {}",
                                    target_addr, e
                                ),
                            }
                        }
                        if !sent_any {
                            warn!(
                                "[DISCOVERY] Write-in target '{}' resolved to no addresses",
                                target_host_port
                            );
                        }
                    }
                    Err(e) => {
                        warn!(
                            "[DISCOVERY] Failed to resolve write-in target '{}': {}",
                            target_host_port, e
                        );
                    }
                };
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

                    let msg_to_verify = build_announcement_signing_payload(
                        &node_id_bytes,
                        ann.quic_port,
                        ann.timestamp,
                        ann.advertise_ip,
                    );

                    if !verify_signature(&node_id_bytes, &msg_to_verify, &sig_bytes) {
                        warn!("[DISCOVERY] Rejected announcement from {}: invalid signature", peer_addr);
                        continue;
                    }

                    let peer_ip = ann
                        .advertise_ip
                        .filter(|ip| !ip.is_unspecified() && !ip.is_loopback() && !ip.is_multicast())
                        .unwrap_or_else(|| peer_addr.ip());
                    let quic_addr = SocketAddr::new(peer_ip, ann.quic_port);
                    info!("[DISCOVERY] Authenticated Peer found: {} at {}", ann.node_id, quic_addr);
                    registry.upsert(ann.node_id, quic_addr);
                }
                Err(e) => warn!("[DISCOVERY] Recv error: {}", e),
            }
        }
    });
}
