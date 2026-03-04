use actix_web::{ http, test, web, App };
use reminisce::*;
use reminisce::test_utils::setup_test_database_with_instance;
use serial_test::serial;
use std::fs;
use std::path::Path;
use chrono::{DateTime, Utc, TimeZone};

mod common;

#[actix_web::test]
#[serial]
async fn test_upload_video() {
    common::init_log();
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
            .service(check_video_exists)
            .service(upload_video)
            .service(get_video)
    ).await;

    let token = common::utils::create_test_jwt_token().await;

    let video_bytes = fs::read("tests/test_video.mp4").unwrap();
    let thumbnail_bytes = fs::read("tests/test_thumbnail.jpg").unwrap();

    let (form, content_type) = common::multipart_builder::create_video_multipart_payload(
        common::TEST_VIDEO_HASH,
        common::TEST_VIDEO_NAME,
        &video_bytes,
        &thumbnail_bytes
    );

    let req = test::TestRequest
        ::post()
        .uri("/upload/video")
        .insert_header(("Authorization", format!("Bearer {}", token)))
        .insert_header(("Content-Type", content_type))
        .set_payload(form)
        .to_request();

    let response = test::call_service(&app, req).await;

    let status = response.status();
    assert_eq!(status, http::StatusCode::CREATED);

    // Verify the file was saved
    let subdir = &common::TEST_VIDEO_HASH[..2];
    let file_path = Path::new(common::TEST_VIDEOS_DIR)
        .join(subdir)
        .join(format!("{}.mp4", common::TEST_VIDEO_HASH));
    assert!(file_path.exists());

    // Verify the database entry
    let client = pool.get().await.expect("Failed to get client from pool");
    let row = client
        .query_one("SELECT ext FROM videos WHERE hash = $1", &[&common::TEST_VIDEO_HASH]).await
        .expect("Failed to query database");
    let ext: &str = row.get(0);
    assert_eq!(ext, "mp4");

    // Clean up the created file
    fs::remove_file(&file_path).unwrap();

    // Clean up the database entry
    client
        .execute("DELETE FROM videos WHERE hash = $1", &[&common::TEST_VIDEO_HASH]).await
        .expect("Failed to clean up database");
}

#[actix_web::test]
#[serial]
async fn test_upload_video_without_thumbnail() {
    common::init_log();
    let (pool, _test_db) = setup_test_database_with_instance().await;

    let config = common::utils::create_test_config();


    // Wrap pools for dependency injection

    let main_pool = common::utils::wrap_main_pool(pool.clone());

    let geotagging_pool = common::utils::create_mock_geotagging_pool(pool.clone());


    let app = test::init_service(
        App::new()
            .app_data(web::Data::new(main_pool.clone()))
            .app_data(web::Data::new(geotagging_pool.clone()))
            .app_data(web::Data::new(config.clone()))
            .service(upload_video)
    ).await;

    let token = common::utils::create_test_jwt_token().await;

    let video_bytes = fs::read("tests/test_video.mp4").unwrap();

    let (form, content_type) = common::multipart_builder::create_video_multipart_payload_without_thumbnail(
        common::TEST_VIDEO_HASH,
        common::TEST_VIDEO_NAME,
        &video_bytes
    );

    let req = test::TestRequest
        ::post()
        .uri("/upload/video")
        .insert_header(("Authorization", format!("Bearer {}", token)))
        .insert_header(("Content-Type", content_type))
        .set_payload(form)
        .to_request();

    let response = test::call_service(&app, req).await;

    let status = response.status();
    // The server should return a Created (201) status even when thumbnail is missing
    assert_eq!(status, http::StatusCode::CREATED);

    // Verify the file was saved
    let subdir = &common::TEST_VIDEO_HASH[..2];
    let file_path = Path::new(common::TEST_VIDEOS_DIR)
        .join(subdir)
        .join(format!("{}.mp4", common::TEST_VIDEO_HASH));
    
    assert!(file_path.exists());

    // Clean up
    fs::remove_file(&file_path).unwrap();
    
    let client = pool.get().await.expect("Failed to get client");
    client.execute("DELETE FROM videos WHERE hash = $1", &[&common::TEST_VIDEO_HASH]).await.ok();
}

