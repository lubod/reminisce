//! Discovery listener over a real UDP loopback: the listener validates
//! timestamps + signatures and upserts authenticated peers into the registry.
use np2p::crypto::NodeIdentity;
use np2p::network::discovery::{ start_listener, DiscoveryAnnouncement };
use np2p::network::peer_registry::PeerRegistry;
use std::net::{ SocketAddr, UdpSocket };

async fn pick_free_port() -> u16 {
    // Bind a throwaway socket to let the OS choose a free port, then free it.
    tokio::net::UdpSocket::bind("127.0.0.1:0").await.unwrap().local_addr().unwrap().port()
}

fn signed_announcement(identity: &NodeIdentity, quic_port: u16, timestamp: u64) -> Vec<u8> {
    let mut to_sign = Vec::new();
    to_sign.extend_from_slice(&identity.node_id());
    to_sign.extend_from_slice(&quic_port.to_be_bytes());
    to_sign.extend_from_slice(&timestamp.to_be_bytes());
    let signature = hex::encode(identity.sign(&to_sign));

    let ann = DiscoveryAnnouncement {
        node_id: hex::encode(identity.node_id()),
        quic_port,
        timestamp,
        signature,
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
    let payload = signed_announcement(&peer_id, 9999, now);

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
    sock.send_to(&signed_announcement(&peer_id, 8001, now - 120), dst).unwrap();
    // Future (timestamp > now+10) -> ignored.
    sock.send_to(&signed_announcement(&peer_id, 8002, now + 60), dst).unwrap();
    // Garbage bytes -> ignored (serde parse fails).
    sock.send_to(b"not a discovery announcement at all", dst).unwrap();
    // A valid one still lands.
    sock.send_to(&signed_announcement(&peer_id, 8003, now), dst).unwrap();

    tokio::time::sleep(std::time::Duration::from_millis(400)).await;
    let peer = registry.get(&peer_hex).expect("valid peer present");
    assert_eq!(peer.addr.port(), 8003);
}
