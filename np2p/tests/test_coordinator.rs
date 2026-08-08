//! Fake-coordinator QUIC loopback harness: exercises the coordinator relay,
//! registration/peer-sync, the reverse connection channel, the HTTP tunnel, and
//! the P2PService facade against a scripted QUIC "coordinator" endpoint.
use np2p::crypto::{verify_signature, NodeIdentity};
use np2p::network::p2p_service::P2PService;
use np2p::network::{
    channel::start_channel_client, coordinator::{self, start_coordinator_client},
    tunnel::start_tunnel_client, Message, Node, Protocol,
};
use np2p::storage::DiskStorage;
use std::net::{SocketAddr, TcpListener};
use std::time::Duration;
use tempfile::tempdir;
use tokio::io::AsyncWriteExt;

async fn wait_until<F: Fn() -> bool>(f: F) {
    for _ in 0..120 {
        if f() {
            return;
        }
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
    panic!("condition not met within timeout");
}

#[derive(Clone)]
enum RelayReply {
    /// Wrap the message as RelayResponse (relay_message returns Ok(inner)).
    Wrapped(Message),
    /// Send the message at the top level (exercises Error/unexpected branches).
    Direct(Message),
}

/// Coordinator answering `RelayRequest`s. `Wrapped` replies arrive inside
/// RelayResponse; `Direct` replies are sent at the top level of the stream.
async fn spawn_relay_coordinator(reply: RelayReply) -> (String, SocketAddr) {
    let id = NodeIdentity::generate();
    let id_hex = hex::encode(id.node_id());
    let node = Node::new("127.0.0.1:0".parse().unwrap(), id).unwrap();
    let addr = node.local_addr().unwrap();
    tokio::spawn(async move {
        while let Some(incoming) = node.accept().await {
            if let Ok(conn) = incoming.await {
                let reply = reply.clone();
                tokio::spawn(async move {
                    if let Ok((mut send, mut recv)) = conn.accept_bi().await {
                        if let Ok(Message::RelayRequest { .. }) = Protocol::receive(&mut recv).await {
                            match reply {
                                RelayReply::Wrapped(msg) => {
                                    let payload = bincode::serialize(&msg).unwrap();
                                    let _ = Protocol::send(&mut send, &Message::RelayResponse { payload }).await;
                                }
                                RelayReply::Direct(msg) => {
                                    let _ = Protocol::send(&mut send, &msg).await;
                                }
                            }
                            // Keep the connection alive so the requester drains its read.
                            tokio::time::sleep(Duration::from_millis(200)).await;
                        }
                    }
                });
            }
        }
    });
    (id_hex, addr)
}

/// Coordinator handling `RegisterNode`/`GetPeers` and replying with a fixed peer list,
/// plus (optionally) an error response for a dedicated connection.
async fn spawn_peers_coordinator(
    register_peers: Vec<(String, String)>,
    getpeers_peers: Vec<(String, String)>,
) -> (String, SocketAddr) {
    let id = NodeIdentity::generate();
    let id_hex = hex::encode(id.node_id());
    let node = Node::new("127.0.0.1:0".parse().unwrap(), id).unwrap();
    let addr = node.local_addr().unwrap();
    tokio::spawn(async move {
        while let Some(incoming) = node.accept().await {
            if let Ok(conn) = incoming.await {
                let register_peers = register_peers.clone();
                let getpeers_peers = getpeers_peers.clone();
                tokio::spawn(async move {
                    if let Ok((mut send, mut recv)) = conn.accept_bi().await {
                        match Protocol::receive(&mut recv).await {
                            Ok(Message::RegisterNode { .. }) => {
                                let _ = Protocol::send(&mut send, &Message::PeerList { peers: register_peers }).await;
                            }
                            Ok(Message::GetPeers { .. }) => {
                                let _ = Protocol::send(&mut send, &Message::PeerList { peers: getpeers_peers }).await;
                            }
                            _ => {}
                        }
                        tokio::time::sleep(Duration::from_millis(200)).await;
                    }
                });
            }
        }
    });
    (id_hex, addr)
}

/// Coordinator that performs the reverse-channel challenge/response handshake, then
/// relays the given request messages downstream and forwards their responses out.
async fn spawn_channel_coordinator(
    relayed: Vec<Message>,
) -> (String, SocketAddr, tokio::sync::mpsc::UnboundedReceiver<Message>) {
    let (tx, rx) = tokio::sync::mpsc::unbounded_channel();
    let id = NodeIdentity::generate();
    let id_hex = hex::encode(id.node_id());
    let node = Node::new("127.0.0.1:0".parse().unwrap(), id).unwrap();
    let addr = node.local_addr().unwrap();
    tokio::spawn(async move {
        while let Some(incoming) = node.accept().await {
            if let Ok(conn) = incoming.await {
                let tx = tx.clone();
                let relayed = relayed.clone();
                tokio::spawn(async move {
                    // Step 1/2/3/4: registration, challenge, signature, acceptance.
                    let (mut send, mut recv) = match conn.accept_bi().await {
                        Ok(s) => s,
                        Err(_) => return,
                    };
                    let register = match Protocol::receive(&mut recv).await {
                        Ok(Message::NodeChannelRegister { node_id }) => node_id,
                        _ => return,
                    };
                    let nonce = vec![7u8; 32];
                    if Protocol::send(&mut send, &Message::NodeChannelChallenge { nonce: nonce.clone() }).await.is_err() {
                        return;
                    }
                    let response = match Protocol::receive(&mut recv).await {
                        Ok(Message::NodeChannelChallengeResponse { signature }) => signature,
                        _ => return,
                    };
                    let expected: [u8; 32] = match hex::decode(&register) {
                        Ok(b) if b.len() == 32 => b.try_into().unwrap(),
                        _ => return,
                    };
                    if !verify_signature(&expected, &nonce, &response) {
                        let _ = Protocol::send(&mut send, &Message::Error { code: 403, message: "bad sig".into() }).await;
                        return;
                    }
                    if Protocol::send(&mut send, &Message::NodeChannelAccepted).await.is_err() {
                        return;
                    }

                    // Relay requests downstream; forward each response back to the test.
                    for msg in relayed {
                        match conn.open_bi().await {
                            Ok((mut s, mut r)) => {
                                if Protocol::send(&mut s, &msg).await.is_err() {
                                    continue;
                                }
                                if let Ok(resp) = Protocol::receive(&mut r).await {
                                    let _ = tx.send(resp);
                                }
                            }
                            Err(_) => break,
                        }
                    }
                });
            }
        }
    });
    (id_hex, addr, rx)
}

/// Coordinator performing the tunnel challenge/response, then piping one byte
/// buffer through the tunnel's local hop and back.
async fn spawn_tunnel_coordinator(
    request: Vec<u8>,
) -> (String, SocketAddr, tokio::sync::mpsc::UnboundedReceiver<Vec<u8>>) {
    let (tx, rx) = tokio::sync::mpsc::unbounded_channel();
    let request_len = request.len();
    let id = NodeIdentity::generate();
    let id_hex = hex::encode(id.node_id());
    let node = Node::new("127.0.0.1:0".parse().unwrap(), id).unwrap();
    let addr = node.local_addr().unwrap();
    tokio::spawn(async move {
        while let Some(incoming) = node.accept().await {
            if let Ok(conn) = incoming.await {
                let tx = tx.clone();
                let request = request.clone();
                tokio::spawn(async move {
                    let (mut send, mut recv) = match conn.accept_bi().await {
                        Ok(s) => s,
                        Err(_) => return,
                    };
                    if !matches!(Protocol::receive(&mut recv).await, Ok(Message::TunnelRegister { .. })) {
                        return;
                    }
                    let nonce = vec![9u8; 32];
                    if Protocol::send(&mut send, &Message::TunnelChallenge { nonce: nonce.clone() }).await.is_err() {
                        return;
                    }
                    match Protocol::receive(&mut recv).await {
                        Ok(Message::TunnelChallengeResponse { .. }) => {}
                        _ => return,
                    }
                    if Protocol::send(&mut send, &Message::TunnelAccepted).await.is_err() {
                        return;
                    }
                    let (mut s, mut r) = match conn.open_bi().await {
                        Ok(s) => s,
                        Err(_) => return,
                    };
                    if s.write_all(&request).await.is_err() {
                        return;
                    }
                    let mut buf = vec![0u8; request_len];
                    if r.read_exact(&mut buf).await.is_err() {
                        return;
                    }
                    let _ = tx.send(buf);
                });
            }
        }
    });
    (id_hex, addr, rx)
}

/// Small TCP echo server for tunnel local-hop tests.
async fn spawn_tcp_echo_server() -> (u16, SocketAddr) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    listener.set_nonblocking(true).unwrap();
    let addr = listener.local_addr().unwrap();
    let listener = tokio::net::TcpListener::from_std(listener).unwrap();
    tokio::spawn(async move {
        loop {
            let Ok((mut sock, _)) = listener.accept().await else { continue };
            tokio::spawn(async move {
                let mut buf = vec![0u8; 8192];
                loop {
                    use tokio::io::AsyncReadExt;
                    let n = match sock.read(&mut buf).await {
                        Ok(0) | Err(_) => break,
                        Ok(n) => n,
                    };
                    if sock.write_all(&buf[..n]).await.is_err() {
                        break;
                    }
                }
            });
        }
    });
    (addr.port(), addr)
}

