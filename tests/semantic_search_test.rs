//! Integration test for the semantic-search SQL (`perform_semantic_search`).
//!
//! Regression guard for the bug where the images+videos UNION (a) joined the wrong
//! stars table for videos and (b) defeated the pgvector HNSW KNN index. Drives the
//! query directly with a supplied embedding — no AI/gRPC service needed.

use actix_web::web;
use pgvector::Vector;
use reminisce::db::MainDbPool;
use reminisce::services::embedding::perform_semantic_search;
use reminisce::test_utils::setup_test_database_with_instance;
use serial_test::serial;

const TEST_USER: &str = "550e8400-e29b-41d4-a716-446655440000";

/// Build a 1152-dim unit vector whose cosine similarity to the query
/// ([1,0,0,...]) is exactly `dominant` (after normalization with `second`).
fn vec_with_sim(dominant: f32, second: f32) -> Vec<f32> {
    let mut v = vec![0.0f32; 1152];
    v[0] = dominant;
    v[1] = second;
    let norm = (dominant * dominant + second * second).sqrt();
    if norm > 0.0 {
        v[0] /= norm;
        v[1] /= norm;
    }
    v
}

fn query_vec() -> Vector {
    Vector::from(vec_with_sim(1.0, 0.0))
}

fn user_uuid() -> uuid::Uuid {
    uuid::Uuid::parse_str(TEST_USER).unwrap()
}

async fn insert_image(pool: &deadpool_postgres::Pool, hash: &str, sim_dominant: f32, sim_second: f32) {
    let client = pool.get().await.unwrap();
    client.execute(
        "INSERT INTO images (user_id, deviceid, hash, name, ext, embedding) VALUES ($1, $2, $3, $4, $5, $6)",
        &[&user_uuid(), &"dev", &hash, &format!("{}.jpg", hash), &"jpg", &Vector::from(vec_with_sim(sim_dominant, sim_second))],
    ).await.unwrap();
}

async fn insert_video(pool: &deadpool_postgres::Pool, hash: &str, sim_dominant: f32, sim_second: f32) {
    let client = pool.get().await.unwrap();
    client.execute(
        "INSERT INTO videos (user_id, deviceid, hash, name, ext, embedding) VALUES ($1, $2, $3, $4, $5, $6)",
        &[&user_uuid(), &"dev", &hash, &format!("{}.mp4", hash), &"mp4", &Vector::from(vec_with_sim(sim_dominant, sim_second))],
    ).await.unwrap();
}

async fn seed(pool: &deadpool_postgres::Pool) {
    // Similarity to query (desc): img_a(1.0) > vid_a(0.9) > img_b(0.8) > vid_b(0.3)
    insert_image(pool, "img_a", 1.0, 0.0).await;
    insert_video(pool, "vid_a", 0.9, 0.4359).await;
    insert_image(pool, "img_b", 0.8, 0.6).await;
    insert_video(pool, "vid_b", 0.3, 0.9539).await;

    let client = pool.get().await.unwrap();
    // Star one image and one video (each in its OWN stars table).
    client.execute("INSERT INTO starred_images (user_id, hash) VALUES ($1, $2)", &[&user_uuid(), &"img_a"]).await.unwrap();
    client.execute("INSERT INTO starred_videos (user_id, hash) VALUES ($1, $2)", &[&user_uuid(), &"vid_a"]).await.unwrap();
}

#[tokio::test]
#[serial(db)]
async fn test_semantic_search_orders_by_similarity_and_types() {
    let (pool, _db) = setup_test_database_with_instance().await;
    seed(&pool).await;
    let pool_data = web::Data::new(MainDbPool(pool));

    let results = perform_semantic_search(
        &query_vec(), &user_uuid(), None, false, 0.0, 10, 0,
        None, None, None, None, None, &pool_data,
    ).await.expect("semantic search failed");

    assert_eq!(results.len(), 4, "all 4 media should match (min_similarity=0)");
    // Ordered by ascending cosine distance == descending similarity.
    assert_eq!(results[0].hash, "img_a");
    assert_eq!(results[1].hash, "vid_a");
    assert_eq!(results[2].hash, "img_b");
    assert_eq!(results[3].hash, "vid_b");

    assert_eq!(results[0].media_type, "image");
    assert_eq!(results[1].media_type, "video");
    assert_eq!(results[3].media_type, "video");
}

#[tokio::test]
#[serial(db)]
async fn test_semantic_search_video_starred_flag() {
    let (pool, _db) = setup_test_database_with_instance().await;
    seed(&pool).await;
    let pool_data = web::Data::new(MainDbPool(pool));

    let results = perform_semantic_search(
        &query_vec(), &user_uuid(), None, false, 0.0, 10, 0,
        None, None, None, None, None, &pool_data,
    ).await.expect("semantic search failed");

    let vid_a = results.iter().find(|r| r.hash == "vid_a").expect("vid_a missing");
    let vid_b = results.iter().find(|r| r.hash == "vid_b").expect("vid_b missing");
    let img_a = results.iter().find(|r| r.hash == "img_a").expect("img_a missing");

    // The exact regression: a starred VIDEO must report starred=true (joined starred_videos).
    assert!(vid_a.starred, "starred video must have starred=true (join starred_videos)");
    assert!(!vid_b.starred, "unstarred video must have starred=false");
    assert!(img_a.starred, "starred image must have starred=true");
}

#[tokio::test]
#[serial(db)]
async fn test_semantic_search_starred_only_includes_videos() {
    let (pool, _db) = setup_test_database_with_instance().await;
    seed(&pool).await;
    let pool_data = web::Data::new(MainDbPool(pool));

    let results = perform_semantic_search(
        &query_vec(), &user_uuid(), None, true, 0.0, 10, 0,
        None, None, None, None, None, &pool_data,
    ).await.expect("semantic search failed");

    // Only starred items, and — critically — the starred video is NOT filtered out.
    assert_eq!(results.len(), 2, "only the 2 starred items should be returned");
    assert!(results.iter().any(|r| r.hash == "img_a"));
    assert!(results.iter().any(|r| r.hash == "vid_a"), "starred_only must include starred videos");
}

#[tokio::test]
#[serial(db)]
async fn test_semantic_search_limit_and_offset() {
    let (pool, _db) = setup_test_database_with_instance().await;
    seed(&pool).await;
    let pool_data = web::Data::new(MainDbPool(pool));

    // First page of 2.
    let page1 = perform_semantic_search(
        &query_vec(), &user_uuid(), None, false, 0.0, 2, 0,
        None, None, None, None, None, &pool_data,
    ).await.expect("page1 failed");
    assert_eq!(page1.len(), 2);
    assert_eq!(page1[0].hash, "img_a");
    assert_eq!(page1[1].hash, "vid_a");

    // Second page of 2 (offset 2) — continues the KNN ordering.
    let page2 = perform_semantic_search(
        &query_vec(), &user_uuid(), None, false, 0.0, 2, 2,
        None, None, None, None, None, &pool_data,
    ).await.expect("page2 failed");
    assert_eq!(page2.len(), 2);
    assert_eq!(page2[0].hash, "img_b");
    assert_eq!(page2[1].hash, "vid_b");

    // No overlap between pages.
    assert!(!page1.iter().any(|r| page2.iter().any(|q| q.hash == r.hash)));
}
