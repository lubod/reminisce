use actix_web::{http, test, web, App};
use reminisce::*;
use reminisce::test_utils::setup_test_database_with_instance;
use chrono;
use serial_test::serial;
use std::fs;
use std::path::Path;

mod common;

/// Test that device_id from the multipart upload form field is correctly stored in the database.
/// Android sends device_id as a multipart field alongside hash, name, and file data.
#[actix_web::test]
#[serial]
async fn test_upload_image_stores_device_id_from_multipart() {
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
            .service(upload_image)
    ).await;

    let token = common::utils::create_test_jwt_token().await;
    let image_bytes = fs::read("tests/test_image.jpg").unwrap();
    let thumbnail_bytes = fs::read("tests/test_thumbnail.jpg").unwrap();
    let custom_device_id = "android_pixel_7_abc123";

    let (form, content_type) = common::multipart_builder::create_multipart_payload_with_device_id(
        common::TEST_IMAGE_HASH,
        common::TEST_IMAGE_NAME,
        &image_bytes,
        &thumbnail_bytes,
        custom_device_id,
    );

    let req = test::TestRequest::post()
        .uri("/upload/image")
        .insert_header(("Authorization", format!("Bearer {}", token)))
        .insert_header(("Content-Type", content_type))
        .set_payload(form)
        .to_request();

    let response = test::call_service(&app, req).await;
    assert_eq!(response.status(), http::StatusCode::CREATED);

    // Verify the device_id was correctly stored in the database
    let client = pool.get().await.expect("Failed to get client");
    let row = client
        .query_one(
            "SELECT deviceid FROM images WHERE hash = $1",
            &[&common::TEST_IMAGE_HASH],
        ).await
        .expect("Failed to query database");

    let stored_device_id: &str = row.get(0);
    assert_eq!(stored_device_id, custom_device_id, "Device ID from multipart should be stored in DB");

    // Clean up
    let subdir = &common::TEST_IMAGE_HASH[..2];
    let file_path = Path::new(common::TEST_UPLOAD_DIR)
        .join(subdir)
        .join(format!("{}.jpg", common::TEST_IMAGE_HASH));
    fs::remove_file(&file_path).ok();
    let thumb_path = Path::new(common::TEST_UPLOAD_DIR)
        .join(subdir)
        .join(format!("{}.thumb.jpg", common::TEST_IMAGE_HASH));
    fs::remove_file(&thumb_path).ok();
    client.execute("DELETE FROM images WHERE hash = $1", &[&common::TEST_IMAGE_HASH]).await.ok();
}

/// Test that upload without a device_id field defaults to "web-client".
#[actix_web::test]
#[serial]
async fn test_upload_image_defaults_device_id_to_web_client() {
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
            .service(upload_image)
    ).await;

    let token = common::utils::create_test_jwt_token().await;
    let image_bytes = fs::read("tests/test_image.jpg").unwrap();
    let thumbnail_bytes = fs::read("tests/test_thumbnail.jpg").unwrap();

    // Use the builder WITHOUT explicit device_id — should default
    let (form, content_type) = common::multipart_builder::create_multipart_payload(
        common::TEST_IMAGE_HASH,
        common::TEST_IMAGE_NAME,
        &image_bytes,
        &thumbnail_bytes,
    );

    let req = test::TestRequest::post()
        .uri("/upload/image")
        .insert_header(("Authorization", format!("Bearer {}", token)))
        .insert_header(("Content-Type", content_type))
        .set_payload(form)
        .to_request();

    let response = test::call_service(&app, req).await;
    assert_eq!(response.status(), http::StatusCode::CREATED);

    // Verify the device_id defaulted correctly
    let client = pool.get().await.expect("Failed to get client");
    let row = client
        .query_one(
            "SELECT deviceid FROM images WHERE hash = $1",
            &[&common::TEST_IMAGE_HASH],
        ).await
        .expect("Failed to query database");

    let stored_device_id: &str = row.get(0);
    assert_eq!(stored_device_id, "test_device_id", "Default device_id from multipart builder should be stored");

    // Clean up
    let subdir = &common::TEST_IMAGE_HASH[..2];
    let file_path = Path::new(common::TEST_UPLOAD_DIR)
        .join(subdir)
        .join(format!("{}.jpg", common::TEST_IMAGE_HASH));
    fs::remove_file(&file_path).ok();
    let thumb_path = Path::new(common::TEST_UPLOAD_DIR)
        .join(subdir)
        .join(format!("{}.thumb.jpg", common::TEST_IMAGE_HASH));
    fs::remove_file(&thumb_path).ok();
    client.execute("DELETE FROM images WHERE hash = $1", &[&common::TEST_IMAGE_HASH]).await.ok();
}