#[tokio::test]
async fn relay_message_roundtrips_through_coordinator() {
    let (coord_id, coord_addr) = spawn_relay_coordinator(RelayReply::Wrapped(Message::HandshakeAck { node_id: [1u8; 32] })).await;
    let client = NodeIdentity::generate();
    let node = Node::new("127.0.0.1:0".parse().unwrap(), client).unwrap();

    let msg = Message::Handshake { node_id: [2u8; 32], version: np2p::PROTOCOL_VERSION.into() };
    let resp = coordinator::relay_message(coord_addr, &coord_id, &node, "target-node", &msg)
        .await
        .expect("relay succeeds");
    assert!(matches!(resp, Message::HandshakeAck { .. }), "relayed response: {:?}", resp);
}

#[tokio::test]
async fn relay_message_surfaces_coordinator_error() {
    let (coord_id, coord_addr) =
        spawn_relay_coordinator(RelayReply::Direct(Message::Error { code: 404, message: "not found".into() })).await;
    let node = Node::new("127.0.0.1:0".parse().unwrap(), NodeIdentity::generate()).unwrap();

    let err = coordinator::relay_message(coord_addr, &coord_id, &node, "t", &Message::GetPeers { namespace: "n".into() })
        .await
        .expect_err("relay error path");
    assert!(err.to_string().contains("Relay error 404"), "err: {err}");
}

