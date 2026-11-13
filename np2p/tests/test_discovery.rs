//! Discovery listener over a real UDP loopback: the listener validates
//! timestamps + signatures and upserts authenticated peers into the registry.
use np2p::crypto::NodeIdentity;
use np2p::network::discovery::{
    build_announcement_signing_payload, normalize_announce_target, start_broadcaster_with_targets,
    start_listener, DiscoveryAnnouncement,
};
use np2p::network::peer_registry::PeerRegistry;
use std::net::{IpAddr, SocketAddr, UdpSocket};
use std::sync::Arc;

async fn pick_free_port() -> u16 {
    // Bind a throwaway socket to let the OS choose a free port, then free it.
    tokio::net::UdpSocket::bind("127.0.0.1:0").await.unwrap().local_addr().unwrap().port()
}

fn signed_announcement(
    identity: &NodeIdentity,
    quic_port: u16,
    timestamp: u64,
    advertise_ip: Option<IpAddr>,
) -> Vec<u8> {
    let to_sign =
        build_announcement_signing_payload(&identity.node_id(), quic_port, timestamp, advertise_ip);
    let signature = hex::encode(identity.sign(&to_sign));

    let ann = DiscoveryAnnouncement {
        node_id: hex::encode(identity.node_id()),
        quic_port,
        timestamp,
        signature,
        advertise_ip,
    };
    serde_json::to_vec(&ann).unwrap()
}

#[tokio::test]
async fn listener_upserts_authenticated_peer() {
    let registry = PeerRegistry::new();
    let port = pick_free_port().await;
    let peer_id = NodeIdentity::generate();

    // The listener runs forever; canceled when the test's runtime drops.
    let reg = registry.clone();
    let peer_hex = hex::encode(peer_id.node_id());
    tokio::spawn(async move { start_listener(reg, port, "self-node".into()) });
    tokio::time::sleep(std::time::Duration::from_millis(150)).await;

    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH).unwrap().as_secs();
    let payload = signed_announcement(&peer_id, 9999, now, None);

    let sock = UdpSocket::bind("127.0.0.1:0").unwrap();
    sock.send_to(&payload, SocketAddr::from(([127, 0, 0, 1], port))).unwrap();
    tokio::time::sleep(std::time::Duration::from_millis(300)).await;

    let peer = registry.get(&peer_hex);
    assert!(peer.is_some(), "authenticated peer is registered");
    assert_eq!(peer.unwrap().addr.port(), 9999);
}

#[tokio::test]
async fn listener_ignores_expired_future_and_garbage() {
    let registry = PeerRegistry::new();
    let port = pick_free_port().await;
    let peer_id = NodeIdentity::generate();
    let peer_hex = hex::encode(peer_id.node_id());
    let reg = registry.clone();
    tokio::spawn(async move { start_listener(reg, port, "self-node".into()) });
    tokio::time::sleep(std::time::Duration::from_millis(150)).await;

    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH).unwrap().as_secs();
    let sock = UdpSocket::bind("127.0.0.1:0").unwrap();
    let dst = SocketAddr::from(([127, 0, 0, 1], port));

    // Expired (timestamp 90s+ old) -> ignored.
    sock.send_to(&signed_announcement(&peer_id, 8001, now - 120, None), dst).unwrap();
    // Future (timestamp > now+10) -> ignored.
    sock.send_to(&signed_announcement(&peer_id, 8002, now + 60, None), dst).unwrap();
    // Garbage bytes -> ignored (serde parse fails).
    sock.send_to(b"not a discovery announcement at all", dst).unwrap();
    // A valid one still lands.
    sock.send_to(&signed_announcement(&peer_id, 8003, now, None), dst).unwrap();

    tokio::time::sleep(std::time::Duration::from_millis(400)).await;
    let peer = registry.get(&peer_hex).expect("valid peer present");
    assert_eq!(peer.addr.port(), 8003);
}

#[tokio::test]
async fn listener_uses_signed_advertise_ip_and_rejects_tampered() {
    let registry = PeerRegistry::new();
    let port = pick_free_port().await;
    let peer_id = NodeIdentity::generate();
    let peer_hex = hex::encode(peer_id.node_id());
    let reg = registry.clone();
    tokio::spawn(async move { start_listener(reg, port, "self-node".into()) });
    tokio::time::sleep(std::time::Duration::from_millis(150)).await;

    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH).unwrap().as_secs();
    let sock = UdpSocket::bind("127.0.0.1:0").unwrap();
    let dst = SocketAddr::from(([127, 0, 0, 1], port));

    // Signed with Tailscale IP 100.100.1.2, sent from 127.0.0.1.
    let ts_ip: IpAddr = "100.100.1.2".parse().unwrap();
    let payload = signed_announcement(&peer_id, 5050, now, Some(ts_ip));
    sock.send_to(&payload, dst).unwrap();
    tokio::time::sleep(std::time::Duration::from_millis(300)).await;

    let peer = registry.get(&peer_hex).expect("peer registered via advertise_ip");
    assert_eq!(peer.addr, "100.100.1.2:5050".parse::<SocketAddr>().unwrap());

    // Tampered advertise_ip without re-signing should be rejected.
    let other_id = NodeIdentity::generate();
    let other_hex = hex::encode(other_id.node_id());
    let valid_payload = signed_announcement(&other_id, 5050, now, Some(ts_ip));
    let mut ann: DiscoveryAnnouncement = serde_json::from_slice(&valid_payload).unwrap();
    ann.advertise_ip = Some("100.100.1.99".parse().unwrap());
    let tampered = serde_json::to_vec(&ann).unwrap();
    sock.send_to(&tampered, dst).unwrap();
    tokio::time::sleep(std::time::Duration::from_millis(200)).await;
    assert!(registry.get(&other_hex).is_none(), "tampered advertise_ip must be rejected");
}

#[tokio::test]
async fn broadcaster_unicast_write_in_reaches_listener() {
    let registry = PeerRegistry::new();
    let port = pick_free_port().await;
    let peer_id = Arc::new(NodeIdentity::generate());
    let peer_hex = hex::encode(peer_id.node_id());
    let reg = registry.clone();
    tokio::spawn(async move { start_listener(reg, port, "self-node".into()) });
    tokio::time::sleep(std::time::Duration::from_millis(150)).await;

    let advertised: IpAddr = "100.88.77.66".parse().unwrap();
    start_broadcaster_with_targets(
        peer_id,
        5051,
        port,
        vec![format!("127.0.0.1:{}", port)],
        Some(advertised),
    );

    tokio::time::sleep(std::time::Duration::from_millis(400)).await;
    let peer = registry.get(&peer_hex).expect("unicast write-in registered peer");
    assert_eq!(peer.addr, "100.88.77.66:5051".parse::<SocketAddr>().unwrap());
}

#[test]
fn normalize_announce_target_appends_default_port_when_missing() {
    assert_eq!(normalize_announce_target("100.64.1.2", 5066), "100.64.1.2:5066");
    assert_eq!(normalize_announce_target("100.64.1.2:6000", 5066), "100.64.1.2:6000");
    assert_eq!(normalize_announce_target("home.tail123.ts.net", 5066), "home.tail123.ts.net:5066");
    assert_eq!(normalize_announce_target("home.tail123.ts.net:5067", 5066), "home.tail123.ts.net:5067");
    assert_eq!(normalize_announce_target("fd7a:115c:a1e0::1", 5066), "[fd7a:115c:a1e0::1]:5066");
    assert_eq!(normalize_announce_target("[fd7a:115c:a1e0::1]:5068", 5066), "[fd7a:115c:a1e0::1]:5068");
}
