use actix_web::{ http, test, web, App };
use reminisce::*;
use reminisce::test_utils::setup_test_database_with_instance;
use serial_test::serial;
use std::fs;
use std::path::Path;

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
