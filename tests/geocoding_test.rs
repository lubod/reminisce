use actix_web::{http, test, web, App};
use reminisce::*;
use reminisce::test_utils::setup_test_database_with_instance;
use serial_test::serial;

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
async fn test_geocoding_endpoint_and_validation() {
    let (pool, _test_db) = setup_test_database_with_instance().await;
    let config = common::utils::create_test_config();
    let config_data = web::Data::new(config);
    let geotagging_pool = web::Data::new(common::utils::create_geotagging_pool().await);

    let app = test::init_service(
        App::new()
            .app_data(web::Data::new(common::utils::wrap_main_pool(pool.clone())))
            .app_data(config_data.clone())
            .app_data(geotagging_pool.clone())
            .service(services::geocoding::search_places)
    ).await;
    let token = common::utils::create_test_jwt_token().await;

    // 1. Query too short (< 2 chars) -> 400 Bad Request
    let resp_short = authed_get!(&app, "/search/places?query=a", &token).await;
    assert_eq!(resp_short.status(), http::StatusCode::BAD_REQUEST);

    // 2. Query valid length -> 200 OK (returns array from geotagging db)
    let resp_valid = authed_get!(&app, "/search/places?query=London&limit=5", &token).await;
    assert_eq!(resp_valid.status(), http::StatusCode::OK);
    let body: serde_json::Value = test::read_body_json(resp_valid).await;
    assert!(body.is_array());

    // 3. Direct function call input validation
    let res_err = services::geocoding::geocode_place_name(" ", &geotagging_pool, 10).await;
    assert!(res_err.is_err(), "empty query should return error");

    // 4. Direct function call with valid name
    let res_ok = services::geocoding::geocode_place_name("Paris", &geotagging_pool, 10).await;
    assert!(res_ok.is_ok(), "valid query against geotagging db should succeed");

    // 5. Unauthenticated rejection
    let unauthed = test::TestRequest::get().uri("/search/places?query=Berlin").to_request();
    let resp_unauthed = test::call_service(&app, unauthed).await;
    assert_eq!(resp_unauthed.status(), http::StatusCode::UNAUTHORIZED);
}