#[tokio::test]
async fn relay_message_rejects_unexpected_payload() {
    let (coord_id, coord_addr) =
        spawn_relay_coordinator(RelayReply::Direct(Message::PeerList { peers: vec![] })).await;
    let node = Node::new("127.0.0.1:0".parse().unwrap(), NodeIdentity::generate()).unwrap();

    let err = coordinator::relay_message(coord_addr, &coord_id, &node, "t", &Message::GetPeers { namespace: "n".into() })
        .await
        .expect_err("unexpected payload err");
    assert!(err.to_string().contains("Unexpected relay response"), "err: {err}");
}

#[tokio::test]
async fn coordinator_client_registers_and_merges_peers() {
    let self_id = NodeIdentity::generate();
    let self_hex = hex::encode(self_id.node_id());
    let peer_a_hex = hex::encode(NodeIdentity::generate().node_id());
    let peer_c_hex = hex::encode(NodeIdentity::generate().node_id());

    let (coord_id, coord_addr) = spawn_peers_coordinator(
        vec![
            (self_hex.clone(), "127.0.0.1:1".into()),  // self: must be skipped
            (peer_a_hex.clone(), "127.0.0.1:9999".into()),
            ("deadbeef".into(), "not-an-addr".into()),  // unparseable: warning branch
        ],
        vec![(peer_c_hex.clone(), "127.0.0.1:8888".into())],
    )
    .await;

    // quic_port = Some -> RegisterNode, then merge the returned peers.
    let registry = np2p::network::PeerRegistry::new();
    start_coordinator_client(
        coord_addr,
        &coord_id,
        Node::new("127.0.0.1:0".parse().unwrap(), self_id).unwrap(),
        self_hex.clone(),
        Some(9001),
        registry.clone(),
        "default".to_string(),
    );
    wait_until(|| registry.get(&peer_a_hex).is_some()).await;
    assert!(registry.get(&self_hex).is_none(), "self must not be merged");

    // quic_port = None -> GetPeers.
    let registry2 = np2p::network::PeerRegistry::new();
    start_coordinator_client(
        coord_addr,
        &coord_id,
        Node::new("127.0.0.1:0".parse().unwrap(), NodeIdentity::generate()).unwrap(),
        "other-node".to_string(),
        None,
        registry2.clone(),
        "default".to_string(),
    );
    wait_until(|| registry2.get(&peer_c_hex).is_some()).await;
}

