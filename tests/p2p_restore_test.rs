/// Integration tests for p2p_restore::restore_file_with_fetcher.
///
/// These tests use a real (ephemeral) Postgres DB spun up by TestDatabase,
/// but replace the network layer with an in-memory mock fetcher so no QUIC
/// peer is needed.
use reminisce::p2p_restore::restore_file_with_fetcher;
use reminisce::test_utils::setup_test_database_with_instance;
use np2p::storage::StorageEngine;
use std::collections::HashMap;
use std::sync::Arc;

const TEST_USER_ID: &str = "550e8400-e29b-41d4-a716-446655440000";

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Builds a mock shard store keyed by shard_hash_hex → bytes.
fn shard_store_from(shards: &[Vec<u8>]) -> HashMap<[u8; 32], Vec<u8>> {
    shards.iter().map(|s| {
        let hash: [u8; 32] = blake3::hash(s).into();
        (hash, s.clone())
    }).collect()
}

/// A fetcher closure that answers from an in-memory HashMap.
fn memory_fetcher(
    store: Arc<HashMap<[u8; 32], Vec<u8>>>,
) -> impl Fn(String, [u8; 32]) -> std::future::Ready<Option<Vec<u8>>> {
    move |_node_id, hash| {
        std::future::ready(store.get(&hash).cloned())
    }
}

/// Insert a minimal `images` row with p2p backup fields already populated.
async fn insert_image(
    client: &deadpool_postgres::Object,
    hash: &str,
    name: &str,
    ext: &str,
    key: &[u8],
    encrypted_size: i32,
    segment_count: i32,
    segment_enc_sizes: Option<&[i64]>,
) {
    let user_id = uuid::Uuid::parse_str(TEST_USER_ID).unwrap();
    client.execute(
        "INSERT INTO images \
         (user_id, deviceid, hash, name, ext, type, has_thumbnail, \
          p2p_synced_at, p2p_encryption_key, p2p_encrypted_size, \
          p2p_segment_count, p2p_segment_enc_sizes) \
         VALUES ($1, 'test', $2, $3, $4, 'camera', false, \
                 NOW(), $5, $6, $7, $8)",
        &[
            &user_id, &hash, &name, &ext,
            &key.to_vec(),
            &encrypted_size,
            &segment_count,
            &segment_enc_sizes,
        ],
    ).await.expect("insert image");
}

/// Insert a minimal `videos` row with p2p backup fields already populated.
async fn insert_video(
    client: &deadpool_postgres::Object,
    hash: &str,
    name: &str,
    ext: &str,
    key: &[u8],
    encrypted_size: i32,
    segment_count: i32,
    segment_enc_sizes: Option<&[i64]>,
) {
    let user_id = uuid::Uuid::parse_str(TEST_USER_ID).unwrap();
    client.execute(
        "INSERT INTO videos \
         (user_id, deviceid, hash, name, ext, type, has_thumbnail, \
          p2p_synced_at, p2p_encryption_key, p2p_encrypted_size, \
          p2p_segment_count, p2p_segment_enc_sizes) \
         VALUES ($1, 'test', $2, $3, $4, 'camera', false, \
                 NOW(), $5, $6, $7, $8)",
        &[
            &user_id, &hash, &name, &ext,
            &key.to_vec(),
            &encrypted_size,
            &segment_count,
            &segment_enc_sizes,
        ],
    ).await.expect("insert video");
}

/// Ensure the test node row exists (p2p_shards has a FK to p2p_nodes).
async fn ensure_test_node(client: &deadpool_postgres::Object) {
    client.execute(
        "INSERT INTO p2p_nodes (node_id) VALUES ('test-node') ON CONFLICT DO NOTHING",
        &[],
    ).await.expect("ensure test node");
}

/// Insert one row into `p2p_shards`.
async fn insert_shard(
    client: &deadpool_postgres::Object,
    file_hash: &str,
    shard_index: i32,
    shard_data: &[u8],
) {
    ensure_test_node(client).await;
    let shard_hash = blake3::hash(shard_data).to_hex().to_string();
    client.execute(
        "INSERT INTO p2p_shards (file_hash, shard_index, node_id, shard_hash) \
         VALUES ($1, $2, 'test-node', $3)",
        &[&file_hash, &shard_index, &shard_hash],
    ).await.expect("insert shard");
}

// ---------------------------------------------------------------------------
// Tests: restore_file_with_fetcher
// ---------------------------------------------------------------------------

#[tokio::test]
async fn test_restore_single_segment_image_all_shards() {
    let (pool, _db) = setup_test_database_with_instance().await;
    let client = pool.get().await.unwrap();

    let key = [0xA1u8; 32];
    let original = b"small image backup test data";
    let (shards, enc_size) = StorageEngine::process_for_backup(original, &key, &key).unwrap();

    let hash = "restore_test_single_001";
    insert_image(&client, hash, "photo", "jpg", &key, enc_size as i32, 1, None).await;
    for (i, shard) in shards.iter().enumerate() {
        insert_shard(&client, hash, i as i32, shard).await;
    }

    let store = Arc::new(shard_store_from(&shards));
    let restored = restore_file_with_fetcher(&pool, hash, memory_fetcher(store))
        .await
        .expect("restore should succeed");

    assert_eq!(restored.data, original);
    assert_eq!(restored.filename, "photo.jpg");
    assert_eq!(restored.media_type, "image");
}

