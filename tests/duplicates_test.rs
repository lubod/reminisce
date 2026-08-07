use actix_web::{ http, test, web, App };
use reminisce::*;
use reminisce::duplicate_worker::{ DuplicateWorkerStatus, SharedDuplicateStatus };
use reminisce::test_utils::setup_test_database_with_instance;
use serial_test::serial;
use std::sync::Arc;

mod common;

const TEST_USER: &str = "550e8400-e29b-41d4-a716-446655440000";

fn user_uuid() -> uuid::Uuid {
    uuid::Uuid::parse_str(TEST_USER).unwrap()
}

/// Authed request helper — a macro so callers never name the app type.
macro_rules! authed {
    ($method:ident, $app:expr, $uri:expr, $token:expr) => {{
        let req = test::TestRequest::$method()
            .uri($uri)
            .insert_header(("Authorization", format!("Bearer {}", $token)))
            .to_request();
        test::call_service($app, req)
    }};
}

async fn seed_image(pool: &deadpool_postgres::Pool, hash: &str, name: &str) {
    let client = pool.get().await.unwrap();
    client
        .execute(
            "INSERT INTO images (user_id, deviceid, hash, name, ext, has_thumbnail) VALUES ($1, $2, $3, $4, $5, true)",
            &[&user_uuid(), &"dev", &hash, &name, &"jpg"],
        )
        .await
        .expect("seed image failed");
}

async fn seed_pair(pool: &deadpool_postgres::Pool, a: &str, b: &str, sim: f32) {
    let client = pool.get().await.unwrap();
    let (ha, hb) = if a < b { (a, b) } else { (b, a) };
    client
        .execute(
            "INSERT INTO image_duplicate_pairs (hash_a, hash_b, similarity, user_id) VALUES ($1, $2, $3, $4)",
            &[&ha, &hb, &sim, &user_uuid()],
        )
        .await
        .expect("seed pair failed");
}

#[actix_web::test]
#[serial]
async fn test_duplicates_empty_library() {
    let (pool, _test_db) = setup_test_database_with_instance().await;
    let config = common::utils::create_test_config();
    let main_pool = common::utils::wrap_main_pool(pool.clone());
    let app = test::init_service(
        App::new()
            .app_data(web::Data::new(main_pool))
            .app_data(web::Data::new(config.clone()))
            .service(services::duplicates::get_duplicates)
    ).await;
    let token = common::utils::create_test_jwt_token().await;

    let resp = authed!(get, &app, "/duplicates", &token).await;
    assert_eq!(resp.status(), http::StatusCode::OK);
    let body: serde_json::Value = test::read_body_json(resp).await;
    assert_eq!(body["groups"].as_array().unwrap().len(), 0);
    assert_eq!(body["total_groups"].as_u64().unwrap(), 0);
}

#[actix_web::test]
#[serial]
async fn test_duplicates_groups_respect_threshold() {
    let (pool, _test_db) = setup_test_database_with_instance().await;
    seed_image(&pool, "i1", "one.jpg").await;
    seed_image(&pool, "i2", "two.jpg").await;
    seed_image(&pool, "i3", "three.jpg").await;
    seed_pair(&pool, "i1", "i2", 0.99).await;
    seed_pair(&pool, "i2", "i3", 0.90).await;

    let config = common::utils::create_test_config();
    let main_pool = common::utils::wrap_main_pool(pool.clone());
    let app = test::init_service(
        App::new()
            .app_data(web::Data::new(main_pool))
            .app_data(web::Data::new(config.clone()))
            .service(services::duplicates::get_duplicates)
    ).await;
    let token = common::utils::create_test_jwt_token().await;

    // threshold 0.95: only the 0.99 pair qualifies -> one group of {i1, i2}.
    let resp = authed!(get, &app, "/duplicates?threshold=0.95", &token).await;
    assert_eq!(resp.status(), http::StatusCode::OK);
    let body: serde_json::Value = test::read_body_json(resp).await;
    assert_eq!(body["total_groups"].as_u64().unwrap(), 1, "body: {}", body);
    let group = &body["groups"][0];
    assert_eq!(group["images"].as_array().unwrap().len(), 2);
    assert!((group["similarity"].as_f64().unwrap() - 0.99).abs() < 1e-6);

    // threshold 0.80: both pairs qualify -> the whole connected component {i1,i2,i3}.
    let resp = authed!(get, &app, "/duplicates?threshold=0.80", &token).await;
    let body: serde_json::Value = test::read_body_json(resp).await;
    assert_eq!(body["total_groups"].as_u64().unwrap(), 1);

    // Out-of-range threshold is clamped to 0.80..=1.0 (1.5 -> 1.0 -> no pairs).
    let resp = authed!(get, &app, "/duplicates?threshold=1.5", &token).await;
    let body: serde_json::Value = test::read_body_json(resp).await;
    assert_eq!(body["total_groups"].as_u64().unwrap(), 0);
}

