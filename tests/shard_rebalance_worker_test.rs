/// Integration tests for shard_rebalance_worker — the DB-accessible helpers
/// (ensure_peers_registered, find_file_info, lookup_node_addr) that can be
/// exercised without a live P2P/QUIC connection.
use reminisce::shard_rebalance_worker::{ensure_peers_registered, find_file_info, lookup_node_addr};
use reminisce::test_utils::setup_test_database_with_instance;
use np2p::network::P2PService;
use np2p::crypto::NodeIdentity;
use serial_test::serial;
use std::net::SocketAddr;
use std::sync::Arc;
use uuid::Uuid;

const TEST_USER_ID: &str = "550e8400-e29b-41d4-a716-446655440000";

async fn make_test_p2p() -> Arc<P2PService> {
    let identity = NodeIdentity::generate();
    let addr: SocketAddr = "127.0.0.1:0".parse().unwrap();
    Arc::new(P2PService::new(addr, identity).await.expect("test P2P service"))
}

fn make_addr(port: u16) -> SocketAddr {
    format!("127.0.0.1:{}", port).parse().unwrap()
}

// ---------------------------------------------------------------------------
// ensure_peers_registered
// ---------------------------------------------------------------------------

#[tokio::test]
#[serial]
async fn test_ensure_peers_upserts_nodes() {
    let (pool, _db) = setup_test_database_with_instance().await;

    let nodes = vec![
        ("node_rebalance_001".to_string(), make_addr(9100)),
        ("node_rebalance_002".to_string(), make_addr(9101)),
    ];

    ensure_peers_registered(&pool, &nodes).await.expect("should succeed");

    let client = pool.get().await.unwrap();
    let count: i64 = client.query_one(
        "SELECT COUNT(*) FROM p2p_nodes WHERE node_id IN ('node_rebalance_001', 'node_rebalance_002') AND is_active = TRUE",
        &[],
    ).await.unwrap().get(0);
    assert_eq!(count, 2);
}

#[tokio::test]
#[serial]
async fn test_ensure_peers_updates_existing_node() {
    let (pool, _db) = setup_test_database_with_instance().await;

    let node_id = "node_rebalance_update_001";
    let nodes_v1 = vec![(node_id.to_string(), make_addr(9200))];
    let nodes_v2 = vec![(node_id.to_string(), make_addr(9201))];

    ensure_peers_registered(&pool, &nodes_v1).await.unwrap();
    ensure_peers_registered(&pool, &nodes_v2).await.unwrap();

    let client = pool.get().await.unwrap();
    let row = client.query_one(
        "SELECT public_addr FROM p2p_nodes WHERE node_id = $1",
        &[&node_id],
    ).await.unwrap();
    let addr: String = row.get(0);
    assert!(addr.contains("9201"), "address should be updated to new port");
}

#[tokio::test]
#[serial]
async fn test_ensure_peers_empty_list_is_noop() {
    let (pool, _db) = setup_test_database_with_instance().await;
    // Should not panic or error with empty list
    ensure_peers_registered(&pool, &[]).await.expect("empty list should be fine");
}

// ---------------------------------------------------------------------------
// find_file_info
// ---------------------------------------------------------------------------

#[tokio::test]
#[serial]
async fn test_find_file_info_returns_image_data() {
    let (pool, _db) = setup_test_database_with_instance().await;
    let client = pool.get().await.unwrap();

    let uid = Uuid::parse_str(TEST_USER_ID).unwrap();
    let hash = "rebalance_find_img_001";
    let key = vec![0xA1u8; 32];
    client.execute(
        "INSERT INTO images (user_id, deviceid, hash, name, ext, type, has_thumbnail, p2p_synced_at, p2p_encryption_key, p2p_encrypted_size)
         VALUES ($1, 'test', $2, $3, 'png', 'camera', false, NOW(), $4, 512)",
        &[&uid, &hash, &format!("{}.png", hash), &key],
    ).await.unwrap();

    let info = find_file_info(&client, hash).await.expect("query should succeed");
    assert!(info.is_some(), "file info should be found");
    let (ext, enc_key, enc_size) = info.unwrap();
    assert_eq!(ext, "png");
    assert_eq!(enc_key, Some(key));
    assert_eq!(enc_size, Some(512));
}

