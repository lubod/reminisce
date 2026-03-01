use actix_web::{ http, test, web, App };
use reminisce::*;
use reminisce::test_utils::setup_test_database_with_instance;
use chrono;
use serial_test::serial;

mod common;

#[actix_web::test]
#[serial]
async fn test_check_image_exists_found() {
    common::init_log();
    let (pool, _test_db) = setup_test_database_with_instance().await;
    let client = pool.get().await.expect("Failed to get client from pool");

    let config = common::utils::create_test_config();
    // Insert test data
    client
        .execute(
            "INSERT INTO images (hash, name, exif, created_at, type, deviceid, ext) VALUES ($1, $2, $3, $4, $5, $6, $7)",
            &[
                &common::TEST_CHECK_HASH,
                &common::TEST_IMAGE_NAME,
                &None::<&str>,
                &chrono::Utc::now(),
                &"camera",
                &"test_device_id",
                &"jpg",
            ]
        ).await
        .expect("Failed to insert test data");

    // Wrap pools for dependency injection
    let main_pool = common::utils::wrap_main_pool(pool.clone());
    let geotagging_pool = common::utils::create_geotagging_pool().await;

    let app = test::init_service(
        App::new()
            .app_data(web::Data::new(main_pool))
            .app_data(web::Data::new(geotagging_pool))
            .app_data(web::Data::new(config.clone()))
            .service(check_image_exists)
    ).await;

    let token = common::utils::create_test_jwt_token().await;

    let req = test::TestRequest
        ::get()
        .uri(&format!("/check_image_exists?hash={}&device_id=test_device_id", common::TEST_CHECK_HASH))
        .insert_header(("Authorization", format!("Bearer {}", token)))
        .to_request();

    let response = test::call_service(&app, req).await;
    assert_eq!(response.status(), http::StatusCode::OK);

    let body: serde_json::Value = test::read_body_json(response).await;
    assert_eq!(body["exists"], true);

    // Clean up
    client
        .execute("DELETE FROM images WHERE hash = $1", &[&common::TEST_CHECK_HASH]).await
        .expect("Failed to clean up database");
}

#[actix_web::test]
#[serial]
async fn test_check_image_exists_not_found() {
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
    ).await;

    let token = common::utils::create_test_jwt_token().await;

    let req = test::TestRequest
        ::get()
        .uri("/check_image_exists?hash=nonexistent_hash&device_id=test_device_id")
        .insert_header(("Authorization", format!("Bearer {}", token)))
        .to_request();

    let response = test::call_service(&app, req).await;
    assert_eq!(response.status(), http::StatusCode::OK);

    let body: serde_json::Value = test::read_body_json(response).await;
    assert_eq!(body["exists"], false);
}

#[actix_web::test]
#[serial]
async fn test_check_image_exists_invalid_token() {
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
    ).await;

    let req = test::TestRequest
        ::get()
        .uri("/check_image_exists?hash=nonexistent_hash&device_id=test_device_id")
        .insert_header(("Authorization", "Bearer invalid_token"))
        .to_request();

    let response = test::call_service(&app, req).await;
    assert_eq!(response.status(), http::StatusCode::UNAUTHORIZED);
}

#[actix_web::test]
#[serial]
async fn test_check_video_exists_found() {
    let (pool, _test_db) = setup_test_database_with_instance().await;
    let client = pool.get().await.expect("Failed to get client from pool");

    let config = common::utils::create_test_config();
    // Insert test data into videos table
    client
        .execute(
            "INSERT INTO videos (hash, name, metadata, created_at, type, deviceid, ext) VALUES ($1, $2, $3, $4, $5, $6, $7)",
            &[
                &"test_video_hash",
                &"test_video.mp4",
                &None::<&str>,
                &chrono::Utc::now(),
                &"camera",
                &"test_device_id",
                &"mp4",
            ]
        ).await
        .expect("Failed to insert test video data");


    // Wrap pools for dependency injection

    let main_pool = common::utils::wrap_main_pool(pool.clone());

    let geotagging_pool = common::utils::create_geotagging_pool().await;


    let app = test::init_service(
        App::new()
            .app_data(web::Data::new(main_pool.clone()))
            .app_data(web::Data::new(geotagging_pool.clone()))
            .app_data(web::Data::new(config.clone()))
            .service(check_video_exists)
    ).await;

    let token = common::utils::create_test_jwt_token().await;

    let req = test::TestRequest
        ::get()
        .uri(&format!("/check_video_exists?hash={}&device_id=test_device_id", "test_video_hash"))
        .insert_header(("Authorization", format!("Bearer {}", token)))
        .to_request();

    let response = test::call_service(&app, req).await;
    assert_eq!(response.status(), http::StatusCode::OK);

    let body: serde_json::Value = test::read_body_json(response).await;
    assert_eq!(body["exists"], true);

    // Clean up
    client
        .execute("DELETE FROM videos WHERE hash = $1", &[&"test_video_hash"]).await
        .expect("Failed to clean up video database");
}

#[actix_web::test]
#[serial]
async fn test_check_video_exists_not_found() {
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
    ).await;

    let token = common::utils::create_test_jwt_token().await;

    let req = test::TestRequest
        ::get()
        .uri("/check_video_exists?hash=nonexistent_video_hash&device_id=test_device_id")
        .insert_header(("Authorization", format!("Bearer {}", token)))
        .to_request();

    let response = test::call_service(&app, req).await;
    assert_eq!(response.status(), http::StatusCode::OK);

    let body: serde_json::Value = test::read_body_json(response).await;
    assert_eq!(body["exists"], false);
}

#[actix_web::test]
#[serial]
async fn test_check_video_exists_invalid_token() {
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
    ).await;

    let req = test::TestRequest
        ::get()
        .uri("/check_video_exists?hash=nonexistent_hash&device_id=test_device_id")
        .insert_header(("Authorization", "Bearer invalid_token"))
        .to_request();

    let response = test::call_service(&app, req).await;
    assert_eq!(response.status(), http::StatusCode::UNAUTHORIZED);
}
