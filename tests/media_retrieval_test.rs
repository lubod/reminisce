use actix_web::{ http, test, web, App };
use reminisce::*;
use reminisce::test_utils::setup_test_database_with_instance;
use serial_test::serial;
use std::fs;
use std::path::Path;
use tokio_util::bytes::Bytes;

mod common;

#[actix_web::test]
#[serial]
async fn test_get_image_success() {
    let (pool, _test_db) = setup_test_database_with_instance().await;
    let client = pool.get().await.expect("Failed to get client from pool");

    let config = common::utils::create_test_config();
    // Clean up any existing test data first
    client.execute("DELETE FROM images WHERE hash = $1", &[&common::TEST_IMAGE_HASH]).await.ok();

    // Insert test data
    client
        .execute(
            "INSERT INTO images (user_id, hash, name, exif, created_at, type, deviceid, ext) VALUES ($1, $2, $3, $4, $5, $6, $7, $8)",
            &[
                &uuid::Uuid::parse_str("550e8400-e29b-41d4-a716-446655440000").unwrap(),
                &common::TEST_IMAGE_HASH,
                &common::TEST_IMAGE_NAME,
                &Option::<&str>::None,
                &chrono::Utc::now(),
                &"camera",
                &"test_device_id",
                &"jpg",
            ]
        ).await
        .expect("Failed to insert test data");

    // Create image file for test
    let subdir = &common::TEST_IMAGE_HASH[..2];
    let sub_dir_path = Path::new(config.get_images_dir()).join(subdir);
    tokio::fs::create_dir_all(&sub_dir_path).await.unwrap();
    let extension = common::TEST_IMAGE_NAME.split('.').next_back().unwrap_or("jpg");
    let image_path = format!(
        "{}/{}/{}.{}",
        config.get_images_dir(),
        subdir,
        common::TEST_IMAGE_HASH,
        extension
    );
    let test_image_data = fs::read("tests/test_image.jpg").unwrap();
    fs::write(&image_path, &test_image_data).unwrap();


    // Wrap pools for dependency injection

    let main_pool = common::utils::wrap_main_pool(pool.clone());

    let geotagging_pool = common::utils::create_geotagging_pool().await;


    let app = test::init_service(
        App::new()
            .app_data(web::Data::new(main_pool.clone()))
            .app_data(web::Data::new(geotagging_pool.clone()))
            .app_data(web::Data::new(config.clone()))
            .service(get_image)
    ).await;

    let token = common::utils::create_test_jwt_token().await;

    let req = test::TestRequest
        ::get()
        .uri(&format!("/image/{}", common::TEST_IMAGE_HASH))
        .insert_header(("Authorization", format!("Bearer {}", token)))
        .to_request();

    let response = test::call_service(&app, req).await;
    assert_eq!(response.status(), http::StatusCode::OK);

    // Clean up
    fs::remove_file(&image_path).unwrap();
    client
        .execute("DELETE FROM images WHERE hash = $1", &[&common::TEST_IMAGE_HASH]).await
        .expect("Failed to clean up database");
}

#[actix_web::test]
#[serial]
async fn test_get_image_not_found() {
    let (pool, _test_db) = setup_test_database_with_instance().await;

    let config = common::utils::create_test_config();

    // Wrap pools for dependency injection
    let main_pool = common::utils::wrap_main_pool(pool.clone());
    let geotagging_pool = common::utils::create_geotagging_pool().await;

    let app = test::init_service(
        App::new()
            .app_data(web::Data::new(main_pool.clone()))
            .app_data(web::Data::new(geotagging_pool.clone()))
            .app_data(web::Data::new(config.clone()))
            .service(get_image)
    ).await;

    let token = common::utils::create_test_jwt_token().await;

    let req = test::TestRequest
        ::get()
        .uri("/image/nonexistent_hash")
        .insert_header(("Authorization", format!("Bearer {}", token)))
        .to_request();

    let response = test::call_service(&app, req).await;
    assert_eq!(response.status(), http::StatusCode::NOT_FOUND);
}