/// Client sends created_at in video multipart — expect it applied to DB.
/// Uses an unparseable filename so filename-date extraction yields nothing
/// and the client date 2020-05-10T12:30:00Z is applied.
#[actix_web::test]
#[serial]
async fn test_upload_video_client_date_applied() {
    common::init_log();
    let (pool, _test_db) = setup_test_database_with_instance().await;

    let config = common::utils::create_test_config();
    let main_pool = common::utils::wrap_main_pool(pool.clone());
    let geotagging_pool = common::utils::create_geotagging_pool().await;

    let app = test::init_service(
        App::new()
            .app_data(web::Data::new(main_pool.clone()))
            .app_data(web::Data::new(geotagging_pool.clone()))
            .app_data(web::Data::new(config.clone()))
            .service(upload_video)
    ).await;

    let token = common::utils::create_test_jwt_token().await;
    let video_bytes = fs::read("tests/test_video.mp4").unwrap();
    let thumbnail_bytes = fs::read("tests/test_thumbnail.jpg").unwrap();

    // Use a non-parseable name so filename-date extraction produces nothing
    let (form, content_type) = common::multipart_builder::create_video_multipart_payload_with_created_at(
        common::TEST_VIDEO_HASH, "video_backup.mp4", &video_bytes, &thumbnail_bytes, "2020-05-10T12:30:00Z",
    );

    let req = test::TestRequest::post()
        .uri("/upload/video")
        .insert_header(("Authorization", format!("Bearer {}", token)))
        .insert_header(("Content-Type", content_type))
        .set_payload(form)
        .to_request();

    let response = test::call_service(&app, req).await;
    assert_eq!(response.status(), http::StatusCode::CREATED);

    let expected: DateTime<Utc> = Utc.with_ymd_and_hms(2020, 5, 10, 12, 30, 0).unwrap();

    let db = pool.get().await.expect("Failed to get client");
    let row = db.query_one("SELECT created_at FROM videos WHERE hash = $1", &[&common::TEST_VIDEO_HASH])
        .await.expect("Failed to query");
    let created_at: DateTime<Utc> = row.get(0);
    assert_eq!(created_at, expected, "Client-supplied date should win over filename, got {}", created_at);

    let subdir = &common::TEST_VIDEO_HASH[..2];
    fs::remove_file(Path::new(common::TEST_VIDEOS_DIR).join(subdir).join(format!("{}.mp4", common::TEST_VIDEO_HASH))).ok();
    db.execute("DELETE FROM videos WHERE hash = $1", &[&common::TEST_VIDEO_HASH]).await.ok();
}

