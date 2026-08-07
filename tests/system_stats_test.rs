use actix_web::{ http, test, web, App };
use reminisce::*;
use reminisce::test_utils::setup_test_database_with_instance;
use serial_test::serial;
use std::sync::{ Arc, Mutex };
use sysinfo::SystemExt;

mod common;

macro_rules! authed_get {
    ($app:expr, $uri:expr, $token:expr) => {{
        let req = test::TestRequest::get()
            .uri($uri)
            .insert_header(("Authorization", format!("Bearer {}", $token)))
            .to_request();
        test::call_service($app, req)
    }};
}

#[actix_web::test]
#[serial]
async fn test_pool_stats() {
    let (pool, _test_db) = setup_test_database_with_instance().await;
    let config = common::utils::create_test_config();
    let config = web::Data::new(config);
    let app = test::init_service(
        App::new()
            .app_data(web::Data::new(common::utils::wrap_main_pool(pool)))
            .app_data(config.clone())
            .service(services::pool_stats::get_pool_stats)
    ).await;
    let token = common::utils::create_test_jwt_token().await;

    let resp = authed_get!(&app, "/pool-stats", &token).await;
    assert_eq!(resp.status(), http::StatusCode::OK);
    let body: serde_json::Value = test::read_body_json(resp).await;
    assert!(body.is_object(), "pool stats is an object: {}", body);

    // Admin-only: unauthenticated request is rejected.
    let req = test::TestRequest::get().uri("/pool-stats").to_request();
    let resp = test::call_service(&app, req).await;
    assert_eq!(resp.status(), http::StatusCode::UNAUTHORIZED);
}

#[actix_web::test]
#[serial]
async fn test_system_stats() {
    let (pool, _test_db) = setup_test_database_with_instance().await;
    let config = common::utils::create_test_config();
    let shared_sys: services::system_stats::SharedSystem =
        Arc::new(Mutex::new(sysinfo::System::new_all()));
    let app = test::init_service(
        App::new()
            .app_data(web::Data::new(common::utils::wrap_main_pool(pool)))
            .app_data(web::Data::new(config.clone()))
            .app_data(web::Data::new(shared_sys))
            .service(services::system_stats::get_system_stats)
    ).await;
    let token = common::utils::create_test_jwt_token().await;

    let resp = authed_get!(&app, "/system-stats", &token).await;
    assert_eq!(resp.status(), http::StatusCode::OK);
    let body: serde_json::Value = test::read_body_json(resp).await;
    assert!(body["cpu_usage_percent"].is_number(), "cpu present: {}", body);
    assert!(body["memory_total_gb"].as_f64().unwrap_or(0.0) > 0.0, "memory total present");
    assert!(body["uptime_seconds"].is_number(), "uptime present");
}

#[actix_web::test]
#[serial]
async fn test_geodb_stats() {
    let (pool, _test_db) = setup_test_database_with_instance().await;
    let config = common::utils::create_test_config();
    let geotagging_pool = common::utils::create_geotagging_pool().await;
    let app = test::init_service(
        App::new()
            .app_data(web::Data::new(common::utils::wrap_main_pool(pool)))
            .app_data(web::Data::new(geotagging_pool))
            .app_data(web::Data::new(config.clone()))
            .service(services::geodb_stats::get_geodb_stats)
    ).await;
    let token = common::utils::create_test_jwt_token().await;

    let resp = authed_get!(&app, "/geodb-stats", &token).await;
    assert_eq!(resp.status(), http::StatusCode::OK);
    let body: serde_json::Value = test::read_body_json(resp).await;
    assert!(
        body["total_boundaries"].is_number() || body["countries"].is_number(),
        "geo db stats have counts: {}",
        body
    );

    // Non-admin users are forbidden.
    let req = test::TestRequest::get().uri("/geodb-stats").to_request();
    let resp = test::call_service(&app, req).await;
    assert_eq!(resp.status(), http::StatusCode::UNAUTHORIZED);
}
