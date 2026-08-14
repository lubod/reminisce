use actix_web::{http, test, web, App};
use reminisce::*;
use reminisce::test_utils::setup_test_database_with_instance;
use serial_test::serial;
use std::sync::{Arc, Mutex};
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
async fn test_observability_logs_and_errors() {
    let (pool, _test_db) = setup_test_database_with_instance().await;
    let config = common::utils::create_test_config();
    let config_data = web::Data::new(config);
    let shared_sys: services::system_stats::SharedSystem =
        Arc::new(Mutex::new(sysinfo::System::new_all()));

    let app = test::init_service(
        App::new()
            .app_data(web::Data::new(common::utils::wrap_main_pool(pool.clone())))
            .app_data(config_data.clone())
            .app_data(web::Data::new(shared_sys.clone()))
            .service(services::observability::get_admin_logs)
            .service(services::observability::get_admin_errors)
            .service(services::observability::get_admin_alerts)
            .service(services::observability::get_admin_pipeline)
            .service(services::observability::get_admin_gpu)
            .service(services::observability::get_admin_ai_models)
    ).await;
    let token = common::utils::create_test_jwt_token().await;

    // 1. Logs endpoint
    let resp = authed_get!(&app, "/admin/logs?level=info&limit=50", &token).await;
    assert_eq!(resp.status(), http::StatusCode::OK);
    let body: serde_json::Value = test::read_body_json(resp).await;
    assert!(body["entries"].is_array());
    assert_eq!(body["source"], "ring");

    // File history logs fallback
    let resp_full = authed_get!(&app, "/admin/logs?full=1&level=debug&limit=10", &token).await;
    assert_eq!(resp_full.status(), http::StatusCode::OK);
    let body_full: serde_json::Value = test::read_body_json(resp_full).await;
    assert_eq!(body_full["source"], "file");

    // 2. Errors endpoint
    let resp_err = authed_get!(&app, "/admin/errors?limit=20", &token).await;
    assert_eq!(resp_err.status(), http::StatusCode::OK);
    let body_err: serde_json::Value = test::read_body_json(resp_err).await;
    assert!(body_err["count_5m"].is_object());
    assert!(body_err["entries"].is_array());

    // 3. Alerts endpoint
    let resp_alerts = authed_get!(&app, "/admin/alerts", &token).await;
    assert_eq!(resp_alerts.status(), http::StatusCode::OK);
    let body_alerts: serde_json::Value = test::read_body_json(resp_alerts).await;
    assert!(body_alerts["alerts"].is_array());
    assert!(body_alerts["alerts"].as_array().unwrap().len() >= 5);

    // 4. Pipeline endpoint
    let resp_pipe = authed_get!(&app, "/admin/pipeline", &token).await;
    assert_eq!(resp_pipe.status(), http::StatusCode::OK);
    let body_pipe: serde_json::Value = test::read_body_json(resp_pipe).await;
    assert!(body_pipe["workers"].is_array());
    assert!(body_pipe["http"].is_object());
    assert!(body_pipe["db_query_ms"].is_object());


    // 5. GPU endpoint
    let resp_gpu = authed_get!(&app, "/admin/gpu", &token).await;
    assert_eq!(resp_gpu.status(), http::StatusCode::OK);

    // 5b. AI Models endpoint
    let resp_ai = authed_get!(&app, "/admin/ai-models", &token).await;
    assert_eq!(resp_ai.status(), http::StatusCode::OK);
    let body_ai: serde_json::Value = test::read_body_json(resp_ai).await;
    assert!(body_ai["models"].is_array());


    // 6. Unauthenticated rejection
    let unauthed = test::TestRequest::get().uri("/admin/logs").to_request();
    let resp_unauthed = test::call_service(&app, unauthed).await;
    assert_eq!(resp_unauthed.status(), http::StatusCode::UNAUTHORIZED);
}

#[actix_web::test]
#[serial]
async fn test_observability_series() {
    let (pool, _test_db) = setup_test_database_with_instance().await;
    let config = common::utils::create_test_config();
    let config_data = web::Data::new(config);

    // Insert sample metrics into metric_samples table
    let client = pool.get().await.unwrap();
    let _ = client.execute(
        "INSERT INTO metric_samples (ts, name, value) VALUES
         (NOW() - INTERVAL '10 minutes', 'system_cpu_percent', 12.5),
         (NOW() - INTERVAL '5 minutes', 'system_cpu_percent', 15.0),
         (NOW() - INTERVAL '2 minutes', 'system_mem_percent', 45.2),
         (NOW() - INTERVAL '1 minute', 'system_disk_free_gb', 250.0),
         (NOW(), 'backup_peers_available', 2.0)",
        &[],
    ).await;

    let app = test::init_service(
        App::new()
            .app_data(web::Data::new(common::utils::wrap_main_pool(pool.clone())))
            .app_data(config_data.clone())
            .service(services::observability::get_admin_series)
    ).await;
    let token = common::utils::create_test_jwt_token().await;

    // Range 1d
    let resp_1d = authed_get!(&app, "/admin/series?range=1d", &token).await;
    assert_eq!(resp_1d.status(), http::StatusCode::OK);
    let body_1d: serde_json::Value = test::read_body_json(resp_1d).await;
    assert_eq!(body_1d["range"], "1d");
    assert!(body_1d["series"].is_array());

    // Range 30d
    let resp_30d = authed_get!(&app, "/admin/series?range=30d", &token).await;
    assert_eq!(resp_30d.status(), http::StatusCode::OK);
    let body_30d: serde_json::Value = test::read_body_json(resp_30d).await;
    assert_eq!(body_30d["range"], "30d");

    // Range 90d
    let resp_90d = authed_get!(&app, "/admin/series?range=90d", &token).await;
    assert_eq!(resp_90d.status(), http::StatusCode::OK);
    let body_90d: serde_json::Value = test::read_body_json(resp_90d).await;
    assert_eq!(body_90d["range"], "90d");
}
