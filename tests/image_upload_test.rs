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
async fn test_upload_image() {
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
            .service(check_image_exists)
            .service(upload_image)
            .service(list_image_thumbnails)
            .service(get_thumbnail)
            .service(get_image)
    ).await;

    let token = common::utils::create_test_jwt_token().await;

    let image_bytes = fs::read("tests/test_image.jpg").unwrap();
    let thumbnail_bytes = fs::read("tests/test_thumbnail.jpg").unwrap();

    let (form, content_type) = common::multipart_builder::create_multipart_payload(
        common::TEST_IMAGE_HASH,
        common::TEST_IMAGE_NAME,
        &image_bytes,
        &thumbnail_bytes
    );

    let req = test::TestRequest
        ::post()
        .uri("/upload/image")
        .insert_header(("Authorization", format!("Bearer {}", token)))
        .insert_header(("Content-Type", content_type))
        .set_payload(form)
        .to_request();

    let response = test::call_service(&app, req).await;

    let status = response.status();
    assert_eq!(status, http::StatusCode::CREATED);

    // Verify the file was saved
    let subdir = &common::TEST_IMAGE_HASH[..2];
    let file_path = Path::new(common::TEST_UPLOAD_DIR)
        .join(subdir)
        .join(format!("{}.jpg", common::TEST_IMAGE_HASH));
    assert!(file_path.exists());

    // Verify the database entry
    let client = pool.get().await.expect("Failed to get client from pool");
    let row = client
        .query_one("SELECT ext FROM images WHERE hash = $1", &[&common::TEST_IMAGE_HASH]).await
        .expect("Failed to query database");
    let ext: &str = row.get(0);
    assert_eq!(ext, "jpg");

    // Clean up the created file
    fs::remove_file(&file_path).unwrap();

    // Clean up the database entry
    client
        .execute("DELETE FROM images WHERE hash = $1", &[&common::TEST_IMAGE_HASH]).await
        .expect("Failed to clean up database");
}

#[actix_web::test]
#[serial]
async fn test_upload_image2() {
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
            .service(check_image_exists)
            .service(upload_image)
            .service(list_image_thumbnails)
            .service(get_thumbnail)
            .service(get_image)
    ).await;

    let token = common::utils::create_test_jwt_token().await;

    let image_bytes = fs::read("tests/test_image2.jpg").unwrap();
    let thumbnail_bytes = fs::read("tests/test_thumbnail.jpg").unwrap();

    let (form, content_type) = common::multipart_builder::create_multipart_payload(
        common::TEST_IMAGE_HASH2,
        common::TEST_IMAGE_NAME2,
        &image_bytes,
        &thumbnail_bytes
    );

    let req = test::TestRequest
        ::post()
        .uri("/upload/image")
        .insert_header(("Authorization", format!("Bearer {}", token)))
        .insert_header(("Content-Type", content_type))
        .set_payload(form)
        .to_request();

    let response = test::call_service(&app, req).await;

    let status = response.status();
    assert_eq!(status, http::StatusCode::CREATED);

    // Verify the file was saved
    let subdir = &common::TEST_IMAGE_HASH2[..2];
    let file_path = Path::new(common::TEST_UPLOAD_DIR)
        .join(subdir)
        .join(format!("{}.jpg", common::TEST_IMAGE_HASH2));
    assert!(file_path.exists());

    // Verify the database entry
    let client = pool.get().await.expect("Failed to get client from pool");
    let row = client
        .query_one("SELECT ext FROM images WHERE hash = $1", &[&common::TEST_IMAGE_HASH2]).await
        .expect("Failed to query database");
    let ext: &str = row.get(0);
    assert_eq!(ext, "jpg");

    // Clean up the created file
    fs::remove_file(&file_path).unwrap();

    // Clean up the database entry
    client
        .execute("DELETE FROM images WHERE hash = $1", &[&common::TEST_IMAGE_HASH2]).await
        .expect("Failed to clean up database");
}

#[actix_web::test]
#[serial]
async fn test_upload_image_without_thumbnail() {
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
            .service(upload_image)
    ).await;

    let token = common::utils::create_test_jwt_token().await;

    let image_bytes = fs::read("tests/test_image.jpg").unwrap();

    let (form, content_type) = common::multipart_builder::create_multipart_payload_without_thumbnail(
        common::TEST_IMAGE_HASH,
        common::TEST_IMAGE_NAME,
        &image_bytes
    );

    let req = test::TestRequest
        ::post()
        .uri("/upload/image")
        .insert_header(("Authorization", format!("Bearer {}", token)))
        .insert_header(("Content-Type", content_type))
        .set_payload(form)
        .to_request();

    let response = test::call_service(&app, req).await;

    let status = response.status();
    // The server should return a Created (201) status and generate the thumbnail server-side
    assert_eq!(status, http::StatusCode::CREATED);

    // Verify the file was saved
    let subdir = &common::TEST_IMAGE_HASH[..2];
    let file_path = Path::new(common::TEST_UPLOAD_DIR)
        .join(subdir)
        .join(format!("{}.jpg", common::TEST_IMAGE_HASH));
    
    assert!(file_path.exists());

    // Clean up
    fs::remove_file(&file_path).unwrap();
    
    let client = pool.get().await.expect("Failed to get client");
    client.execute("DELETE FROM images WHERE hash = $1", &[&common::TEST_IMAGE_HASH]).await.ok();
}

