use np2p::crypto::NodeIdentity;
use np2p::network::{Node, Message, Protocol, ConnectionHandler};
use np2p::storage::{StorageEngine, DiskStorage};
use std::sync::Arc;
use tempfile::tempdir;

#[tokio::test]
async fn test_e2e_backup_and_restore() {
    // 1. Setup Server (Storage Node)
    let server_id = NodeIdentity::generate();
    let server_node = Node::new("127.0.0.1:0".parse().unwrap(), server_id).unwrap();
    let server_addr = server_node.local_addr().unwrap();
    
    let server_tmp = tempdir().unwrap();
    let server_storage = DiskStorage::new(server_tmp.path()).await.unwrap();
    let server_identity = Arc::new(NodeIdentity::generate());

    tokio::spawn(async move {
        while let Some(incoming) = server_node.accept().await {
            let storage = server_storage.clone();
            let identity = server_identity.clone();
            tokio::spawn(async move {
                if let Ok(conn) = incoming.await {
                    let handler = ConnectionHandler::new(conn, storage, identity);
                    handler.run().await;
                }
            });
        }
    });

    // 2. Setup Client (Home Server)
    let client_id = NodeIdentity::generate();
    let client_node = Node::new("127.0.0.1:0".parse().unwrap(), client_id).unwrap();
    
    // Original data to backup
    let original_data = b"E2E test data for np2p distributed backup system.";
    let backup_key = [0x55u8; 32];

    // 3. Client: Encrypt and Shard
    let (shards, encrypted_size) = StorageEngine::process_for_backup(original_data, &backup_key).unwrap();
    assert_eq!(shards.len(), 5);

    // 4. Client: Connect and Store Shards
    let conn = client_node.connect(server_addr).await.expect("Failed to connect");
    
    // Store each shard in a separate stream (simulating parallel storage)
    for (i, shard_data) in shards.iter().enumerate() {
        let (mut send, mut recv) = conn.open_bi().await.unwrap();
        
        let shard_hash = blake3::hash(shard_data).into();
        let store_req = Message::StoreShardRequest {
            shard_hash,
            data: shard_data.clone(),
        };
        
        Protocol::send(&mut send, &store_req).await.unwrap();
        let resp = Protocol::receive(&mut recv).await.unwrap();
        
        if let Message::StoreShardResponse { success, .. } = resp {
            assert!(success, "Shard {} storage failed", i);
        } else {
            panic!("Unexpected response for shard {}", i);
        }
        send.finish().unwrap();
    }

    // 5. Client: Retrieve Shards
    let mut retrieved_shards: Vec<Option<Vec<u8>>> = vec![None; 5];
    
    for i in 0..3 { // Retrieve only 3 shards (minimum needed for 3/5 EC)
        let (mut send, mut recv) = conn.open_bi().await.unwrap();
        
        let shard_hash = blake3::hash(&shards[i]).into();
        let retrieve_req = Message::RetrieveShardRequest { shard_hash };
        
        Protocol::send(&mut send, &retrieve_req).await.unwrap();
        let resp = Protocol::receive(&mut recv).await.unwrap();
        
        if let Message::RetrieveShardResponse { data, .. } = resp {
            retrieved_shards[i] = data;
        } else {
            panic!("Unexpected response for shard retrieval {}", i);
        }
        send.finish().unwrap();
    }

    // 6. Client: Reconstruct and Decrypt
    let restored_data = StorageEngine::restore_from_backup(retrieved_shards, encrypted_size, &backup_key).unwrap();
    
    assert_eq!(original_data.to_vec(), restored_data);
    
    conn.close(0u32.into(), b"done");
}
