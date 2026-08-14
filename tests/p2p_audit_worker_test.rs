use np2p::crypto::NodeIdentity;
use np2p::network::P2PService;
use std::sync::Arc;
/// Integration tests for p2p_audit_worker — focused on the DB-only helpers
/// (orphan cleanup, under-sharded file detection) that can be verified without
/// a live P2P network.
use reminisce::p2p_audit_worker::{cleanup_orphaned_shards, find_undersharded_files};
use reminisce::test_utils::setup_test_database_with_instance;
use serial_test::serial;
use uuid::Uuid;

const TEST_USER_ID: &str = "550e8400-e29b-41d4-a716-446655440000";

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

async fn ensure_test_node(client: &deadpool_postgres::Object) {
    client.execute(
        "INSERT INTO p2p_nodes (node_id) VALUES ('audit-test-node') ON CONFLICT DO NOTHING",
        &[],
    ).await.expect("ensure test node");
}

async fn insert_image_synced(client: &deadpool_postgres::Object, hash: &str) {
    let uid = Uuid::parse_str(TEST_USER_ID).unwrap();
    client.execute(
        "INSERT INTO images (user_id, deviceid, hash, name, ext, type, has_thumbnail, p2p_synced_at, p2p_encryption_key, p2p_encrypted_size)
         VALUES ($1, 'test', $2, $3, 'jpg', 'camera', false, NOW(), '\\x0000000000000000000000000000000000000000000000000000000000000000'::bytea, 100)",
        &[&uid, &hash, &format!("{}.jpg", hash)],
    ).await.expect("insert synced image");
}

async fn insert_image_deleted(client: &deadpool_postgres::Object, hash: &str) {
    let uid = Uuid::parse_str(TEST_USER_ID).unwrap();
    client.execute(
        "INSERT INTO images (user_id, deviceid, hash, name, ext, type, has_thumbnail, p2p_synced_at, deleted_at)
         VALUES ($1, 'test', $2, $3, 'jpg', 'camera', false, NOW(), NOW())",
        &[&uid, &hash, &format!("{}.jpg", hash)],
    ).await.expect("insert deleted image");
}

async fn insert_shard(client: &deadpool_postgres::Object, file_hash: &str, shard_index: i32) {
    ensure_test_node(client).await;
    let shard_hash = format!("fakehash_{file_hash}_{shard_index}");
    client.execute(
        "INSERT INTO p2p_shards (file_hash, shard_index, node_id, shard_hash)
         VALUES ($1, $2, 'audit-test-node', $3)
         ON CONFLICT DO NOTHING",
        &[&file_hash, &shard_index, &shard_hash],
    ).await.expect("insert shard");
}

// ---------------------------------------------------------------------------
// Tests: cleanup_orphaned_shards
// ---------------------------------------------------------------------------

#[tokio::test]
#[serial]
async fn test_cleanup_removes_shards_for_deleted_image() {
    let (pool, _db) = setup_test_database_with_instance().await;
    let client = pool.get().await.unwrap();

    let hash = "orphan_cleanup_deleted_001";
    insert_image_deleted(&client, hash).await;
    insert_shard(&client, hash, 0).await;
    insert_shard(&client, hash, 1).await;

    let before: i64 = client.query_one(
        "SELECT COUNT(*) FROM p2p_shards WHERE file_hash = $1", &[&hash],
    ).await.unwrap().get(0);
    assert_eq!(before, 2, "expected 2 shards before cleanup");

    let deleted = cleanup_orphaned_shards(&pool).await.expect("cleanup should succeed");
    assert_eq!(deleted, 2, "expected 2 orphaned rows purged");

    let after: i64 = client.query_one(
        "SELECT COUNT(*) FROM p2p_shards WHERE file_hash = $1", &[&hash],
    ).await.unwrap().get(0);
    assert_eq!(after, 0, "shards should be gone after cleanup");
}

#[tokio::test]
#[serial]
async fn test_cleanup_keeps_shards_for_active_image() {
    let (pool, _db) = setup_test_database_with_instance().await;
    let client = pool.get().await.unwrap();

    let hash = "orphan_cleanup_active_001";
    insert_image_synced(&client, hash).await;
    insert_shard(&client, hash, 0).await;
    insert_shard(&client, hash, 1).await;

    let deleted = cleanup_orphaned_shards(&pool).await.expect("cleanup should succeed");
    assert_eq!(deleted, 0, "should not delete shards for active image");

    let remaining: i64 = client.query_one(
        "SELECT COUNT(*) FROM p2p_shards WHERE file_hash = $1", &[&hash],
    ).await.unwrap().get(0);
    assert_eq!(remaining, 2);
}