/// Test upload_video_metadata endpoint — used for cross-device dedup with videos.
#[actix_web::test]
#[serial]
async fn test_upload_video_metadata_success() {
    common::init_log();
    let (pool, _test_db) = setup_test_database_with_instance().await;
    let client = pool.get().await.expect("Failed to get client from pool");

    let config = common::utils::create_test_config();

    // Insert a dummy video record for "original_device" (verified)
    client
        .execute(
            "INSERT INTO videos (hash, name, metadata, created_at, type, deviceid, ext, has_thumbnail, verification_status) VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9)",
            &[
                &"test_video_meta_hash",
                &"VID_20250101.mp4",
                &None::<&str>,
                &chrono::Utc::now(),
                &"camera",
                &"original_device",
                &"mp4",
                &true,
                &1i32,
            ],
        ).await
        .expect("Failed to insert test video");

    let main_pool = common::utils::wrap_main_pool(pool.clone());
    let geotagging_pool = common::utils::create_geotagging_pool().await;

    let app = test::init_service(
        App::new()
            .app_data(web::Data::new(main_pool))
            .app_data(web::Data::new(geotagging_pool))
            .app_data(web::Data::new(config.clone()))
            .service(upload_video_metadata)
    ).await;

    let token = common::utils::create_test_jwt_token().await;

    // Upload metadata from a second device
    let request_body = serde_json::json!({
        "deviceid": "second_device",
        "hash": "test_video_meta_hash",
        "name": "/sdcard/DCIM/VID_20250101.mp4",
        "ext": "mp4",
    });

    let req = test::TestRequest::post()
        .uri("/upload/video/metadata")
        .insert_header(("Authorization", format!("Bearer {}", token)))
        .set_json(&request_body)
        .to_request();

    let response = test::call_service(&app, req).await;
    assert_eq!(response.status(), http::StatusCode::OK);

    let body: serde_json::Value = test::read_body_json(response).await;
    assert_eq!(body["status"], "success");

    // Verify both records exist
    let rows = client
        .query("SELECT deviceid, name FROM videos WHERE hash = $1 ORDER BY deviceid", &[&"test_video_meta_hash"])
        .await
        .expect("Failed to query database");

    assert_eq!(rows.len(), 2, "Should have 2 entries (one per device)");

    let device_ids: Vec<String> = rows.iter().map(|r| r.get::<_, String>("deviceid")).collect();
    assert!(device_ids.contains(&"original_device".to_string()));
    assert!(device_ids.contains(&"second_device".to_string()));

    // Clean up
    client.execute("DELETE FROM videos WHERE hash = $1", &[&"test_video_meta_hash"]).await.ok();
}

/// Test soft-delete of a video (admin only).
#[actix_web::test]
#[serial]
async fn test_delete_video_soft_delete() {
    common::init_log();
    let (pool, _test_db) = setup_test_database_with_instance().await;
    let client = pool.get().await.expect("Failed to get client from pool");

    let config = common::utils::create_test_config();

    // Insert test video record
    client
        .execute(
            "INSERT INTO videos (hash, name, metadata, created_at, type, deviceid, ext) VALUES ($1, $2, $3, $4, $5, $6, $7)",
            &[
                &"test_delete_video_hash",
                &"VID_delete.mp4",
                &None::<&str>,
                &chrono::Utc::now(),
                &"camera",
                &"test_device_id",
                &"mp4",
            ],
        ).await
        .expect("Failed to insert test video");

    let main_pool = common::utils::wrap_main_pool(pool.clone());
    let geotagging_pool = common::utils::create_geotagging_pool().await;

    let app = test::init_service(
        App::new()
            .app_data(web::Data::new(main_pool))
            .app_data(web::Data::new(geotagging_pool))
            .app_data(web::Data::new(config.clone()))
            .service(delete_video)
    ).await;

    let token = common::utils::create_test_jwt_token().await;

    let req = test::TestRequest::post()
        .uri("/video/test_delete_video_hash/delete")
        .insert_header(("Authorization", format!("Bearer {}", token)))
        .to_request();

    let response = test::call_service(&app, req).await;
    assert_eq!(response.status(), http::StatusCode::OK);

    let body: serde_json::Value = test::read_body_json(response).await;
    assert_eq!(body["status"], "success");

    // Verify the video is soft-deleted (deleted_at is set)
    let row = client
        .query_one(
            "SELECT deleted_at FROM videos WHERE hash = $1",
            &[&"test_delete_video_hash"],
        ).await
        .expect("Failed to query database");

    let deleted_at: Option<chrono::DateTime<chrono::Utc>> = row.get(0);
    assert!(deleted_at.is_some(), "deleted_at should be set after soft delete");

    // Clean up
    client.execute("DELETE FROM videos WHERE hash = $1", &[&"test_delete_video_hash"]).await.ok();
}