#[tokio::test]
#[serial]
async fn test_find_file_info_returns_video_data() {
    let (pool, _db) = setup_test_database_with_instance().await;
    let client = pool.get().await.unwrap();

    let uid = Uuid::parse_str(TEST_USER_ID).unwrap();
    let hash = "rebalance_find_vid_001";
    let key = vec![0xB2u8; 32];
    client.execute(
        "INSERT INTO videos (user_id, deviceid, hash, name, ext, type, has_thumbnail, p2p_synced_at, p2p_encryption_key, p2p_encrypted_size)
         VALUES ($1, 'test', $2, $3, 'mp4', 'camera', false, NOW(), $4, 1024)",
        &[&uid, &hash, &format!("{}.mp4", hash), &key],
    ).await.unwrap();

    let info = find_file_info(&client, hash).await.expect("query should succeed");
    assert!(info.is_some());
    let (ext, _, enc_size) = info.unwrap();
    assert_eq!(ext, "mp4");
    assert_eq!(enc_size, Some(1024));
}

#[tokio::test]
#[serial]
async fn test_find_file_info_returns_none_for_unknown_hash() {
    let (pool, _db) = setup_test_database_with_instance().await;
    let client = pool.get().await.unwrap();

    let info = find_file_info(&client, "no_such_hash_xyz").await.expect("query should not fail");
    assert!(info.is_none());
}

#[tokio::test]
#[serial]
async fn test_find_file_info_returns_none_key_when_key_is_null() {
    let (pool, _db) = setup_test_database_with_instance().await;
    let client = pool.get().await.unwrap();

    let uid = Uuid::parse_str(TEST_USER_ID).unwrap();
    let hash = "rebalance_null_key_001";
    client.execute(
        "INSERT INTO images (user_id, deviceid, hash, name, ext, type, has_thumbnail)
         VALUES ($1, 'test', $2, $3, 'jpg', 'camera', false)",
        &[&uid, &hash, &format!("{}.jpg", hash)],
    ).await.unwrap();

    let info = find_file_info(&client, hash).await.expect("query should not fail");
    assert!(info.is_some());
    let (_, enc_key, _) = info.unwrap();
    assert!(enc_key.is_none(), "key should be None when not set");
}

// ---------------------------------------------------------------------------
// lookup_node_addr — DB fallback path
// ---------------------------------------------------------------------------

#[tokio::test]
#[serial]
async fn test_lookup_node_addr_finds_via_db() {
    let (pool, _db) = setup_test_database_with_instance().await;
    let client = pool.get().await.unwrap();

    let node_id = "lookup_db_node_001";
    let expected_addr = "127.0.0.1:9300";
    client.execute(
        "INSERT INTO p2p_nodes (node_id, public_addr, is_active) VALUES ($1, $2, TRUE)
         ON CONFLICT (node_id) DO UPDATE SET public_addr = $2",
        &[&node_id, &expected_addr],
    ).await.unwrap();

    // Empty in-memory registry — forces DB fallback
    let p2p = make_test_p2p().await;

    let addr = lookup_node_addr(&pool, &p2p, node_id).await;
    assert!(addr.is_some(), "should find node via DB fallback");
    assert_eq!(addr.unwrap().to_string(), expected_addr);
}

#[tokio::test]
#[serial]
async fn test_lookup_node_addr_returns_none_for_unknown_node() {
    let (pool, _db) = setup_test_database_with_instance().await;
    let p2p = make_test_p2p().await;

    let addr = lookup_node_addr(&pool, &p2p, "no_such_node_xyz").await;
    assert!(addr.is_none());
}

#[tokio::test]
#[serial]
async fn test_lookup_node_addr_prefers_registry_over_db() {
    let (pool, _db) = setup_test_database_with_instance().await;
    let client = pool.get().await.unwrap();

    let node_id = "lookup_pref_node_001";
    // DB has stale address
    client.execute(
        "INSERT INTO p2p_nodes (node_id, public_addr, is_active) VALUES ($1, $2, TRUE)
         ON CONFLICT (node_id) DO UPDATE SET public_addr = $2",
        &[&node_id, &"127.0.0.1:9400"],
    ).await.unwrap();

    let p2p = make_test_p2p().await;
    // Register fresh address in the in-memory registry
    let fresh_addr: SocketAddr = "127.0.0.1:9401".parse().unwrap();
    p2p.registry.upsert(node_id.to_string(), fresh_addr);

    let addr = lookup_node_addr(&pool, &p2p, node_id).await;
    assert_eq!(addr.unwrap(), fresh_addr, "in-memory registry address should take priority");
}