#[tokio::test]
async fn test_restore_single_segment_video() {
    let (pool, _db) = setup_test_database_with_instance().await;
    let client = pool.get().await.unwrap();

    let key = [0xA2u8; 32];
    let original = vec![0xFFu8; 512];
    let (shards, enc_size) = StorageEngine::process_for_backup(&original, &key, &key).unwrap();

    let hash = "restore_test_video_001";
    insert_video(&client, hash, "clip", "mp4", &key, enc_size as i32, 1, None).await;
    for (i, shard) in shards.iter().enumerate() {
        insert_shard(&client, hash, i as i32, shard).await;
    }

    let store = Arc::new(shard_store_from(&shards));
    let restored = restore_file_with_fetcher(&pool, hash, memory_fetcher(store))
        .await
        .expect("restore should succeed");

    assert_eq!(restored.data, original);
    assert_eq!(restored.filename, "clip.mp4");
    assert_eq!(restored.media_type, "video");
}

#[tokio::test]
async fn test_restore_degraded_one_shard_missing() {
    let (pool, _db) = setup_test_database_with_instance().await;
    let client = pool.get().await.unwrap();

    let key = [0xA3u8; 32];
    let original = vec![0x55u8; 800];
    let (shards, enc_size) = StorageEngine::process_for_backup(&original, &key, &key).unwrap();

    let hash = "restore_test_degraded_001";
    insert_image(&client, hash, "degraded", "png", &key, enc_size as i32, 1, None).await;
    // Register all 5 shards in DB, but only store 4 in the mock (shard 2 is missing)
    for (i, shard) in shards.iter().enumerate() {
        insert_shard(&client, hash, i as i32, shard).await;
    }

    let mut store_map = shard_store_from(&shards);
    let missing_hash: [u8; 32] = blake3::hash(&shards[2]).into();
    store_map.remove(&missing_hash);
    let store = Arc::new(store_map);

    let restored = restore_file_with_fetcher(&pool, hash, memory_fetcher(store))
        .await
        .expect("should recover with 4/5 shards");

    assert_eq!(restored.data, original);
}

#[tokio::test]
async fn test_restore_fails_with_only_two_shards() {
    let (pool, _db) = setup_test_database_with_instance().await;
    let client = pool.get().await.unwrap();

    let key = [0xA4u8; 32];
    let original = vec![0x77u8; 200];
    let (shards, enc_size) = StorageEngine::process_for_backup(&original, &key, &key).unwrap();

    let hash = "restore_test_toofew_001";
    insert_image(&client, hash, "toofew", "jpg", &key, enc_size as i32, 1, None).await;
    for (i, shard) in shards.iter().enumerate() {
        insert_shard(&client, hash, i as i32, shard).await;
    }

    // Only expose 2 shards in the mock store
    let mut store_map = shard_store_from(&shards);
    let h1: [u8;32] = blake3::hash(&shards[1]).into(); store_map.remove(&h1);
    let h2: [u8;32] = blake3::hash(&shards[2]).into(); store_map.remove(&h2);
    let h3: [u8;32] = blake3::hash(&shards[3]).into(); store_map.remove(&h3);
    let store = Arc::new(store_map);

    let err = restore_file_with_fetcher(&pool, hash, memory_fetcher(store))
        .await
        .unwrap_err();

    assert!(err.to_string().contains("cannot restore"), "got: {}", err);
}

#[tokio::test]
async fn test_restore_hash_mismatch_treated_as_missing() {
    // Fetcher returns data that doesn't match the stored shard_hash.
    // Should be treated as unavailable, not as a successful fetch.
    let (pool, _db) = setup_test_database_with_instance().await;
    let client = pool.get().await.unwrap();

    let key = [0xA5u8; 32];
    let original = vec![0x88u8; 300];
    let (shards, enc_size) = StorageEngine::process_for_backup(&original, &key, &key).unwrap();

    let hash = "restore_test_mismatch_001";
    insert_image(&client, hash, "corrupt", "jpg", &key, enc_size as i32, 1, None).await;
    for (i, shard) in shards.iter().enumerate() {
        insert_shard(&client, hash, i as i32, shard).await;
    }

    // Shard 0's entry in the mock returns wrong bytes (hash won't match)
    let mut store_map = shard_store_from(&shards);
    let shard0_hash: [u8; 32] = blake3::hash(&shards[0]).into();
    store_map.insert(shard0_hash, b"corrupted garbage data".to_vec());
    let store = Arc::new(store_map);

    // Still has 4 valid shards — restore should succeed despite the corrupted one
    let restored = restore_file_with_fetcher(&pool, hash, memory_fetcher(store))
        .await
        .expect("should recover ignoring corrupted shard");

    assert_eq!(restored.data, original);
}