#[actix_web::test]
#[serial]
async fn test_get_image_invalid_token() {
    let (pool, _test_db) = setup_test_database_with_instance().await;

    let config = common::utils::create_test_config();

    // Wrap pools for dependency injection
    let main_pool = common::utils::wrap_main_pool(pool.clone());
    let geotagging_pool = common::utils::create_geotagging_pool().await;

    let app = test::init_service(
        App::new()
            .app_data(web::Data::new(main_pool.clone()))
            .app_data(web::Data::new(geotagging_pool.clone()))
            .app_data(web::Data::new(config.clone()))
            .service(get_image)
    ).await;

    let req = test::TestRequest
        ::get()
        .uri("/image/some_hash")
        .insert_header(("Authorization", "Bearer invalid_token"))
        .to_request();

    let response = test::call_service(&app, req).await;
    assert_eq!(response.status(), http::StatusCode::UNAUTHORIZED);
}

#[actix_web::test]
#[serial]
async fn test_get_video_success() {
    let (pool, _test_db) = setup_test_database_with_instance().await;
    let client = pool.get().await.expect("Failed to get client from pool");

    let config = common::utils::create_test_config();
    // Clean up any existing test data first
    client.execute("DELETE FROM videos WHERE hash = $1", &[&common::TEST_VIDEO_HASH]).await.ok();

    // Insert test data
    client
        .execute(
            "INSERT INTO videos (user_id, hash, name, metadata, created_at, type, deviceid, ext) VALUES ($1, $2, $3, $4, $5, $6, $7, $8)",
            &[
                &uuid::Uuid::parse_str("550e8400-e29b-41d4-a716-446655440000").unwrap(),
                &common::TEST_VIDEO_HASH,
                &common::TEST_VIDEO_NAME,
                &None::<&str>,
                &chrono::Utc::now(),
                &"camera",
                &"test_device_id",
                &"mp4",
            ]
        ).await
        .expect("Failed to insert test data");

    // Create video file for test
    let subdir = &common::TEST_VIDEO_HASH[..2];
    let sub_dir_path = Path::new(config.get_videos_dir()).join(subdir);
    tokio::fs::create_dir_all(&sub_dir_path).await.unwrap();
    let extension = common::TEST_VIDEO_NAME.split('.').next_back().unwrap_or("mp4");
    let video_path = format!(
        "{}/{}/{}.{}",
        config.get_videos_dir(),
        subdir,
        common::TEST_VIDEO_HASH,
        extension
    );
    let test_video_data = fs::read("tests/test_video.mp4").unwrap();
    fs::write(&video_path, &test_video_data).unwrap();


    // Wrap pools for dependency injection

    let main_pool = common::utils::wrap_main_pool(pool.clone());

    let geotagging_pool = common::utils::create_geotagging_pool().await;


    let app = test::init_service(
        App::new()
            .app_data(web::Data::new(main_pool.clone()))
            .app_data(web::Data::new(geotagging_pool.clone()))
            .app_data(web::Data::new(config.clone()))
            .service(get_video)
    ).await;

    let token = common::utils::create_test_jwt_token().await;

    let req = test::TestRequest
        ::get()
        .uri(&format!("/video/{}", common::TEST_VIDEO_HASH))
        .insert_header(("Authorization", format!("Bearer {}", token)))
        .to_request();

    let response = test::call_service(&app, req).await;
    assert_eq!(response.status(), http::StatusCode::OK);

    // Check that it's a video content type
    let content_type = response.headers().get("content-type").unwrap().to_str().unwrap();
    assert!(
        content_type.starts_with("video/") || content_type.starts_with("application/octet-stream")
    );

    // Clean up
    fs::remove_file(&video_path).unwrap();
    client
        .execute("DELETE FROM videos WHERE hash = $1", &[&common::TEST_VIDEO_HASH]).await
        .expect("Failed to clean up database");
}

