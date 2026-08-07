use deadpool_postgres::Pool;
use reminisce::media_replication_worker::requeue_under_replicated;
use reminisce::test_utils::setup_test_database_with_instance;
use serial_test::serial;

mod common;

const TEST_USER: &str = "550e8400-e29b-41d4-a716-446655440000";

fn user_uuid() -> uuid::Uuid {
    uuid::Uuid::parse_str(TEST_USER).unwrap()
}

async fn seed_image(pool: &Pool, hash: &str, synced: bool) {
    let client = pool.get().await.unwrap();
    let synced_at = if synced { "NOW()" } else { "NULL" };
    client
        .execute(
            &format!(
                "INSERT INTO images (user_id, deviceid, hash, name, ext, type, has_thumbnail, p2p_synced_at) \
                 VALUES ($1, 'dev', $2, 'pic.jpg', 'jpg', 'camera', true, {})",
                synced_at
            ),
            &[&user_uuid(), &hash],
        )
        .await
        .unwrap();
}

async fn seed_active_node(pool: &Pool) -> String {
    let client = pool.get().await.unwrap();
    let node = format!("node_{}", &uuid::Uuid::new_v4().to_string().replace('-', "")[..8]);
    client
        .execute(
            "INSERT INTO p2p_nodes (node_id, is_active, last_seen) VALUES ($1, true, NOW()) ON CONFLICT DO NOTHING",
            &[&node],
        )
        .await
        .unwrap();
    node
}

async fn add_shards(pool: &Pool, node: &str, file_hash: &str, count: i32) {
    let client = pool.get().await.unwrap();
    for i in 0..count {
        client
            .execute(
                "INSERT INTO p2p_shards (file_hash, shard_index, node_id, shard_hash) \
                 VALUES ($1, $2, $3, $4) ON CONFLICT DO NOTHING",
                &[&file_hash, &i, &node, &format!("sh_{}_{}", file_hash, i)],
            )
            .await
            .unwrap();
    }
}

#[actix_web::test]
#[serial]
async fn requeues_only_under_replicated_files() {
    let (pool, _db) = setup_test_database_with_instance().await;
    seed_image(&pool, "ok_full", true).await;       // 3 live shards -> replicated enough
    seed_image(&pool, "und_full", true).await;      // 1 live shard  -> under-replicated
    seed_image(&pool, "und_none", true).await;      // 0 shards      -> under-replicated
    seed_image(&pool, "not_synced", false).await;   // not synced -> untouched

    let node = seed_active_node(&pool).await;
    add_shards(&pool, &node, "ok_full", 3).await;
    add_shards(&pool, &node, "und_full", 1).await;

    // Target 3 shards: ok_full(3) keeps its sync; und_full(1) and und_none(0) requeue.
    let n = requeue_under_replicated(&pool, 100, 3).await.expect("requeue ok");
    assert_eq!(n, 2, "two files requeued");

    let client = pool.get().await.unwrap();
    let requeued: Vec<String> = client
        .query(
            "SELECT hash FROM images WHERE p2p_synced_at IS NULL ORDER BY hash",
            &[],
        )
        .await
        .unwrap()
        .iter()
        .map(|r| r.get(0))
        .collect();
    assert!(requeued.contains(&"und_full".to_string()), "und_full requeued: {:?}", requeued);
    assert!(requeued.contains(&"und_none".to_string()), "und_none requeued: {:?}", requeued);
    assert!(requeued.contains(&"not_synced".to_string()), "not_synced untouched: {:?}", requeued);
    assert!(!requeued.contains(&"ok_full".to_string()), "ok_full keeps sync: {:?}", requeued);
}

#[actix_web::test]
#[serial]
async fn requeue_respects_limit_and_target() {
    let (pool, _db) = setup_test_database_with_instance().await;
    for i in 0..4 {
        seed_image(&pool, &format!("und_{}", i), true).await;
    }
    let node = seed_active_node(&pool).await;
    add_shards(&pool, &node, "und_0", 1).await; // just 1 - still below target 5

    // target 5 -> all 4 are under-replicated, but only 2 requeued (limit).
    let n = requeue_under_replicated(&pool, 2, 5).await.expect("requeue ok");
    assert_eq!(n, 2, "limit respected");
}