#[tokio::test]
#[serial]
async fn test_cleanup_removes_shards_with_no_file_record_at_all() {
    let (pool, _db) = setup_test_database_with_instance().await;
    let client = pool.get().await.unwrap();

    // Insert a shard whose file_hash has no matching image or video row
    let hash = "orphan_no_file_001";
    ensure_test_node(&client).await;
    client.execute(
        "INSERT INTO p2p_shards (file_hash, shard_index, node_id, shard_hash)
         VALUES ($1, 0, 'audit-test-node', 'fakehash_orphan')
         ON CONFLICT DO NOTHING",
        &[&hash],
    ).await.unwrap();

    let deleted = cleanup_orphaned_shards(&pool).await.expect("cleanup should succeed");
    assert!(deleted >= 1, "at least one orphaned row should be deleted");
}

#[tokio::test]
#[serial]
async fn test_cleanup_mixed_active_and_deleted() {
    let (pool, _db) = setup_test_database_with_instance().await;
    let client = pool.get().await.unwrap();

    let active_hash = "orphan_mixed_active_001";
    let deleted_hash = "orphan_mixed_deleted_001";

    insert_image_synced(&client, active_hash).await;
    insert_image_deleted(&client, deleted_hash).await;
    insert_shard(&client, active_hash, 0).await;
    insert_shard(&client, deleted_hash, 0).await;

    let purged = cleanup_orphaned_shards(&pool).await.expect("cleanup should succeed");
    // Only the deleted file's shard should be purged
    assert_eq!(purged, 1, "only the deleted file's shard should be removed");

    let active_shards: i64 = client.query_one(
        "SELECT COUNT(*) FROM p2p_shards WHERE file_hash = $1", &[&active_hash],
    ).await.unwrap().get(0);
    assert_eq!(active_shards, 1, "active file's shard must survive");
}

// ---------------------------------------------------------------------------
// Tests: find_undersharded_files
// ---------------------------------------------------------------------------

#[tokio::test]
#[serial]
async fn test_find_undersharded_returns_file_with_no_shards() {
    let (pool, _db) = setup_test_database_with_instance().await;
    let client = pool.get().await.unwrap();

    let hash = "undersharded_none_001";
    insert_image_synced(&client, hash).await;
    // Deliberately no shards inserted

    let results = find_undersharded_files(&pool, 100).await.expect("query should succeed");
    assert!(results.contains(&hash.to_string()), "file with no shards should appear");
}

#[tokio::test]
#[serial]
async fn test_find_undersharded_returns_file_with_two_shards() {
    let (pool, _db) = setup_test_database_with_instance().await;
    let client = pool.get().await.unwrap();

    let hash = "undersharded_two_001";
    insert_image_synced(&client, hash).await;
    insert_shard(&client, hash, 0).await;
    insert_shard(&client, hash, 1).await;
    // Only 2 shards — below the threshold of 3

    let results = find_undersharded_files(&pool, 100).await.expect("query should succeed");
    assert!(results.contains(&hash.to_string()), "file with 2/5 shards should appear");
}

#[tokio::test]
#[serial]
async fn test_find_undersharded_does_not_return_fully_sharded_file() {
    let (pool, _db) = setup_test_database_with_instance().await;
    let client = pool.get().await.unwrap();

    let hash = "fullsharded_001";
    insert_image_synced(&client, hash).await;
    for i in 0..5 {
        insert_shard(&client, hash, i).await;
    }

    let results = find_undersharded_files(&pool, 100).await.expect("query should succeed");
    assert!(!results.contains(&hash.to_string()), "fully-sharded file must not appear");
}

