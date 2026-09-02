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
    let store_token = client_id.create_shard_token(np2p::crypto::ShardOp::Store, &name_hash);
    let get_token = client_id.create_shard_token(np2p::crypto::ShardOp::Retrieve, &name_hash);
    let data = b"hello pinned object".to_vec();

    {
        let (mut send, mut recv) = conn.open_bi().await.unwrap();
        Protocol::send(&mut send, &Message::StorePinnedObject { name: name.clone(), data: data.clone(), token: store_token }).await.unwrap();
        let resp = Protocol::receive(&mut recv).await.unwrap();
        if let Message::StorePinnedResponse { success } = resp {
            assert!(success, "pinned store succeeded");
        } else { panic!("expected StorePinnedResponse, got {:?}", resp); }
    }

    {
        // A *store* token must NOT authorize retrieving: only a Retrieve token can.
        let (mut send, mut recv) = conn.open_bi().await.unwrap();
        Protocol::send(&mut send, &Message::GetPinnedObject { name: name.clone(), token: get_token }).await.unwrap();
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
    let token = client_id.create_shard_token(np2p::crypto::ShardOp::Store, &wrong_hash);

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

#[tokio::test]
async fn list_and_delete_shards() {

    let tmp = tempdir().unwrap();
    let server_storage = DiskStorage::new(tmp.path()).await.unwrap();

    // Store 2 test shards directly
    let shard1: [u8; 32] = [0xabu8; 32];
    let shard2: [u8; 32] = [0xacu8; 32];
    server_storage.store(shard1, b"shard data 1").await.unwrap();
    server_storage.store(shard2, b"shard data 2").await.unwrap();

    let server_identity = Arc::new(NodeIdentity::generate());
    let (server_id_hex, addr) = spawn_server(server_storage.clone(), server_identity, None).await;

    let client_id = NodeIdentity::generate();
    let client_node = Node::new("127.0.0.1:0".parse().unwrap(), client_id.clone()).unwrap();
    let conn = client_node.connect(addr, &server_id_hex).await.expect("connect");

    // 1. List shards (all)
    let list_scope: [u8; 32] = blake3::hash(b"").into();
    let list_token = client_id.create_shard_token(np2p::crypto::ShardOp::List, &list_scope);
    {
        let (mut send, mut recv) = conn.open_bi().await.unwrap();
        Protocol::send(&mut send, &Message::ListShardsRequest { prefix: None, token: list_token }).await.unwrap();
        let resp = Protocol::receive(&mut recv).await.unwrap();
        if let Message::ListShardsResponse { shards, .. } = resp {
            assert_eq!(shards.len(), 2);
            assert!(shards.contains(&shard1));
            assert!(shards.contains(&shard2));
        } else { panic!("expected ListShardsResponse, got {:?}", resp); }
    }

    // 2. Delete shard1
    let del_token = client_id.create_shard_token(np2p::crypto::ShardOp::Delete, &shard1);
    {
        let (mut send, mut recv) = conn.open_bi().await.unwrap();
        Protocol::send(&mut send, &Message::DeleteShardRequest { shard_hash: shard1, token: del_token }).await.unwrap();
        let resp = Protocol::receive(&mut recv).await.unwrap();
        if let Message::DeleteShardResponse { success, .. } = resp {
            assert!(success, "delete succeeded");
        } else { panic!("expected DeleteShardResponse, got {:?}", resp); }
    }

    // 3. Verify shard1 is gone, shard2 remains
    assert!(!server_storage.exists(shard1));
    assert!(server_storage.exists(shard2));
}

#[tokio::test]
async fn oversize_stream_init_rejected_with_413() {
    let tmp = tempdir().unwrap();
    let server_storage = DiskStorage::new(tmp.path()).await.unwrap();
    let server_identity = Arc::new(NodeIdentity::generate());
    let (server_id_hex, addr) = spawn_server(server_storage, server_identity, None).await;

    let client_id = NodeIdentity::generate();
    let client_node = Node::new("127.0.0.1:0".parse().unwrap(), client_id.clone()).unwrap();
    let conn = client_node.connect(addr, &server_id_hex).await.expect("connect");

    let file_hash: [u8; 32] = [7u8; 32];
    let shard_index: u8 = 0;
    // Token is valid on purpose: the rejection must come from the SIZE cap, not from
    // authentication. Such a shard would otherwise store fine yet never be retrievable
    // (retrieval ships one frame capped at MAX_MESSAGE_LEN).
    let binding: [u8; 32] = blake3::hash(&[file_hash.as_slice(), &[shard_index]].concat()).into();
    let token = client_id.create_shard_token(np2p::crypto::ShardOp::Store, &binding);
    let oversize: u64 = (np2p::network::protocol::MAX_MESSAGE_LEN as u64) - 512;

    let (mut send, mut recv) = conn.open_bi().await.unwrap();
    Protocol::send(&mut send, &Message::StoreShardStreamInit {
        file_hash,
        shard_index,
        total_shard_bytes: oversize,
        segment_count: 1,
        token,
    }).await.unwrap();

    match Protocol::receive(&mut recv).await.unwrap() {
        Message::Error { code, .. } => assert_eq!(code, 413, "oversize init refused at store time"),
        other => panic!("expected Error(413), got {:?}", other),
    }
}

// Paused time: once the client goes fully silent, the runtime fast-forwards to the
// server's next timer deadline — the 120s read deadline fires instantly in virtual
// time instead of burning real seconds.
#[tokio::test(start_paused = true)]
async fn silent_stream_dropped_without_leaking_permit() {
    let tmp = tempdir().unwrap();
    let server_storage = DiskStorage::new(tmp.path()).await.unwrap();
    let server_identity = Arc::new(NodeIdentity::generate());
    let (server_id_hex, addr) = spawn_server(server_storage, server_identity, None).await;

    let client_id = NodeIdentity::generate();
    let client_node = Node::new("127.0.0.1:0".parse().unwrap(), client_id.clone()).unwrap();
    let conn = client_node.connect(addr, &server_id_hex).await.expect("connect");

    // Open a stream and write an INCOMPLETE protocol frame (QUIC only transmits a
    // stream once data exists, and a totally silent stream is never even accepted):
    // the handler starts, takes a semaphore permit, then blocks on the first message.
    // The read deadline must abort it (releasing the permit) instead of pinning it.
    let (mut send, mut recv) = conn.open_bi().await.unwrap();
    send.write_all(&[0x00, 0x00]).await.expect("write partial frame");
    tokio::time::timeout(std::time::Duration::from_secs(600), async move {
        loop {
            match recv.read(&mut [0u8; 64]).await {
                Ok(Some(0)) | Ok(None) => break,
                Err(_) => break,
                Ok(Some(_)) => {}
            }
        }
    }).await.expect("server never dropped the silent stream");

    // The permit came back: a fresh stream is still served normally.
    let (mut send, mut recv) = conn.open_bi().await.unwrap();
    Protocol::send(&mut send, &Message::Handshake {
        node_id: client_id.node_id(),
        version: np2p::PROTOCOL_VERSION.into(),
    }).await.unwrap();
    match Protocol::receive(&mut recv).await.unwrap() {
        Message::HandshakeAck { .. } => {}
        other => panic!("expected HandshakeAck after deadline drop, got {:?}", other),
    }
}

#[tokio::test]
async fn retrieve_shard_stream_success_and_not_found() {
    let tmp = tempdir().unwrap();
    let server_storage = DiskStorage::new(tmp.path()).await.unwrap();

    let shard_data = vec![0xA5u8; 1024 * 512]; // 512 KB test shard
    let shard_hash: [u8; 32] = blake3::hash(&shard_data).into();
    server_storage.store(shard_hash, &shard_data).await.unwrap();

    let server_identity = Arc::new(NodeIdentity::generate());
    let (server_id_hex, addr) = spawn_server(server_storage, server_identity, None).await;

    let client_id = NodeIdentity::generate();
    let client_node = Node::new("127.0.0.1:0".parse().unwrap(), client_id.clone()).unwrap();
    let conn = client_node.connect(addr, &server_id_hex).await.expect("connect");

    // 1. Success case: retrieve existing shard via stream
    {
        let (mut send, mut recv) = conn.open_bi().await.unwrap();
        let token = client_id.create_shard_token(np2p::crypto::ShardOp::Retrieve, &shard_hash);
        Protocol::send(&mut send, &Message::RetrieveShardStreamInit { shard_hash, token }).await.unwrap();

        match Protocol::receive(&mut recv).await.unwrap() {
            Message::RetrieveShardStreamAck { found: true, total_bytes } => {
                assert_eq!(total_bytes, shard_data.len() as u64);
            }
            other => panic!("expected RetrieveShardStreamAck found=true, got {:?}", other),
        }

        let mut streamed_bytes = Vec::new();
        loop {
            match Protocol::receive(&mut recv).await.unwrap() {
                Message::RetrieveShardChunk { data } => {
                    streamed_bytes.extend_from_slice(&data);
                }
                Message::RetrieveShardStreamFinal { shard_hash: final_hash } => {
                    assert_eq!(final_hash, shard_hash);
                    break;
                }
                other => panic!("unexpected message: {:?}", other),
            }
        }
        assert_eq!(streamed_bytes, shard_data);
    }

    // 2. Not found case: retrieve nonexistent shard via stream
    {
        let (mut send, mut recv) = conn.open_bi().await.unwrap();
        let missing_hash = [0xEEu8; 32];
        let token = client_id.create_shard_token(np2p::crypto::ShardOp::Retrieve, &missing_hash);
        Protocol::send(&mut send, &Message::RetrieveShardStreamInit { shard_hash: missing_hash, token }).await.unwrap();

        match Protocol::receive(&mut recv).await.unwrap() {
            Message::RetrieveShardStreamAck { found: false, total_bytes: 0 } => {}
            other => panic!("expected RetrieveShardStreamAck found=false, got {:?}", other),
        }
    }
}
