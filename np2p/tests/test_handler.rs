//! ConnectionHandler over a real QUIC loopback connection: covers the
//! store/get pinned-object paths, authentication failures, and the handshake.
use np2p::crypto::NodeIdentity;
use np2p::network::{ Node, Message, Protocol, ConnectionHandler };
use np2p::storage::DiskStorage;
use std::sync::Arc;
use tempfile::tempdir;

async fn spawn_server(
    storage: DiskStorage,
    identity: Arc<NodeIdentity>,
    allowed_owner: Option<[u8; 32]>,
) -> (String, std::net::SocketAddr) {
    let server_id = NodeIdentity::generate();
    let server_id_hex = hex::encode(server_id.node_id());
    let node = Node::new("127.0.0.1:0".parse().unwrap(), server_id).unwrap();
    let addr = node.local_addr().unwrap();
    tokio::spawn(async move {
        while let Some(incoming) = node.accept().await {
            let storage = storage.clone();
            let identity = identity.clone();
            let owner = allowed_owner;
            tokio::spawn(async move {
                if let Ok(conn) = incoming.await {
                    let mut handler = ConnectionHandler::new(conn, storage, identity);
                    if let Some(owner) = owner {
                        handler = handler.with_allowed_owner(Some(owner));
                    }
                    handler.run().await;
                }
            });
        }
    });
    (server_id_hex, addr)
}

#[tokio::test]
async fn store_and_get_pinned_object() {
    let tmp = tempdir().unwrap();
    let server_storage = DiskStorage::new(tmp.path()).await.unwrap();
    let server_identity = Arc::new(NodeIdentity::generate());
    let (server_id_hex, addr) = spawn_server(server_storage, server_identity, None).await;

    let client_id = NodeIdentity::generate();
    let client_node = Node::new("127.0.0.1:0".parse().unwrap(), client_id.clone()).unwrap();
    let conn = client_node.connect(addr, &server_id_hex).await.expect("connect");

    let name = "latest-manifest".to_string();
    let name_hash: [u8; 32] = blake3::hash(name.as_bytes()).into();
    let token = client_id.create_shard_token(&name_hash);
    let data = b"hello pinned object".to_vec();

    {
        let (mut send, mut recv) = conn.open_bi().await.unwrap();
        Protocol::send(&mut send, &Message::StorePinnedObject { name: name.clone(), data: data.clone(), token: token.clone() }).await.unwrap();
        let resp = Protocol::receive(&mut recv).await.unwrap();
        if let Message::StorePinnedResponse { success } = resp {
            assert!(success, "pinned store succeeded");
        } else { panic!("expected StorePinnedResponse, got {:?}", resp); }
    }

    {
        let (mut send, mut recv) = conn.open_bi().await.unwrap();
        Protocol::send(&mut send, &Message::GetPinnedObject { name: name.clone(), token }).await.unwrap();
        let resp = Protocol::receive(&mut recv).await.unwrap();
        if let Message::PinnedObjectResponse { data } = resp {
            assert_eq!(data.as_deref(), Some(b"hello pinned object".as_slice()), "retrieved pinned data matches");
        } else { panic!("expected PinnedObjectResponse, got {:?}", resp); }
    }
}

#[tokio::test]
async fn unauthorized_pinned_access_rejected() {
    let tmp = tempdir().unwrap();
    let server_storage = DiskStorage::new(tmp.path()).await.unwrap();
    let server_identity = Arc::new(NodeIdentity::generate());
    let (server_id_hex, addr) = spawn_server(server_storage, server_identity, None).await;

    let client_id = NodeIdentity::generate();
    let client_node = Node::new("127.0.0.1:0".parse().unwrap(), client_id.clone()).unwrap();
    let conn = client_node.connect(addr, &server_id_hex).await.expect("connect");

    let name = "sec manifest".to_string();
    let wrong_hash: [u8; 32] = blake3::hash(b"spoofed").into();
    let token = client_id.create_shard_token(&wrong_hash);

    let (mut send, mut recv) = conn.open_bi().await.unwrap();
    Protocol::send(&mut send, &Message::StorePinnedObject { name: name.clone(), data: b"x".to_vec(), token: token.clone() }).await.unwrap();
    let resp = Protocol::receive(&mut recv).await.unwrap();
    if let Message::StorePinnedResponse { success } = resp {
        assert!(!success, "wrong-hash token must be rejected");
    } else { panic!("expected StorePinnedResponse, got {:?}", resp); }

    let (mut send, mut recv) = conn.open_bi().await.unwrap();
    Protocol::send(&mut send, &Message::GetPinnedObject { name, token }).await.unwrap();
    let resp = Protocol::receive(&mut recv).await.unwrap();
    if let Message::Error { code, .. } = resp {
        assert_eq!(code, 401, "unauthorized get -> 401");
    } else { panic!("expected Error(401), got {:?}", resp); }
}

#[tokio::test]
async fn handshake_is_acknowledged() {
    let tmp = tempdir().unwrap();
    let server_storage = DiskStorage::new(tmp.path()).await.unwrap();
    let server_identity = Arc::new(NodeIdentity::generate());
    let (server_id_hex, addr) = spawn_server(server_storage, server_identity, None).await;

    let client_id = NodeIdentity::generate();
    let client_node = Node::new("127.0.0.1:0".parse().unwrap(), client_id.clone()).unwrap();
    let conn = client_node.connect(addr, &server_id_hex).await.expect("connect");

    let (mut send, mut recv) = conn.open_bi().await.unwrap();
    Protocol::send(&mut send, &Message::Handshake { node_id: client_id.node_id(), version: np2p::PROTOCOL_VERSION.into() }).await.unwrap();
    let resp = Protocol::receive(&mut recv).await.unwrap();
    match resp {
        Message::HandshakeAck { .. } => {}
        Message::Error { code, .. } => panic!("handshake error {code}"),
        other => panic!("expected HandshakeAck, got {:?}", other),
    }
}
