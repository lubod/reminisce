use actix_web::{http, test, web, App};
use reminisce::*;
use reminisce::test_utils::setup_test_database_with_instance;
use serial_test::serial;

mod common;

/// Test health check endpoint returns healthy status when database is connected
#[actix_web::test]
#[serial]
async fn test_health_check_healthy() {
    common::init_log();
    let (pool, _test_db) = setup_test_database_with_instance().await;
    let main_pool = common::utils::wrap_main_pool(pool.clone());

    let app = test::init_service(
        App::new()
            .app_data(web::Data::new(main_pool))
            .service(health_check)
    ).await;

    let req = test::TestRequest::get()
        .uri("/health")
        .to_request();

    let response = test::call_service(&app, req).await;
    assert_eq!(response.status(), http::StatusCode::OK);

    let body: serde_json::Value = test::read_body_json(response).await;
    assert_eq!(body["status"], "healthy");
    assert_eq!(body["database"], "connected");
    assert!(body["timestamp"].is_string());
}

/// Test health check returns correct structure
#[actix_web::test]
#[serial]
async fn test_health_check_response_structure() {
    common::init_log();
    let (pool, _test_db) = setup_test_database_with_instance().await;
    let main_pool = common::utils::wrap_main_pool(pool.clone());

    let app = test::init_service(
        App::new()
            .app_data(web::Data::new(main_pool))
            .service(health_check)
    ).await;

    let req = test::TestRequest::get()
        .uri("/health")
        .to_request();

    let response = test::call_service(&app, req).await;
    assert_eq!(response.status(), http::StatusCode::OK);

    let body: serde_json::Value = test::read_body_json(response).await;

    // Verify required fields exist
    assert!(body.get("status").is_some(), "Response should have 'status' field");
    assert!(body.get("database").is_some(), "Response should have 'database' field");
    assert!(body.get("timestamp").is_some(), "Response should have 'timestamp' field");

    // Verify timestamp is valid ISO 8601 format
    let timestamp = body["timestamp"].as_str().unwrap();
    assert!(timestamp.contains("T"), "Timestamp should be in ISO 8601 format");
}

/// Test ping endpoint returns OK
#[actix_web::test]
#[serial]
async fn test_ping_endpoint() {
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
            .service(ping)
    ).await;

    let req = test::TestRequest::get()
        .uri("/ping")
        .to_request();

    let response = test::call_service(&app, req).await;
    assert_eq!(response.status(), http::StatusCode::OK);

    let body = test::read_body(response).await;
    assert_eq!(body, "OK");
}