#[tokio::test]
async fn channel_registers_relays_requests_and_validates_tokens() {
    let tmp = tempdir().unwrap();
    let storage = DiskStorage::new(tmp.path()).await.unwrap();
    let channel_identity = NodeIdentity::generate();
    let name = "relayed-name".to_string();
    let wrong_hash: [u8; 32] = blake3::hash(b"attacker").into();
    let wrong_token = channel_identity.create_shard_token(np2p::crypto::ShardOp::Retrieve, &wrong_hash);

    let (coord_id, coord_addr, mut rx) = spawn_channel_coordinator(vec![
        Message::Handshake {
            node_id: channel_identity.node_id(),
            version: np2p::PROTOCOL_VERSION.into(),
        },
        Message::GetPinnedObject { name: name.clone(), token: wrong_token },
    ])
    .await;

    start_channel_client(
        coord_addr,
        &coord_id,
        Node::new("127.0.0.1:0".parse().unwrap(), NodeIdentity::generate()).unwrap(),
        channel_identity.clone(),
        storage,
        None,
    );

    // Relayed handshake -> HandshakeAck.
    let first = rx.recv().await.expect("relayed handshake ack");
    assert!(matches!(first, Message::HandshakeAck { .. }), "got {:?}", first);

    // Relayed GetPinnedObject with a spoofed token -> 401.
    let second = rx.recv().await.expect("relayed 401");
    match second {
        Message::Error { code, .. } => assert_eq!(code, 401, "unauthorized relayed get -> 401"),
        other => panic!("expected Error(401), got {:?}", other),
    }
}

#[tokio::test]
async fn channel_survives_coordinator_rejection() {
    // Coordinator answers the registration with Error instead of Accepted: the
    // client logs "Coordinator rejected channel" and schedules a reconnect.
    let (tx, _rx) = tokio::sync::mpsc::unbounded_channel::<Message>();
    let tmp = tempdir().unwrap();
    let storage = DiskStorage::new(tmp.path()).await.unwrap();
    let id = NodeIdentity::generate();
    let id_hex = hex::encode(id.node_id());
    let node = Node::new("127.0.0.1:0".parse().unwrap(), id).unwrap();
    let addr = node.local_addr().unwrap();
    tokio::spawn(async move {
        while let Some(incoming) = node.accept().await {
            if let Ok(conn) = incoming.await {
                let _tx = tx.clone();
                tokio::spawn(async move {
                    if let Ok((mut send, mut recv)) = conn.accept_bi().await {
                        if let Ok(Message::NodeChannelRegister { .. }) = Protocol::receive(&mut recv).await {
                            let nonce = vec![1u8; 32];
                            let _ = Protocol::send(&mut send, &Message::NodeChannelChallenge { nonce: nonce.clone() }).await;
                            if let Ok(Message::NodeChannelChallengeResponse { .. }) = Protocol::receive(&mut recv).await {
                                let _ = Protocol::send(&mut send, &Message::Error { code: 403, message: "denied".into() }).await;
                            }
                        }
                    }
                });
            }
        }
    });

    start_channel_client(addr, &id_hex, Node::new("127.0.0.1:0".parse().unwrap(), NodeIdentity::generate()).unwrap(), NodeIdentity::generate(), storage, None);
    tokio::time::sleep(Duration::from_millis(500)).await;
}