/// Video upload without client created_at — filename VID_20250614_224725 should set the date.
/// TEST_VIDEO_NAME = ".../VID_20250614_224725.mp4" → 2025-06-14 22:47:25 UTC
#[actix_web::test]
#[serial]
async fn test_upload_video_filename_date_applied() {
    common::init_log();
    let (pool, _test_db) = setup_test_database_with_instance().await;

    let config = common::utils::create_test_config();
    let main_pool = common::utils::wrap_main_pool(pool.clone());
    let geotagging_pool = common::utils::create_geotagging_pool().await;

    let app = test::init_service(
        App::new()
            .app_data(web::Data::new(main_pool.clone()))
            .app_data(web::Data::new(geotagging_pool.clone()))
            .app_data(web::Data::new(config.clone()))
            .service(upload_video)
    ).await;

    let token = common::utils::create_test_jwt_token().await;
    let video_bytes = fs::read("tests/test_video.mp4").unwrap();
    let thumbnail_bytes = fs::read("tests/test_thumbnail.jpg").unwrap();

    let (form, content_type) = common::multipart_builder::create_video_multipart_payload(
        common::TEST_VIDEO_HASH, common::TEST_VIDEO_NAME, &video_bytes, &thumbnail_bytes,
    );

    let req = test::TestRequest::post()
        .uri("/upload/video")
        .insert_header(("Authorization", format!("Bearer {}", token)))
        .insert_header(("Content-Type", content_type))
        .set_payload(form)
        .to_request();

    let response = test::call_service(&app, req).await;
    assert_eq!(response.status(), http::StatusCode::CREATED);

    // VID_20250614_224725 → 2025-06-14 22:47:25 UTC
    let expected: DateTime<Utc> = Utc.with_ymd_and_hms(2025, 6, 14, 22, 47, 25).unwrap();

    let db = pool.get().await.expect("Failed to get client");
    let row = db.query_one("SELECT created_at FROM videos WHERE hash = $1", &[&common::TEST_VIDEO_HASH])
        .await.expect("Failed to query");
    let created_at: DateTime<Utc> = row.get(0);
    assert_eq!(created_at, expected, "Filename date should be applied, got {}", created_at);

    let subdir = &common::TEST_VIDEO_HASH[..2];
    fs::remove_file(Path::new(common::TEST_VIDEOS_DIR).join(subdir).join(format!("{}.mp4", common::TEST_VIDEO_HASH))).ok();
    db.execute("DELETE FROM videos WHERE hash = $1", &[&common::TEST_VIDEO_HASH]).await.ok();
}

#[actix_web::test]
#[serial]
async fn test_upload_video_no_auth() {
    let (pool, _test_db) = setup_test_database_with_instance().await;
    let config = common::utils::create_test_config();

    let main_pool = common::utils::wrap_main_pool(pool.clone());
    let geotagging_pool = common::utils::create_geotagging_pool().await;

    let app = test::init_service(
        App::new()
            .app_data(web::Data::new(main_pool))
            .app_data(web::Data::new(geotagging_pool))
            .app_data(web::Data::new(config.clone()))
            .service(upload_video)
    ).await;

    let video_bytes = fs::read("tests/test_video.mp4").unwrap();
    let thumbnail_bytes = fs::read("tests/test_thumbnail.jpg").unwrap();

    let (form, content_type) = common::multipart_builder::create_video_multipart_payload(
        common::TEST_VIDEO_HASH,
        common::TEST_VIDEO_NAME,
        &video_bytes,
        &thumbnail_bytes
    );

    let req = test::TestRequest::post()
        .uri("/upload/video")
        .insert_header(("Content-Type", content_type))
        .set_payload(form)
        .to_request();

    let response = test::call_service(&app, req).await;
    assert_eq!(response.status(), http::StatusCode::UNAUTHORIZED);
}

#[actix_web::test]
#[serial]
async fn test_upload_video_invalid_token() {
    let (pool, _test_db) = setup_test_database_with_instance().await;
    let config = common::utils::create_test_config();

    let main_pool = common::utils::wrap_main_pool(pool.clone());
    let geotagging_pool = common::utils::create_geotagging_pool().await;

    let app = test::init_service(
        App::new()
            .app_data(web::Data::new(main_pool))
            .app_data(web::Data::new(geotagging_pool))
            .app_data(web::Data::new(config.clone()))
            .service(upload_video)
    ).await;

    let video_bytes = fs::read("tests/test_video.mp4").unwrap();
    let thumbnail_bytes = fs::read("tests/test_thumbnail.jpg").unwrap();

    let (form, content_type) = common::multipart_builder::create_video_multipart_payload(
        common::TEST_VIDEO_HASH,
        common::TEST_VIDEO_NAME,
        &video_bytes,
        &thumbnail_bytes
    );

    let req = test::TestRequest::post()
        .uri("/upload/video")
        .insert_header(("Authorization", "Bearer invalid_token"))
        .insert_header(("Content-Type", content_type))
        .set_payload(form)
        .to_request();

    let response = test::call_service(&app, req).await;
    assert_eq!(response.status(), http::StatusCode::UNAUTHORIZED);
}