/// Verify that when an image with EXIF DateTimeOriginal is uploaded,
/// the server sets created_at to the EXIF date rather than the upload time.
/// test_image.jpg has DateTimeOriginal = 2023:12:22 19:12:41
#[actix_web::test]
#[serial]
async fn test_upload_image_exif_date_applied() {
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
            .service(upload_image)
    ).await;

    let token = common::utils::create_test_jwt_token().await;
    let image_bytes = fs::read("tests/test_image.jpg").unwrap();
    let thumbnail_bytes = fs::read("tests/test_thumbnail.jpg").unwrap();

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

    // EXIF DateTimeOriginal in test_image.jpg is 2023:12:22 19:12:41
    let expected: DateTime<Utc> = Utc.with_ymd_and_hms(2023, 12, 22, 19, 12, 41).unwrap();

    let client = pool.get().await.expect("Failed to get client from pool");
    let row = client
        .query_one(
            "SELECT created_at FROM images WHERE hash = $1",
            &[&common::TEST_IMAGE_HASH],
        )
        .await
        .expect("Failed to query database");

    let created_at: DateTime<Utc> = row.get(0);

    // Should match EXIF date, not upload time (within 1 second tolerance for timezone handling)
    let diff = (created_at - expected).num_seconds().abs();
    assert!(
        diff <= 1,
        "created_at={} expected EXIF date={} (diff={} sec)",
        created_at,
        expected,
        diff
    );

    // Cleanup
    let subdir = &common::TEST_IMAGE_HASH[..2];
    let file_path = Path::new(common::TEST_UPLOAD_DIR)
        .join(subdir)
        .join(format!("{}.jpg", common::TEST_IMAGE_HASH));
    fs::remove_file(&file_path).ok();
    client.execute("DELETE FROM images WHERE hash = $1", &[&common::TEST_IMAGE_HASH]).await.ok();
}

/// Verify that when an image WITHOUT EXIF is uploaded with a WhatsApp-style filename
/// (IMG-YYYYMMDD-WAXXXX), created_at is set from the filename date.
/// tests/IMG-20210615-WA0000.jpg has no EXIF tags; expected date: 2021-06-15.
#[actix_web::test]
#[serial]
async fn test_upload_image_filename_date_applied() {
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
            .service(upload_image)
    ).await;

    let token = common::utils::create_test_jwt_token().await;
    let image_bytes = fs::read("tests/IMG-20210615-WA0000.jpg").unwrap();
    let thumbnail_bytes = fs::read("tests/test_thumbnail.jpg").unwrap();

    // Compute hash dynamically so we don't need to hardcode it
    let hash = blake3::hash(&image_bytes).to_hex().to_string();
    let name = "IMG-20210615-WA0000.jpg";

    let (form, content_type) = common::multipart_builder::create_multipart_payload(
        &hash, name, &image_bytes, &thumbnail_bytes,
    );

    let req = test::TestRequest::post()
        .uri("/upload/image")
        .insert_header(("Authorization", format!("Bearer {}", token)))
        .insert_header(("Content-Type", content_type))
        .set_payload(form)
        .to_request();

    let response = test::call_service(&app, req).await;
    assert_eq!(response.status(), http::StatusCode::CREATED);

    // parse_date_from_image_name("IMG-20210615-WA0000.jpg") → 2021-06-15 00:00:00 UTC
    let expected: DateTime<Utc> = Utc.with_ymd_and_hms(2021, 6, 15, 0, 0, 0).unwrap();

    let client = pool.get().await.expect("Failed to get client from pool");
    let row = client
        .query_one("SELECT created_at FROM images WHERE hash = $1", &[&hash])
        .await
        .expect("Failed to query database");

    let created_at: DateTime<Utc> = row.get(0);
    assert_eq!(
        created_at.date_naive(),
        expected.date_naive(),
        "created_at date should come from filename, got {}",
        created_at
    );

    // Cleanup
    let subdir = &hash[..2];
    let file_path = Path::new(common::TEST_UPLOAD_DIR)
        .join(subdir)
        .join(format!("{}.jpg", hash));
    fs::remove_file(&file_path).ok();
    client.execute("DELETE FROM images WHERE hash = $1", &[&hash]).await.ok();
}