#[actix_web::test]
#[serial]
async fn test_get_video_invalid_token() {
    let (pool, _test_db) = setup_test_database_with_instance().await;

    let config = common::utils::create_test_config();

    // Wrap pools for dependency injection
    let main_pool = common::utils::wrap_main_pool(pool.clone());
    let geotagging_pool = common::utils::create_geotagging_pool().await;

    let app = test::init_service(
        App::new()
            .app_data(web::Data::new(main_pool.clone()))
            .app_data(web::Data::new(geotagging_pool.clone()))
            .app_data(web::Data::new(config.clone()))
            .service(get_video)
    ).await;

    let req = test::TestRequest
        ::get()
        .uri("/video/some_hash")
        .insert_header(("Authorization", "Bearer invalid_token"))
        .to_request();

    let response = test::call_service(&app, req).await;
    assert_eq!(response.status(), http::StatusCode::UNAUTHORIZED);
}

#[actix_web::test]
#[serial]
async fn test_get_video_not_found() {
    let (pool, _test_db) = setup_test_database_with_instance().await;

    let config = common::utils::create_test_config();

    let main_pool = common::utils::wrap_main_pool(pool.clone());
    let geotagging_pool = common::utils::create_geotagging_pool().await;

    let app = test::init_service(
        App::new()
            .app_data(web::Data::new(main_pool.clone()))
            .app_data(web::Data::new(geotagging_pool.clone()))
            .app_data(web::Data::new(config.clone()))
            .service(get_video)
    ).await;

    let token = common::utils::create_test_jwt_token().await;

    let req = test::TestRequest::get()
        .uri("/video/nonexistent_hash")
        .insert_header(("Authorization", format!("Bearer {}", token)))
        .to_request();

    let response = test::call_service(&app, req).await;
    assert_eq!(response.status(), http::StatusCode::NOT_FOUND);
}

/// Test HEAD request on image endpoint - Android uses this to get Content-Disposition header
/// with the filename before downloading the full image.
#[actix_web::test]
#[serial]
async fn test_head_image_returns_content_disposition() {
    let (pool, _test_db) = setup_test_database_with_instance().await;
    let client = pool.get().await.expect("Failed to get client from pool");

    let config = common::utils::create_test_config();
    client.execute("DELETE FROM images WHERE hash = $1", &[&common::TEST_IMAGE_HASH]).await.ok();

    // Insert test data
    client
        .execute(
            "INSERT INTO images (user_id, hash, name, exif, created_at, type, deviceid, ext) VALUES ($1, $2, $3, $4, $5, $6, $7, $8)",
            &[
                &uuid::Uuid::parse_str("550e8400-e29b-41d4-a716-446655440000").unwrap(),
                &common::TEST_IMAGE_HASH,
                &common::TEST_IMAGE_NAME,
                &Option::<&str>::None,
                &chrono::Utc::now(),
                &"camera",
                &"test_device_id",
                &"jpg",
            ]
        ).await
        .expect("Failed to insert test data");

    // Create image file
    let subdir = &common::TEST_IMAGE_HASH[..2];
    let sub_dir_path = Path::new(config.get_images_dir()).join(subdir);
    tokio::fs::create_dir_all(&sub_dir_path).await.unwrap();
    let image_path = format!("{}/{}/{}.jpg", config.get_images_dir(), subdir, common::TEST_IMAGE_HASH);
    let test_image_data = fs::read("tests/test_image.jpg").unwrap();
    fs::write(&image_path, &test_image_data).unwrap();

    let main_pool = common::utils::wrap_main_pool(pool.clone());
    let geotagging_pool = common::utils::create_geotagging_pool().await;

    let app = test::init_service(
        App::new()
            .app_data(web::Data::new(main_pool.clone()))
            .app_data(web::Data::new(geotagging_pool.clone()))
            .app_data(web::Data::new(config.clone()))
            .service(get_image)
    ).await;

    let token = common::utils::create_test_jwt_token().await;

    // Send GET request and verify Content-Disposition header
    let req = test::TestRequest::get()
        .uri(&format!("/image/{}", common::TEST_IMAGE_HASH))
        .insert_header(("Authorization", format!("Bearer {}", token)))
        .to_request();

    let response = test::call_service(&app, req).await;
    assert_eq!(response.status(), http::StatusCode::OK);

    // Verify Content-Disposition header contains the original filename
    let content_disposition = response.headers().get("content-disposition")
        .expect("Response should have Content-Disposition header")
        .to_str().unwrap();
    assert!(
        content_disposition.contains(common::TEST_IMAGE_NAME),
        "Content-Disposition should contain the original filename '{}', got: '{}'",
        common::TEST_IMAGE_NAME, content_disposition
    );

    // Clean up
    fs::remove_file(&image_path).unwrap();
    client.execute("DELETE FROM images WHERE hash = $1", &[&common::TEST_IMAGE_HASH]).await.ok();
}