#[tokio::test]
async fn channel_tolerates_unknown_protocol_message() {
    // The coordinator sends raw bytes that decode to an unknown message variant:
    // the channel client logs the protocol-version mismatch and retries in 60s.
    let tmp = tempdir().unwrap();
    let storage = DiskStorage::new(tmp.path()).await.unwrap();
    let id = NodeIdentity::generate();
    let id_hex = hex::encode(id.node_id());
    let node = Node::new("127.0.0.1:0".parse().unwrap(), id).unwrap();
    let addr = node.local_addr().unwrap();
    tokio::spawn(async move {
        while let Some(incoming) = node.accept().await {
            if let Ok(conn) = incoming.await {
                tokio::spawn(async move {
                    // bincode enum variant index 255 as a varint, then payload bytes.
                    let garbage = vec![0xFF, 0x01, 0x00, 0x00, 0x00, 0x00];
                    if let Ok((mut send, _recv)) = conn.accept_bi().await {
                        let len = (garbage.len() as u32).to_be_bytes();
                        let _ = send.write_all(&len).await;
                        let _ = send.write_all(&garbage).await;
                    }
                });
            }
        }
    });

    start_channel_client(addr, &id_hex, Node::new("127.0.0.1:0".parse().unwrap(), NodeIdentity::generate()).unwrap(), NodeIdentity::generate(), storage, None);
    tokio::time::sleep(Duration::from_millis(500)).await;
}

#[tokio::test]
async fn tunnel_pipes_through_local_http_server() {
    let (local_port, _server_addr) = spawn_tcp_echo_server().await;
    let payload = b"GET /api/status HTTP/1.1\r\nHost: localhost\r\n\r\n".to_vec();

    let (coord_id, coord_addr, mut rx) = spawn_tunnel_coordinator(payload.clone()).await;

    start_tunnel_client(
        coord_addr,
        &coord_id,
        Node::new("127.0.0.1:0".parse().unwrap(), NodeIdentity::generate()).unwrap(),
        NodeIdentity::generate(),
        local_port,
    );

    let echoed = tokio::time::timeout(Duration::from_secs(15), rx.recv())
        .await
        .expect("echo within timeout")
        .expect("echo channel delivered");
    assert_eq!(echoed, payload, "tunnel rounds bytes through the local server");
}

#[tokio::test]
async fn tunnel_handles_rejection_and_unreachable_local() {
    // Coordinator rejects the tunnel registration with an Error, and a second
    // coordinator opens a stream while nothing listens locally. Both run within
    // one window that covers the tunnel client's 3s pre-registration delay.
    let tmp_id = NodeIdentity::generate();
    let tmp_hex = hex::encode(tmp_id.node_id());
    let reject_node = Node::new("127.0.0.1:0".parse().unwrap(), tmp_id).unwrap();
    let reject_addr = reject_node.local_addr().unwrap();
    tokio::spawn(async move {
        while let Some(incoming) = reject_node.accept().await {
            if let Ok(conn) = incoming.await {
                tokio::spawn(async move {
                    if let Ok((mut send, mut recv)) = conn.accept_bi().await {
                        if let Ok(Message::TunnelRegister { .. }) = Protocol::receive(&mut recv).await {
                            let nonce = vec![3u8; 32];
                            let _ = Protocol::send(&mut send, &Message::TunnelChallenge { nonce: nonce.clone() }).await;
                            if let Ok(Message::TunnelChallengeResponse { .. }) = Protocol::receive(&mut recv).await {
                                let _ = Protocol::send(&mut send, &Message::Error { code: 409, message: "conflict".into() }).await;
                            }
                        }
                    }
                });
            }
        }
    });

    let ok_id = NodeIdentity::generate();
    let ok_hex = hex::encode(ok_id.node_id());
    let ok_node = Node::new("127.0.0.1:0".parse().unwrap(), ok_id).unwrap();
    let ok_addr = ok_node.local_addr().unwrap();
    tokio::spawn(async move {
        while let Some(incoming) = ok_node.accept().await {
            if let Ok(conn) = incoming.await {
                tokio::spawn(async move {
                    let (mut send, mut recv) = match conn.accept_bi().await { Ok(s) => s, Err(_) => return };
                    if !matches!(Protocol::receive(&mut recv).await, Ok(Message::TunnelRegister { .. })) { return; }
                    let nonce = vec![4u8; 32];
                    if Protocol::send(&mut send, &Message::TunnelChallenge { nonce: nonce.clone() }).await.is_err() { return; }
                    match Protocol::receive(&mut recv).await {
                        Ok(Message::TunnelChallengeResponse { .. }) => {}
                        _ => return,
                    }
                    if Protocol::send(&mut send, &Message::TunnelAccepted).await.is_err() { return; }
                    if let Ok((mut s, mut _r)) = conn.open_bi().await {
                        let _ = s.write_all(b"request to nowhere").await;
                    }
                });
            }
        }
    });

    // No local listener on these ports: both tunnel clients exercise the
    // rejection branch and the pipe-to-local connect-failure branch.
    let dead_port = 1u16;
    start_tunnel_client(reject_addr, &tmp_hex, Node::new("127.0.0.1:0".parse().unwrap(), NodeIdentity::generate()).unwrap(), NodeIdentity::generate(), dead_port);
    start_tunnel_client(ok_addr, &ok_hex, Node::new("127.0.0.1:0".parse().unwrap(), NodeIdentity::generate()).unwrap(), NodeIdentity::generate(), dead_port);
    tokio::time::sleep(Duration::from_secs(5)).await;
}