/// Client sends created_at in the multipart form (Android DATE_TAKEN path).
/// Image has no EXIF and an unparseable filename — client date should be applied.
/// File: IMG-20210615-WA0000.jpg bytes, sent with filename "photo_backup.jpg" (no date pattern)
/// Client date: 2019-03-15T10:00:00Z → should win (no EXIF, no filename date)
#[actix_web::test]
#[serial]
async fn test_upload_image_client_date_applied() {
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
            .service(upload_image)
    ).await;

    let token = common::utils::create_test_jwt_token().await;
    let image_bytes = fs::read("tests/IMG-20210615-WA0000.jpg").unwrap();
    let thumbnail_bytes = fs::read("tests/test_thumbnail.jpg").unwrap();
    let hash = blake3::hash(&image_bytes).to_hex().to_string();

    // Use a non-parseable filename so filename-date extraction yields nothing
    let client_date = "2019-03-15T10:00:00Z";
    let (form, content_type) = common::multipart_builder::create_multipart_payload_with_created_at(
        &hash, "photo_backup.jpg", &image_bytes, &thumbnail_bytes, client_date,
    );

    let req = test::TestRequest::post()
        .uri("/upload/image")
        .insert_header(("Authorization", format!("Bearer {}", token)))
        .insert_header(("Content-Type", content_type))
        .set_payload(form)
        .to_request();

    let response = test::call_service(&app, req).await;
    assert_eq!(response.status(), http::StatusCode::CREATED);

    let expected: DateTime<Utc> = Utc.with_ymd_and_hms(2019, 3, 15, 10, 0, 0).unwrap();

    let db = pool.get().await.expect("Failed to get client");
    let row = db.query_one("SELECT created_at FROM images WHERE hash = $1", &[&hash])
        .await.expect("Failed to query");
    let created_at: DateTime<Utc> = row.get(0);
    assert_eq!(created_at, expected, "Client-supplied date should be used, got {}", created_at);

    let subdir = &hash[..2];
    fs::remove_file(Path::new(common::TEST_UPLOAD_DIR).join(subdir).join(format!("{}.jpg", hash))).ok();
    db.execute("DELETE FROM images WHERE hash = $1", &[&hash]).await.ok();
}

/// EXIF DateTimeOriginal beats a client-supplied created_at.
/// File: test_image.jpg (EXIF DateTimeOriginal = 2023-12-22 19:12:41)
/// Client date: 2019-01-01T00:00:00Z → EXIF should win
#[actix_web::test]
#[serial]
async fn test_upload_image_exif_beats_client_date() {
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
            .service(upload_image)
    ).await;

    let token = common::utils::create_test_jwt_token().await;
    let image_bytes = fs::read("tests/test_image.jpg").unwrap();
    let thumbnail_bytes = fs::read("tests/test_thumbnail.jpg").unwrap();

    let (form, content_type) = common::multipart_builder::create_multipart_payload_with_created_at(
        common::TEST_IMAGE_HASH, common::TEST_IMAGE_NAME,
        &image_bytes, &thumbnail_bytes, "2019-01-01T00:00:00Z",
    );

    let req = test::TestRequest::post()
        .uri("/upload/image")
        .insert_header(("Authorization", format!("Bearer {}", token)))
        .insert_header(("Content-Type", content_type))
        .set_payload(form)
        .to_request();

    let response = test::call_service(&app, req).await;
    assert_eq!(response.status(), http::StatusCode::CREATED);

    // EXIF DateTimeOriginal = 2023-12-22 19:12:41, must beat the client's 2019-01-01
    let expected: DateTime<Utc> = Utc.with_ymd_and_hms(2023, 12, 22, 19, 12, 41).unwrap();

    let db = pool.get().await.expect("Failed to get client");
    let row = db.query_one("SELECT created_at FROM images WHERE hash = $1", &[&common::TEST_IMAGE_HASH])
        .await.expect("Failed to query");
    let created_at: DateTime<Utc> = row.get(0);
    let diff = (created_at - expected).num_seconds().abs();
    assert!(diff <= 1, "EXIF date should win over client date, got {} (expected ~{})", created_at, expected);

    let subdir = &common::TEST_IMAGE_HASH[..2];
    fs::remove_file(Path::new(common::TEST_UPLOAD_DIR).join(subdir).join(format!("{}.jpg", common::TEST_IMAGE_HASH))).ok();
    db.execute("DELETE FROM images WHERE hash = $1", &[&common::TEST_IMAGE_HASH]).await.ok();
}

#[actix_web::test]
#[serial]
async fn test_upload_image_no_auth() {
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
            .service(upload_image)
    ).await;

    let req = test::TestRequest::post().uri("/upload/image").to_request();

    let response = test::call_service(&app, req).await;
    assert_eq!(response.status(), http::StatusCode::UNAUTHORIZED);
}

#[actix_web::test]
#[serial]
async fn test_upload_image_invalid_token() {
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
            .service(upload_image)
    ).await;

    let req = test::TestRequest
        ::post()
        .uri("/upload/image")
        .insert_header(("Authorization", "Bearer invalid_token"))
        .to_request();

    let response = test::call_service(&app, req).await;
    assert_eq!(response.status(), http::StatusCode::UNAUTHORIZED);
}