#[tokio::test]
async fn test_restore_file_not_found() {
    let (pool, _db) = setup_test_database_with_instance().await;

    let store = Arc::new(HashMap::<[u8; 32], Vec<u8>>::new());
    let err = restore_file_with_fetcher(&pool, "nonexistent_hash_xyz", memory_fetcher(store))
        .await
        .unwrap_err();

    assert!(err.to_string().contains("not found in database"), "got: {}", err);
}

#[tokio::test]
async fn test_restore_no_shards_in_db() {
    let (pool, _db) = setup_test_database_with_instance().await;
    let client = pool.get().await.unwrap();

    let key = [0xA6u8; 32];
    let hash = "restore_test_noshards_001";
    // Insert file metadata but NO shard rows
    insert_image(&client, hash, "noshards", "jpg", &key, 100, 1, None).await;

    let store = Arc::new(HashMap::<[u8; 32], Vec<u8>>::new());
    let err = restore_file_with_fetcher(&pool, hash, memory_fetcher(store))
        .await
        .unwrap_err();

    assert!(err.to_string().contains("No shards found"), "got: {}", err);
}

#[tokio::test]
async fn test_restore_multi_segment_full_fetch() {
    let (pool, _db) = setup_test_database_with_instance().await;
    let client = pool.get().await.unwrap();

    let key = [0xB1u8; 32];
    // Three segments of varying sizes
    let seg0 = vec![0x11u8; 1000];
    let seg1 = vec![0x22u8; 2000];
    let seg2 = vec![0x33u8; 500];

    let (s0, e0) = StorageEngine::process_for_backup(&seg0, &key, &key).unwrap();
    let (s1, e1) = StorageEngine::process_for_backup(&seg1, &key, &key).unwrap();
    let (s2, e2) = StorageEngine::process_for_backup(&seg2, &key, &key).unwrap();

    // Concatenate sub-shards to form full Pi shards
    let enc_sizes = [e0 as i64, e1 as i64, e2 as i64];
    let full_shards: Vec<Vec<u8>> = (0..5).map(|i| {
        let mut buf = Vec::new();
        for (shards, enc_size) in [(&s0, e0), (&s1, e1), (&s2, e2)] {
            let sub = (enc_size + 3 - 1) / 3;
            let mut chunk = shards[i][..sub.min(shards[i].len())].to_vec();
            chunk.resize(sub, 0);
            buf.extend_from_slice(&chunk);
        }
        buf
    }).collect();

    let hash = "restore_test_multiseg_001";
    insert_image(
        &client, hash, "bigfile", "jpg", &key,
        0, // p2p_encrypted_size unused for multi-segment
        3,
        Some(&enc_sizes),
    ).await;
    for (i, shard) in full_shards.iter().enumerate() {
        insert_shard(&client, hash, i as i32, shard).await;
    }

    let store = Arc::new(shard_store_from(&full_shards));
    let restored = restore_file_with_fetcher(&pool, hash, memory_fetcher(store))
        .await
        .expect("multi-segment restore should succeed");

    let mut expected = seg0.clone();
    expected.extend_from_slice(&seg1);
    expected.extend_from_slice(&seg2);
    assert_eq!(restored.data, expected);
    assert_eq!(restored.filename, "bigfile.jpg");
}

#[tokio::test]
async fn test_restore_multi_segment_one_shard_missing() {
    let (pool, _db) = setup_test_database_with_instance().await;
    let client = pool.get().await.unwrap();

    let key = [0xB2u8; 32];
    let seg0 = vec![0xAAu8; 600];
    let seg1 = vec![0xBBu8; 400];

    let (s0, e0) = StorageEngine::process_for_backup(&seg0, &key, &key).unwrap();
    let (s1, e1) = StorageEngine::process_for_backup(&seg1, &key, &key).unwrap();

    let enc_sizes = [e0 as i64, e1 as i64];
    let full_shards: Vec<Vec<u8>> = (0..5).map(|i| {
        let mut buf = Vec::new();
        for (shards, enc_size) in [(&s0, e0), (&s1, e1)] {
            let sub = (enc_size + 3 - 1) / 3;
            let mut chunk = shards[i][..sub.min(shards[i].len())].to_vec();
            chunk.resize(sub, 0);
            buf.extend_from_slice(&chunk);
        }
        buf
    }).collect();

    let hash = "restore_test_multiseg_degraded_001";
    insert_image(&client, hash, "bigdegraded", "mp4", &key, 0, 2, Some(&enc_sizes)).await;
    for (i, shard) in full_shards.iter().enumerate() {
        insert_shard(&client, hash, i as i32, shard).await;
    }

    // Drop one parity shard from the fetcher
    let mut store_map = shard_store_from(&full_shards);
    let hf4: [u8;32] = blake3::hash(&full_shards[4]).into(); store_map.remove(&hf4);
    let store = Arc::new(store_map);

    let restored = restore_file_with_fetcher(&pool, hash, memory_fetcher(store))
        .await
        .expect("should recover multi-segment with 4/5 shards");

    let mut expected = seg0.clone();
    expected.extend_from_slice(&seg1);
    assert_eq!(restored.data, expected);
}