#[tokio::test]
async fn p2p_service_requires_registry_for_direct_connects() {
    let mut service = P2PService::new("127.0.0.1:0".parse().unwrap(), NodeIdentity::generate())
        .await
        .expect("binds");

    let err = service.connect_to_addr("127.0.0.1:1".parse().unwrap()).await.unwrap_err();
    assert!(err.to_string().contains("not in registry"), "err: {err}");

    let err = service.connect_to_peer("deadbeef").await.unwrap_err();
    assert!(err.to_string().contains("Peer not in registry"), "err: {err}");

    service.set_coordinator("127.0.0.1:2".parse().unwrap(), hex::encode(NodeIdentity::generate().node_id()));
    assert!(service.coordinator_addr.is_some());

    let _ = service.node().local_addr().unwrap();
    let _ = service.identity().node_id().len();
}

#[tokio::test]
async fn p2p_service_sends_direct_then_relay_then_falls_back() {
    // 1. Direct: peer known + reachable (a listening fake node).
    let server_id = NodeIdentity::generate();
    let server_hex = hex::encode(server_id.node_id());
    let server_node = Node::new("127.0.0.1:0".parse().unwrap(), server_id).unwrap();
    let server_addr = server_node.local_addr().unwrap();
    tokio::spawn(async move {
        while let Some(incoming) = server_node.accept().await {
            if let Ok(conn) = incoming.await {
                tokio::spawn(async move {
                    if let Ok((mut send, mut recv)) = conn.accept_bi().await {
                        if let Ok(Message::Handshake { node_id, .. }) = Protocol::receive(&mut recv).await {
                            let _ = Protocol::send(&mut send, &Message::HandshakeAck { node_id }).await;
                        }
                        tokio::time::sleep(Duration::from_millis(200)).await;
                    }
                });
            }
        }
    });

    // 2. Relay coordinator.
    let (coord_id, coord_addr) = spawn_relay_coordinator(RelayReply::Wrapped(Message::HandshakeAck { node_id: [5u8; 32] })).await;

    let mut client = P2PService::new("127.0.0.1:0".parse().unwrap(), NodeIdentity::generate()).await.unwrap();
    client.registry.upsert(server_hex.clone(), server_addr);
    let msg = Message::Handshake { node_id: [6u8; 32], version: np2p::PROTOCOL_VERSION.into() };

    // Direct path succeeds.
    let resp = client.send_message(&server_hex, &msg).await.expect("direct send");
    assert!(matches!(resp, Message::HandshakeAck { .. }));

    // Without a coordinator, an unknown peer is an error.
    let err = client.send_message("nosuchnode", &msg).await.unwrap_err();
    assert!(err.to_string().contains("no coordinator"), "err: {err}");

    // Coordinator configured + unreachable registry entry -> direct fails, relay succeeds.
    client.set_coordinator(coord_addr, coord_id.clone());
    client.registry.upsert("ghost-peer".into(), "127.0.0.1:1".parse().unwrap());
    let resp = client.send_message("ghost-peer", &msg).await.expect("relay fallback");
    assert!(matches!(resp, Message::HandshakeAck { .. }));

    // Unknown peer with a coordinator -> relay directly (no known address).
    let resp = client.send_message("relay-only-peer", &msg).await.expect("relay-only");
    assert!(matches!(resp, Message::HandshakeAck { .. }));
}