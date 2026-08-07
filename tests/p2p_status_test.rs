use actix_web::{ http, test, web, App };
use reminisce::*;
use reminisce::test_utils::setup_test_database_with_instance;
use serial_test::serial;

mod common;

const TEST_USER: &str = "550e8400-e29b-41d4-a716-446655440000";

fn user_uuid() -> uuid::Uuid {
    uuid::Uuid::parse_str(TEST_USER).unwrap()
}

macro_rules! authed_get {
    ($app:expr, $uri:expr, $token:expr) => {{
        let req = test::TestRequest::get()
            .uri($uri)
            .insert_header(("Authorization", format!("Bearer {}", $token)))
            .to_request();
        test::call_service($app, req)
    }};
}

async fn seed_synced_image(pool: &deadpool_postgres::Pool, hash: &str) {
    let client = pool.get().await.unwrap();
    let synced: chrono::DateTime<chrono::Utc> = "2024-01-01T00:00:00Z".parse().unwrap();
    client
        .execute(
            "INSERT INTO images (user_id, deviceid, hash, name, ext, has_thumbnail, p2p_synced_at) \
             VALUES ($1, $2, $3, $4, $5, true, $6)",
            &[&user_uuid(), &"dev", &hash, &format!("{}.jpg", hash), &"jpg", &synced],
        )
        .await
        .expect("seed synced image failed");
}

#[actix_web::test]
#[serial]
async fn test_verify_backup_empty_and_missing() {
    let (pool, _test_db) = setup_test_database_with_instance().await;
    let config = common::utils::create_test_config();
    let main_pool = common::utils::wrap_main_pool(pool.clone());
    let app = test::init_service(
        App::new()
            .app_data(web::Data::new(main_pool))
            .app_data(web::Data::new(config.clone()))
            .service(services::p2p_status::verify_p2p_backup)
    ).await;
    let token = common::utils::create_test_jwt_token().await;

    // No synced files -> all counters zero.
    let resp = authed_get!(&app, "/p2p/backup/verify", &token).await;
    assert_eq!(resp.status(), http::StatusCode::OK);
    let body: serde_json::Value = test::read_body_json(resp).await;
    assert_eq!(body["total_files"].as_u64().unwrap(), 0);
    assert_eq!(body["verified_files"].as_u64().unwrap(), 0);

    // A synced image with no reachable shards is reported "missing".
    seed_synced_image(&pool, "synced_no_shards").await;
    let resp = authed_get!(&app, "/p2p/backup/verify", &token).await;
    let body: serde_json::Value = test::read_body_json(resp).await;
    assert_eq!(body["total_files"].as_u64().unwrap(), 1, "body: {}", body);
    assert_eq!(body["missing_files"].as_u64().unwrap(), 1);
    assert_eq!(body["files"][0]["status"], "missing");
    assert_eq!(body["files"][0]["shards_available"].as_u64().unwrap(), 0);
}

#[actix_web::test]
#[serial]
async fn test_verify_backup_requires_auth() {
    let (pool, _test_db) = setup_test_database_with_instance().await;
    let config = common::utils::create_test_config();
    let main_pool = common::utils::wrap_main_pool(pool.clone());
    let app = test::init_service(
        App::new()
            .app_data(web::Data::new(main_pool))
            .app_data(web::Data::new(config.clone()))
            .service(services::p2p_status::verify_p2p_backup)
    ).await;
    let req = test::TestRequest::get().uri("/p2p/backup/verify").to_request();
    let resp = test::call_service(&app, req).await;
    assert_eq!(resp.status(), http::StatusCode::UNAUTHORIZED);
}

#[actix_web::test]
#[serial]
async fn test_backup_list_and_timestamps() {
    let (pool, _test_db) = setup_test_database_with_instance().await;
    let client = pool.get().await.unwrap();
    client
        .execute(
            "INSERT INTO db_backups (backup_hash, created_at, size_bytes, encrypted_size, encryption_key) \
             VALUES ('firsthash', NOW(), 1000, 2000, '\\x0000000000000000000000000000000000000000000000000000000000000000'::bytea)",
            &[],
        )
        .await
        .unwrap();

    let config = common::utils::create_test_config();
    let main_pool = common::utils::wrap_main_pool(pool.clone());
    let app = test::init_service(
        App::new()
            .app_data(web::Data::new(main_pool))
            .app_data(web::Data::new(config.clone()))
            .service(services::p2p_status::list_p2p_backups)
            .service(services::p2p_status::list_backup_timestamps)
    ).await;
    let token = common::utils::create_test_jwt_token().await;

    let resp = authed_get!(&app, "/p2p/backup/list", &token).await;
    assert_eq!(resp.status(), http::StatusCode::OK);
    let body: serde_json::Value = test::read_body_json(resp).await;
    let backups = body["backups"].as_array().unwrap().clone();
    assert_eq!(backups.len(), 1, "one backup listed: {}", body);
    assert_eq!(backups[0]["size"].as_u64().unwrap(), 1000);
    assert!(backups[0]["filename"].as_str().unwrap().contains("firsthash"), "filename from hash");

    let resp = authed_get!(&app, "/p2p/backup/timestamps", &token).await;
    assert_eq!(resp.status(), http::StatusCode::OK);
    let body: serde_json::Value = test::read_body_json(resp).await;
    let ts = body["timestamps"].as_array().unwrap().clone();
    assert_eq!(ts.len(), 1);
    assert!(ts[0].as_u64().unwrap() > 1_700_000_000, "a plausible epoch timestamp");
}

#[actix_web::test]
#[serial]
async fn test_invite_status() {
    let (pool, _test_db) = setup_test_database_with_instance().await;
    let config = common::utils::create_test_config();
    let main_pool = common::utils::wrap_main_pool(pool.clone());
    let app = test::init_service(
        App::new()
            .app_data(web::Data::new(main_pool))
            .app_data(web::Data::new(config.clone()))
            .service(services::p2p_status::get_invite_status)
    ).await;
    let req = test::TestRequest::get().uri("/p2p-invite-status").to_request();
    let resp = test::call_service(&app, req).await;
    assert_eq!(resp.status(), http::StatusCode::OK);
    let body: serde_json::Value = test::read_body_json(resp).await;
    assert_eq!(body["is_member"], true);
}