/// Test GET on video endpoint returns Content-Disposition header with filename.
/// Android uses this (or a HEAD request) to determine the original filename.
#[actix_web::test]
#[serial]
async fn test_get_video_returns_content_disposition() {
    let (pool, _test_db) = setup_test_database_with_instance().await;
    let client = pool.get().await.expect("Failed to get client from pool");

    let config = common::utils::create_test_config();
    client.execute("DELETE FROM videos WHERE hash = $1", &[&common::TEST_VIDEO_HASH]).await.ok();

    // Insert test data
    client
        .execute(
            "INSERT INTO videos (user_id, hash, name, metadata, created_at, type, deviceid, ext) VALUES ($1, $2, $3, $4, $5, $6, $7, $8)",
            &[
                &uuid::Uuid::parse_str("550e8400-e29b-41d4-a716-446655440000").unwrap(),
                &common::TEST_VIDEO_HASH,
                &common::TEST_VIDEO_NAME,
                &None::<&str>,
                &chrono::Utc::now(),
                &"camera",
                &"test_device_id",
                &"mp4",
            ]
        ).await
        .expect("Failed to insert test data");

    // Create video file
    let subdir = &common::TEST_VIDEO_HASH[..2];
    let sub_dir_path = Path::new(config.get_videos_dir()).join(subdir);
    tokio::fs::create_dir_all(&sub_dir_path).await.unwrap();
    let video_path = format!("{}/{}/{}.mp4", config.get_videos_dir(), subdir, common::TEST_VIDEO_HASH);
    let test_video_data = fs::read("tests/test_video.mp4").unwrap();
    fs::write(&video_path, &test_video_data).unwrap();

    let main_pool = common::utils::wrap_main_pool(pool.clone());
    let geotagging_pool = common::utils::create_geotagging_pool().await;

    let app = test::init_service(
        App::new()
            .app_data(web::Data::new(main_pool.clone()))
            .app_data(web::Data::new(geotagging_pool.clone()))
            .app_data(web::Data::new(config.clone()))
            .service(get_video)
    ).await;

    let token = common::utils::create_test_jwt_token().await;

    let req = test::TestRequest::get()
        .uri(&format!("/video/{}", common::TEST_VIDEO_HASH))
        .insert_header(("Authorization", format!("Bearer {}", token)))
        .to_request();

    let response = test::call_service(&app, req).await;
    assert_eq!(response.status(), http::StatusCode::OK);

    // Verify Content-Disposition header contains the original filename
    let content_disposition = response.headers().get("content-disposition")
        .expect("Response should have Content-Disposition header")
        .to_str().unwrap();
    assert!(
        content_disposition.contains(common::TEST_VIDEO_NAME),
        "Content-Disposition should contain the original filename '{}', got: '{}'",
        common::TEST_VIDEO_NAME, content_disposition
    );

    // Clean up
    fs::remove_file(&video_path).unwrap();
    client.execute("DELETE FROM videos WHERE hash = $1", &[&common::TEST_VIDEO_HASH]).await.ok();
}

#[actix_web::test]
async fn test_ping_service() {
    let app = test::init_service(App::new().service(reminisce::ping)).await;

    let req = test::TestRequest::get().uri("/ping").to_request();
    let response = test::call_service(&app, req).await;

    assert_eq!(response.status(), http::StatusCode::OK);
    let response_body = test::read_body(response).await;
    assert_eq!(response_body, Bytes::from_static(b"OK"));
}

