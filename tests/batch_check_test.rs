use actix_web::{http, test, web, App};
use reminisce::*;
use reminisce::test_utils::setup_test_database_with_instance;
use chrono;
use serial_test::serial;

mod common;

/// Test batch_check_images with empty hashes - should return empty array
#[actix_web::test]
#[serial]
async fn test_batch_check_images_empty() {
    common::init_log();
    let (pool, _test_db) = setup_test_database_with_instance().await;
    let config = common::utils::create_test_config();

    let main_pool = common::utils::wrap_main_pool(pool.clone());
    let geotagging_pool = common::utils::create_geotagging_pool().await;

    let app = test::init_service(
        App::new()
            .app_data(web::Data::new(main_pool))
            .app_data(web::Data::new(geotagging_pool))
            .app_data(web::Data::new(config.clone()))
            .service(batch_check_images)
    ).await;

    let token = common::utils::create_test_jwt_token().await;

    let request_body = serde_json::json!({
        "device_id": "test_device_id",
        "hashes": []
    });

    let req = test::TestRequest::post()
        .uri("/upload/batch-check-images")
        .insert_header(("Authorization", format!("Bearer {}", token)))
        .insert_header(("Content-Type", "application/json"))
        .set_json(&request_body)
        .to_request();

    let response = test::call_service(&app, req).await;
    assert_eq!(response.status(), http::StatusCode::OK);

    let body: serde_json::Value = test::read_body_json(response).await;
    assert_eq!(body["existing_hashes"], serde_json::json!([]));
}

/// Test batch_check_images when all files need upload (none exist)
#[actix_web::test]
#[serial]
async fn test_batch_check_images_all_need_upload() {
    common::init_log();
    let (pool, _test_db) = setup_test_database_with_instance().await;
    let config = common::utils::create_test_config();

    let main_pool = common::utils::wrap_main_pool(pool.clone());
    let geotagging_pool = common::utils::create_geotagging_pool().await;

    let app = test::init_service(
        App::new()
            .app_data(web::Data::new(main_pool))
            .app_data(web::Data::new(geotagging_pool))
            .app_data(web::Data::new(config.clone()))
            .service(batch_check_images)
    ).await;

    let token = common::utils::create_test_jwt_token().await;

    let request_body = serde_json::json!({
        "device_id": "test_device_id",
        "hashes": ["nonexistent_hash_1", "nonexistent_hash_2", "nonexistent_hash_3"]
    });

    let req = test::TestRequest::post()
        .uri("/upload/batch-check-images")
        .insert_header(("Authorization", format!("Bearer {}", token)))
        .insert_header(("Content-Type", "application/json"))
        .set_json(&request_body)
        .to_request();

    let response = test::call_service(&app, req).await;
    assert_eq!(response.status(), http::StatusCode::OK);

    let body: serde_json::Value = test::read_body_json(response).await;
    let existing = body["existing_hashes"].as_array().unwrap();
    assert_eq!(existing.len(), 0);
}

/// Test batch_check_images when some files exist for device
#[actix_web::test]
#[serial]
async fn test_batch_check_images_some_exist() {
    common::init_log();
    let (pool, _test_db) = setup_test_database_with_instance().await;
    let client = pool.get().await.expect("Failed to get client from pool");
    let config = common::utils::create_test_config();

    // Insert existing image for this device
    client
        .execute(
            "INSERT INTO images (hash, name, exif, created_at, type, deviceid, ext) VALUES ($1, $2, $3, $4, $5, $6, $7)",
            &[
                &"existing_hash_1",
                &"IMG_existing.jpg",
                &None::<&str>,
                &chrono::Utc::now(),
                &"camera",
                &"test_device_id",
                &"jpg",
            ]
        ).await
        .expect("Failed to insert test data");

    let main_pool = common::utils::wrap_main_pool(pool.clone());
    let geotagging_pool = common::utils::create_geotagging_pool().await;

    let app = test::init_service(
        App::new()
            .app_data(web::Data::new(main_pool))
            .app_data(web::Data::new(geotagging_pool))
            .app_data(web::Data::new(config.clone()))
            .service(batch_check_images)
    ).await;

    let token = common::utils::create_test_jwt_token().await;

    let request_body = serde_json::json!({
        "device_id": "test_device_id",
        "hashes": ["existing_hash_1", "new_hash_1"]
    });

    let req = test::TestRequest::post()
        .uri("/upload/batch-check-images")
        .insert_header(("Authorization", format!("Bearer {}", token)))
        .insert_header(("Content-Type", "application/json"))
        .set_json(&request_body)
        .to_request();

    let response = test::call_service(&app, req).await;
    assert_eq!(response.status(), http::StatusCode::OK);

    let body: serde_json::Value = test::read_body_json(response).await;
    let existing = body["existing_hashes"].as_array().unwrap();
    assert_eq!(existing.len(), 1);
    assert!(existing.contains(&serde_json::json!("existing_hash_1")));
}