#[tokio::test]
#[serial]
async fn test_find_undersharded_excludes_unsynced_files() {
    let (pool, _db) = setup_test_database_with_instance().await;
    let client = pool.get().await.unwrap();

    let uid = Uuid::parse_str(TEST_USER_ID).unwrap();
    let hash = "unsynced_no_shards_001";
    // Insert image WITHOUT p2p_synced_at (not yet backed up)
    client.execute(
        "INSERT INTO images (user_id, deviceid, hash, name, ext, type, has_thumbnail)
         VALUES ($1, 'test', $2, $3, 'jpg', 'camera', false)",
        &[&uid, &hash, &format!("{}.jpg", hash)],
    ).await.unwrap();

    let results = find_undersharded_files(&pool, 100).await.expect("query should succeed");
    assert!(!results.contains(&hash.to_string()), "unsynced file should not appear");
}

#[tokio::test]
#[serial]
async fn test_find_undersharded_respects_limit() {
    let (pool, _db) = setup_test_database_with_instance().await;
    let client = pool.get().await.unwrap();

    // Insert 5 synced images with no shards
    for i in 0..5 {
        let hash = format!("limit_test_{:03}", i);
        insert_image_synced(&client, &hash).await;
    }

    let results = find_undersharded_files(&pool, 3).await.expect("query should succeed");
    assert!(results.len() <= 3, "limit of 3 should be respected");
}


async fn insert_video_synced(client: &deadpool_postgres::Object, hash: &str) {
    let uid = Uuid::parse_str(TEST_USER_ID).unwrap();
    client.execute(
        "INSERT INTO videos (user_id, deviceid, hash, name, ext, type, has_thumbnail, p2p_synced_at, p2p_encryption_key, p2p_encrypted_size)
         VALUES ($1, 'test', $2, $3, 'mp4', 'camera', false, NOW(), '\\x0000000000000000000000000000000000000000000000000000000000000000'::bytea, 100)",
        &[&uid, &hash, &format!("{}.mp4", hash)],
    ).await.expect("insert synced video");
}

#[tokio::test]
#[serial]
async fn test_find_undersharded_includes_videos() {
    let (pool, _db) = setup_test_database_with_instance().await;
    let client = pool.get().await.unwrap();

    let hash = "undersharded_video_001";
    insert_video_synced(&client, hash).await;
    insert_shard(&client, hash, 0).await;
    insert_shard(&client, hash, 1).await;

    let results = find_undersharded_files(&pool, 100).await.expect("query should succeed");
    assert!(results.contains(&hash.to_string()), "synced video with 2/5 shards should appear");
}

#[tokio::test]
#[serial]
async fn test_find_undersharded_respects_custom_data_shards() {
    let (pool, _db) = setup_test_database_with_instance().await;
    let client = pool.get().await.unwrap();

    // Image with p2p_data_shards = 1 and a single shard -> not undersharded.
    let uid = Uuid::parse_str(TEST_USER_ID).unwrap();
    let hash = "undersharded_override_001";
    client.execute(
        "INSERT INTO images (user_id, deviceid, hash, name, ext, type, has_thumbnail, p2p_synced_at, p2p_data_shards)
         VALUES ($1, 'test', $2, $3, 'jpg', 'camera', false, NOW(), 1)",
        &[&uid, &hash, &format!("{}.jpg", hash)],
    ).await.unwrap();
    insert_shard(&client, hash, 0).await;

    let results = find_undersharded_files(&pool, 100).await.expect("query should succeed");
    assert!(!results.contains(&hash.to_string()), "1 shard meets its data_shards of 1");

    // Same again but with 2 required shards + 1 present -> undersharded.
    let hash2 = "undersharded_override_002";
    client.execute(
        "INSERT INTO images (user_id, deviceid, hash, name, ext, type, has_thumbnail, p2p_synced_at, p2p_data_shards)
         VALUES ($1, 'test', $2, $3, 'jpg', 'camera', false, NOW(), 2)",
        &[&uid, &hash2, &format!("{}.jpg", hash2)],
    ).await.unwrap();
    insert_shard(&client, hash2, 0).await;

    let results2 = find_undersharded_files(&pool, 100).await.expect("query should succeed");
    assert!(results2.contains(&hash2.to_string()), "1 shard is below its data_shards of 2");
}