/// Test delete_video returns 404 for non-existent video.
#[actix_web::test]
#[serial]
async fn test_delete_video_not_found() {
    let (pool, _test_db) = setup_test_database_with_instance().await;
    let config = common::utils::create_test_config();

    let main_pool = common::utils::wrap_main_pool(pool.clone());
    let geotagging_pool = common::utils::create_geotagging_pool().await;

    let app = test::init_service(
        App::new()
            .app_data(web::Data::new(main_pool))
            .app_data(web::Data::new(geotagging_pool))
            .app_data(web::Data::new(config.clone()))
            .service(delete_video)
    ).await;

    let token = common::utils::create_test_jwt_token().await;

    let req = test::TestRequest::post()
        .uri("/video/nonexistent_hash/delete")
        .insert_header(("Authorization", format!("Bearer {}", token)))
        .to_request();

    let response = test::call_service(&app, req).await;
    assert_eq!(response.status(), http::StatusCode::NOT_FOUND);
}

/// Test get_device_ids endpoint — returns all distinct device IDs for the user's media.
#[actix_web::test]
#[serial]
async fn test_get_device_ids() {
    common::init_log();
    let (pool, _test_db) = setup_test_database_with_instance().await;
    let client = pool.get().await.expect("Failed to get client from pool");

    let config = common::utils::create_test_config();

    // Insert images and videos from different devices, all belonging to the test user
    let user_uuid = uuid::Uuid::parse_str("550e8400-e29b-41d4-a716-446655440000").unwrap();

    client
        .execute(
            "INSERT INTO images (hash, name, exif, created_at, type, deviceid, ext, user_id) VALUES ($1, $2, $3, $4, $5, $6, $7, $8)",
            &[
                &"device_ids_test_img1",
                &"IMG_001.jpg",
                &None::<&str>,
                &chrono::Utc::now(),
                &"camera",
                &"pixel_7",
                &"jpg",
                &user_uuid,
            ],
        ).await
        .expect("Failed to insert image for pixel_7");

    client
        .execute(
            "INSERT INTO images (hash, name, exif, created_at, type, deviceid, ext, user_id) VALUES ($1, $2, $3, $4, $5, $6, $7, $8)",
            &[
                &"device_ids_test_img2",
                &"IMG_002.jpg",
                &None::<&str>,
                &chrono::Utc::now(),
                &"camera",
                &"samsung_s24",
                &"jpg",
                &user_uuid,
            ],
        ).await
        .expect("Failed to insert image for samsung_s24");

    client
        .execute(
            "INSERT INTO videos (hash, name, metadata, created_at, type, deviceid, ext, user_id) VALUES ($1, $2, $3, $4, $5, $6, $7, $8)",
            &[
                &"device_ids_test_vid1",
                &"VID_001.mp4",
                &None::<&str>,
                &chrono::Utc::now(),
                &"camera",
                &"ipad_pro",
                &"mp4",
                &user_uuid,
            ],
        ).await
        .expect("Failed to insert video for ipad_pro");

    let main_pool = common::utils::wrap_main_pool(pool.clone());
    let geotagging_pool = common::utils::create_geotagging_pool().await;

    let app = test::init_service(
        App::new()
            .app_data(web::Data::new(main_pool))
            .app_data(web::Data::new(geotagging_pool))
            .app_data(web::Data::new(config.clone()))
            .service(get_device_ids)
    ).await;

    let token = common::utils::create_test_jwt_token().await;

    let req = test::TestRequest::get()
        .uri("/device_ids")
        .insert_header(("Authorization", format!("Bearer {}", token)))
        .to_request();

    let response = test::call_service(&app, req).await;
    assert_eq!(response.status(), http::StatusCode::OK);

    let body: serde_json::Value = test::read_body_json(response).await;
    let device_ids = body["device_ids"].as_array().expect("device_ids should be an array");

    assert_eq!(device_ids.len(), 3, "Should have 3 distinct device IDs");

    let ids: Vec<&str> = device_ids.iter().map(|v| v.as_str().unwrap()).collect();
    assert!(ids.contains(&"pixel_7"));
    assert!(ids.contains(&"samsung_s24"));
    assert!(ids.contains(&"ipad_pro"));

    // Clean up
    client.execute("DELETE FROM images WHERE hash LIKE 'device_ids_test_%'", &[]).await.ok();
    client.execute("DELETE FROM videos WHERE hash LIKE 'device_ids_test_%'", &[]).await.ok();
}

/// Test get_device_ids with invalid token.
#[actix_web::test]
#[serial]
async fn test_get_device_ids_invalid_token() {
    let (pool, _test_db) = setup_test_database_with_instance().await;
    let config = common::utils::create_test_config();

    let main_pool = common::utils::wrap_main_pool(pool.clone());
    let geotagging_pool = common::utils::create_geotagging_pool().await;

    let app = test::init_service(
        App::new()
            .app_data(web::Data::new(main_pool))
            .app_data(web::Data::new(geotagging_pool))
            .app_data(web::Data::new(config.clone()))
            .service(get_device_ids)
    ).await;

    let req = test::TestRequest::get()
        .uri("/device_ids")
        .insert_header(("Authorization", "Bearer invalid_token"))
        .to_request();

    let response = test::call_service(&app, req).await;
    assert_eq!(response.status(), http::StatusCode::UNAUTHORIZED);
}