// Note: Invalid token tests require the full app middleware stack (JWT validation middleware)
// which is not available in unit test setup. Auth is tested in auth_test.rs.

// ============== VIDEO TESTS ==============

/// Test batch_check_videos with empty hashes
#[actix_web::test]
#[serial]
async fn test_batch_check_videos_empty() {
    common::init_log();
    let (pool, _test_db) = setup_test_database_with_instance().await;
    let config = common::utils::create_test_config();

    let main_pool = common::utils::wrap_main_pool(pool.clone());
    let geotagging_pool = common::utils::create_geotagging_pool().await;

    let app = test::init_service(
        App::new()
            .app_data(web::Data::new(main_pool))
            .app_data(web::Data::new(geotagging_pool))
            .app_data(web::Data::new(config.clone()))
            .service(batch_check_videos)
    ).await;

    let token = common::utils::create_test_jwt_token().await;

    let request_body = serde_json::json!({
        "device_id": "test_device_id",
        "hashes": []
    });

    let req = test::TestRequest::post()
        .uri("/upload/batch-check-videos")
        .insert_header(("Authorization", format!("Bearer {}", token)))
        .insert_header(("Content-Type", "application/json"))
        .set_json(&request_body)
        .to_request();

    let response = test::call_service(&app, req).await;
    assert_eq!(response.status(), http::StatusCode::OK);

    let body: serde_json::Value = test::read_body_json(response).await;
    assert_eq!(body["existing_hashes"], serde_json::json!([]));
}

/// Test batch_check_videos when all files need upload
#[actix_web::test]
#[serial]
async fn test_batch_check_videos_all_need_upload() {
    common::init_log();
    let (pool, _test_db) = setup_test_database_with_instance().await;
    let config = common::utils::create_test_config();

    let main_pool = common::utils::wrap_main_pool(pool.clone());
    let geotagging_pool = common::utils::create_geotagging_pool().await;

    let app = test::init_service(
        App::new()
            .app_data(web::Data::new(main_pool))
            .app_data(web::Data::new(geotagging_pool))
            .app_data(web::Data::new(config.clone()))
            .service(batch_check_videos)
    ).await;

    let token = common::utils::create_test_jwt_token().await;

    let request_body = serde_json::json!({
        "device_id": "test_device_id",
        "hashes": ["video_hash_1", "video_hash_2"]
    });

    let req = test::TestRequest::post()
        .uri("/upload/batch-check-videos")
        .insert_header(("Authorization", format!("Bearer {}", token)))
        .insert_header(("Content-Type", "application/json"))
        .set_json(&request_body)
        .to_request();

    let response = test::call_service(&app, req).await;
    assert_eq!(response.status(), http::StatusCode::OK);

    let body: serde_json::Value = test::read_body_json(response).await;
    let existing = body["existing_hashes"].as_array().unwrap();
    assert_eq!(existing.len(), 0);
}

/// Test batch_check_videos when some exist for device
#[actix_web::test]
#[serial]
async fn test_batch_check_videos_some_exist() {
    common::init_log();
    let (pool, _test_db) = setup_test_database_with_instance().await;
    let client = pool.get().await.expect("Failed to get client from pool");
    let config = common::utils::create_test_config();

    // Insert existing video for this device
    client
        .execute(
            "INSERT INTO videos (hash, name, metadata, created_at, type, deviceid, ext) VALUES ($1, $2, $3, $4, $5, $6, $7)",
            &[
                &"existing_video_hash",
                &"VID_existing.mp4",
                &None::<&str>,
                &chrono::Utc::now(),
                &"camera",
                &"test_device_id",
                &"mp4",
            ]
        ).await
        .expect("Failed to insert test data");

    let main_pool = common::utils::wrap_main_pool(pool.clone());
    let geotagging_pool = common::utils::create_geotagging_pool().await;

    let app = test::init_service(
        App::new()
            .app_data(web::Data::new(main_pool))
            .app_data(web::Data::new(geotagging_pool))
            .app_data(web::Data::new(config.clone()))
            .service(batch_check_videos)
    ).await;

    let token = common::utils::create_test_jwt_token().await;

    let request_body = serde_json::json!({
        "device_id": "test_device_id",
        "hashes": ["existing_video_hash", "new_video_hash"]
    });

    let req = test::TestRequest::post()
        .uri("/upload/batch-check-videos")
        .insert_header(("Authorization", format!("Bearer {}", token)))
        .insert_header(("Content-Type", "application/json"))
        .set_json(&request_body)
        .to_request();

    let response = test::call_service(&app, req).await;
    assert_eq!(response.status(), http::StatusCode::OK);

    let body: serde_json::Value = test::read_body_json(response).await;
    let existing = body["existing_hashes"].as_array().unwrap();
    assert_eq!(existing.len(), 1);
    assert!(existing.contains(&serde_json::json!("existing_video_hash")));
}

// Note: Invalid token tests for videos also require full middleware stack.
// Auth is tested in auth_test.rs.