#[tokio::test]
#[serial]
async fn test_cleanup_purges_shards_of_deleted_video() {
    let (pool, _db) = setup_test_database_with_instance().await;
    let client = pool.get().await.unwrap();

    let uid = Uuid::parse_str(TEST_USER_ID).unwrap();
    let hash = "orphan_cleanup_video_001";
    client.execute(
        "INSERT INTO videos (user_id, deviceid, hash, name, ext, type, has_thumbnail, p2p_synced_at, deleted_at)
         VALUES ($1, 'test', $2, $3, 'mp4', 'camera', false, NOW(), NOW())",
        &[&uid, &hash, &format!("{}.mp4", hash)],
    ).await.unwrap();
    insert_shard(&client, hash, 0).await;
    insert_shard(&client, hash, 1).await;

    let deleted = cleanup_orphaned_shards(&pool).await.expect("cleanup should succeed");
    assert_eq!(deleted, 2, "deleted video shards purged");
}

#[tokio::test]
#[serial]
async fn test_sweep_storage_node_orphans_prunes_unreferenced() {
    use reminisce::p2p_audit_worker::sweep_storage_node_orphans;
    use np2p::storage::DiskStorage;
    use np2p::network::{Node, ConnectionHandler};

    let (pool, _db) = setup_test_database_with_instance().await;
    let client = pool.get().await.unwrap();

    // 1. Setup local storage node
    let tmp_dir = std::env::temp_dir().join(format!("test_shards_{}", uuid::Uuid::new_v4()));
    let storage: DiskStorage = DiskStorage::new(&tmp_dir).await.unwrap();
    let server_identity = Arc::new(NodeIdentity::generate());
    let server_node_id = hex::encode(server_identity.node_id());

    let server_node = Node::new("127.0.0.1:0".parse().unwrap(), (*server_identity).clone()).unwrap();
    let server_addr = server_node.local_addr().unwrap();

    let storage_clone = storage.clone();
    let server_id_clone = server_identity.clone();
    tokio::spawn(async move {
        while let Some(incoming) = server_node.accept().await {
            let s = storage_clone.clone();
            let id = server_id_clone.clone();
            tokio::spawn(async move {
                if let Ok(conn) = incoming.await {
                    let handler = ConnectionHandler::new(conn, s, id);
                    handler.run().await;
                }
            });
        }
    });

    // 2. Setup client P2P service
    let client_identity = NodeIdentity::generate();
    let p2p_service = P2PService::new("127.0.0.1:0".parse().unwrap(), client_identity).await.unwrap();
    p2p_service.registry.upsert(server_node_id.clone(), server_addr);
    let p2p_service = Arc::new(p2p_service);

    // 3. Register node in database
    client.execute(
        "INSERT INTO p2p_nodes (node_id, public_addr, is_active) VALUES ($1, $2, true)",
        &[&server_node_id, &server_addr.to_string()],
    ).await.unwrap();

    // 4. Store 2 shards on the node:
    // Shard 1: Referenced in DB (valid)
    let shard1: [u8; 32] = [0x55u8; 32];
    let shard1_hex = hex::encode(shard1);
    storage.store(shard1, b"valid data".as_slice()).await.unwrap();

    let uid = Uuid::parse_str(TEST_USER_ID).unwrap();
    client.execute(
        "INSERT INTO images (user_id, deviceid, hash, name, ext, type, has_thumbnail, p2p_synced_at)
         VALUES ($1, 'test', 'img_valid_123', 'img.jpg', 'jpg', 'camera', false, NOW())",
        &[&uid],
    ).await.unwrap();
    client.execute(
        "INSERT INTO p2p_shards (file_hash, shard_index, node_id, shard_hash)
         VALUES ('img_valid_123', 0, $1, $2)",
        &[&server_node_id, &shard1_hex],
    ).await.unwrap();

    // Shard 2: Unreferenced on node (orphan)
    let shard2: [u8; 32] = [0x77u8; 32];
    storage.store(shard2, b"orphan data".as_slice()).await.unwrap();

    // 5. Run sweep
    let pruned = sweep_storage_node_orphans(&pool, &p2p_service).await.expect("sweep should succeed");
    assert_eq!(pruned, 1, "exactly 1 orphan shard pruned");

    // 6. Assert shard1 exists, shard2 was deleted
    assert!(storage.exists(shard1), "valid shard must remain on disk");
    assert!(!storage.exists(shard2), "orphan shard must be deleted from disk");

    let _ = std::fs::remove_dir_all(&tmp_dir);
}