#[actix_web::test]
#[serial]
async fn test_update_image_orientation() {
    let (pool, _test_db) = setup_test_database_with_instance().await;
    let client = pool.get().await.expect("Failed to get client from pool");

    let config = common::utils::create_test_config();
    client.execute("DELETE FROM images WHERE hash = $1", &[&common::TEST_IMAGE_HASH]).await.ok();

    client
        .execute(
            "INSERT INTO images (user_id, hash, name, exif, created_at, type, deviceid, ext, orientation, width, height) VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11)",
            &[
                &uuid::Uuid::parse_str("550e8400-e29b-41d4-a716-446655440000").unwrap(),
                &common::TEST_IMAGE_HASH,
                &common::TEST_IMAGE_NAME,
                &Option::<&str>::None,
                &chrono::Utc::now(),
                &"camera",
                &"test_device_id",
                &"jpg",
                &1i16,
                &4000i32,
                &3000i32,
            ]
        ).await
        .expect("Failed to insert test data");

    let app = test::init_service(
        App::new()
            .app_data(web::Data::new(reminisce::db::MainDbPool(pool.clone())))
            .app_data(web::Data::new(config.clone()))
            .service(update_image_orientation)
    ).await;

    let token = common::utils::create_test_jwt_token().await;

    // Rotate CW (1 -> 6, portrait)
    let req = test::TestRequest::post()
        .uri(&format!("/image/{}/orientation", common::TEST_IMAGE_HASH))
        .insert_header(("Authorization", format!("Bearer {}", token)))
        .set_json(serde_json::json!({ "rotate": "cw" }))
        .to_request();

    let response = test::call_service(&app, req).await;
    assert_eq!(response.status(), http::StatusCode::OK);
    let body: serde_json::Value = test::read_body_json(response).await;
    assert_eq!(body["status"], "success");
    assert_eq!(body["orientation"], 6);
    assert_eq!(body["orientation_label"], "Portrait");

    // Check DB
    let row = client.query_one("SELECT orientation FROM images WHERE hash = $1", &[&common::TEST_IMAGE_HASH]).await.unwrap();
    let o: i16 = row.get(0);
    assert_eq!(o, 6);

    // Rotate again CW (6 -> 3)
    let req = test::TestRequest::post()
        .uri(&format!("/image/{}/orientation", common::TEST_IMAGE_HASH))
        .insert_header(("Authorization", format!("Bearer {}", token)))
        .set_json(serde_json::json!({ "rotate": "cw" }))
        .to_request();
    let response = test::call_service(&app, req).await;
    let body: serde_json::Value = test::read_body_json(response).await;
    assert_eq!(body["orientation"], 3);

    // Clean up
    client.execute("DELETE FROM images WHERE hash = $1", &[&common::TEST_IMAGE_HASH]).await.ok();
}