#[actix_web::test]
#[serial]
async fn test_duplicates_status_and_scan() {
    let (pool, _test_db) = setup_test_database_with_instance().await;
    seed_image(&pool, "i1", "one.jpg").await;
    seed_image(&pool, "i2", "two.jpg").await;
    seed_pair(&pool, "i1", "i2", 0.99).await;

    let config = common::utils::create_test_config();
    let main_pool = common::utils::wrap_main_pool(pool.clone());
    let shared: SharedDuplicateStatus = Arc::new(tokio::sync::Mutex::new(DuplicateWorkerStatus::new()));
    let app = test::init_service(
        App::new()
            .app_data(web::Data::new(main_pool))
            .app_data(web::Data::new(config.clone()))
            .app_data(web::Data::new(shared.clone()))
            .service(services::duplicates::get_duplicate_status)
            .service(services::duplicates::trigger_duplicate_scan)
    ).await;
    let token = common::utils::create_test_jwt_token().await;

    // Status reports a fresh, idle worker.
    let resp = authed!(get, &app, "/duplicates/status", &token).await;
    assert_eq!(resp.status(), http::StatusCode::OK);
    let body: serde_json::Value = test::read_body_json(resp).await;
    assert!(body["running"].is_boolean());
    assert!(body["total_pairs"].is_number());

    // Scan clears pairs and marks images for re-scan (admin only; token is admin).
    let resp = authed!(post, &app, "/duplicates/scan", &token).await;
    assert_eq!(resp.status(), http::StatusCode::OK);
    let body: serde_json::Value = test::read_body_json(resp).await;
    assert_eq!(body["status"], "scan triggered");

    let client = pool.get().await.unwrap();
    let pairs_left: i64 = client
        .query_one("SELECT COUNT(*) FROM image_duplicate_pairs", &[])
        .await
        .unwrap()
        .get(0);
    assert_eq!(pairs_left, 0, "scan truncates duplicate pairs");
    let unchecked: i64 = client
        .query_one("SELECT COUNT(*) FROM images WHERE duplicates_checked_at IS NULL", &[])
        .await
        .unwrap()
        .get(0);
    assert_eq!(unchecked, 2, "scan resets duplicates_checked_at for all images");
}

#[actix_web::test]
#[serial]
async fn test_duplicates_requires_auth() {
    let (pool, _test_db) = setup_test_database_with_instance().await;
    let config = common::utils::create_test_config();
    let main_pool = common::utils::wrap_main_pool(pool.clone());
    let app = test::init_service(
        App::new()
            .app_data(web::Data::new(main_pool))
            .app_data(web::Data::new(config.clone()))
            .service(services::duplicates::get_duplicates)
    ).await;
    let req = test::TestRequest::get().uri("/duplicates").to_request();
    let resp = test::call_service(&app, req).await;
    assert_eq!(resp.status(), http::StatusCode::UNAUTHORIZED);
}