#[actix_web::test]
#[serial]
async fn test_update_image_place() {
    let (pool, _test_db) = setup_test_database_with_instance().await;
    let client = pool.get().await.expect("Failed to get client from pool");

    let config = common::utils::create_test_config();
    client.execute("DELETE FROM images WHERE hash = $1", &[&common::TEST_IMAGE_HASH]).await.ok();

    client
        .execute(
            "INSERT INTO images (user_id, hash, name, exif, created_at, type, deviceid, ext) VALUES ($1, $2, $3, $4, $5, $6, $7, $8)",
            &[
                &uuid::Uuid::parse_str("550e8400-e29b-41d4-a716-446655440000").unwrap(),
                &common::TEST_IMAGE_HASH,
                &common::TEST_IMAGE_NAME,
                &Option::<&str>::None,
                &chrono::Utc::now(),
                &"camera",
                &"test_device_id",
                &"jpg",
            ]
        ).await
        .expect("Failed to insert test data");

    let app = test::init_service(
        App::new()
            .app_data(web::Data::new(reminisce::db::MainDbPool(pool.clone())))
            .app_data(web::Data::new(config.clone()))
            .service(update_image_place)
    ).await;

    let token = common::utils::create_test_jwt_token().await;

    // Update place with coordinates
    let req = test::TestRequest::post()
        .uri(&format!("/image/{}/place", common::TEST_IMAGE_HASH))
        .insert_header(("Authorization", format!("Bearer {}", token)))
        .set_json(serde_json::json!({
            "place": "Eiffel Tower, Paris",
            "latitude": 48.8584,
            "longitude": 2.2945
        }))
        .to_request();

    let response = test::call_service(&app, req).await;
    assert_eq!(response.status(), http::StatusCode::OK);
    let body: serde_json::Value = test::read_body_json(response).await;
    assert_eq!(body["status"], "success");
    assert_eq!(body["place"], "Eiffel Tower, Paris");

    // Verify DB
    let row = client.query_one("SELECT place, ST_Y(location::geometry), ST_X(location::geometry) FROM images WHERE hash = $1", &[&common::TEST_IMAGE_HASH]).await.unwrap();
    let place: String = row.get(0);
    let lat: f64 = row.get(1);
    let lon: f64 = row.get(2);
    assert_eq!(place, "Eiffel Tower, Paris");
    assert!((lat - 48.8584).abs() < 0.0001);
    assert!((lon - 2.2945).abs() < 0.0001);

    // 1. Reject partial coordinate pair (latitude without longitude)
    let req = test::TestRequest::post()
        .uri(&format!("/image/{}/place", common::TEST_IMAGE_HASH))
        .insert_header(("Authorization", format!("Bearer {}", token)))
        .set_json(serde_json::json!({
            "place": "Test",
            "latitude": 48.8584
        }))
        .to_request();
    let response = test::call_service(&app, req).await;
    assert_eq!(response.status(), http::StatusCode::BAD_REQUEST);

    // 2. Reject out-of-range coordinates
    let req = test::TestRequest::post()
        .uri(&format!("/image/{}/place", common::TEST_IMAGE_HASH))
        .insert_header(("Authorization", format!("Bearer {}", token)))
        .set_json(serde_json::json!({
            "latitude": 95.0,
            "longitude": 2.2945
        }))
        .to_request();
    let response = test::call_service(&app, req).await;
    assert_eq!(response.status(), http::StatusCode::BAD_REQUEST);

    // 3. Update coordinates without place (sets location, clears place, echoes persisted values)
    let req = test::TestRequest::post()
        .uri(&format!("/image/{}/place", common::TEST_IMAGE_HASH))
        .insert_header(("Authorization", format!("Bearer {}", token)))
        .set_json(serde_json::json!({
            "latitude": 51.5074,
            "longitude": -0.1278
        }))
        .to_request();
    let response = test::call_service(&app, req).await;
    assert_eq!(response.status(), http::StatusCode::OK);
    let body: serde_json::Value = test::read_body_json(response).await;
    assert_eq!(body["place"], serde_json::Value::Null);
    assert!((body["latitude"].as_f64().unwrap() - 51.5074).abs() < 0.0001);
    assert!((body["longitude"].as_f64().unwrap() - -0.1278).abs() < 0.0001);

    // 4. Update place without coordinates (preserves existing coordinates in DB and response)
    let req = test::TestRequest::post()
        .uri(&format!("/image/{}/place", common::TEST_IMAGE_HASH))
        .insert_header(("Authorization", format!("Bearer {}", token)))
        .set_json(serde_json::json!({
            "place": "London Eye"
        }))
        .to_request();
    let response = test::call_service(&app, req).await;
    assert_eq!(response.status(), http::StatusCode::OK);
    let body: serde_json::Value = test::read_body_json(response).await;
    assert_eq!(body["place"], "London Eye");
    assert!((body["latitude"].as_f64().unwrap() - 51.5074).abs() < 0.0001);
    assert!((body["longitude"].as_f64().unwrap() - -0.1278).abs() < 0.0001);

    // 5. Clear place and location
    let req = test::TestRequest::post()
        .uri(&format!("/image/{}/place", common::TEST_IMAGE_HASH))
        .insert_header(("Authorization", format!("Bearer {}", token)))
        .set_json(serde_json::json!({
            "place": ""
        }))
        .to_request();
    let response = test::call_service(&app, req).await;
    assert_eq!(response.status(), http::StatusCode::OK);
    let body: serde_json::Value = test::read_body_json(response).await;
    assert_eq!(body["place"], serde_json::Value::Null);
    assert_eq!(body["latitude"], serde_json::Value::Null);
    assert_eq!(body["longitude"], serde_json::Value::Null);

    // Clean up
    client.execute("DELETE FROM images WHERE hash = $1", &[&common::TEST_IMAGE_HASH]).await.ok();
}
