use actix_web::{ http, test, web, App };
use reminisce::*;
use reminisce::config::Config;
use reminisce::test_utils::setup_test_database_with_instance;
use serial_test::serial;

mod common;

const TEST_USER: &str = "550e8400-e29b-41d4-a716-446655440000";

fn user_uuid() -> uuid::Uuid {
    uuid::Uuid::parse_str(TEST_USER).unwrap()
}

macro_rules! authed_get {
    ($app:expr, $uri:expr, $token:expr) => {{
        let req = test::TestRequest::get()
            .uri($uri)
            .insert_header(("Authorization", format!("Bearer {}", $token)))
            .to_request();
        test::call_service($app, req)
    }};
}

/// Config with an ai_grpc_url that refuses connections instantly so the
/// semantic / hybrid / keyframe paths deterministically return the 500 path.
fn config_with_dead_ai() -> Config {
    let mut c = common::utils::create_test_config();
    c.ai_grpc_url = "http://127.0.0.1:1".to_string();
    c
}

async fn seed_image(pool: &deadpool_postgres::Pool, hash: &str, name: &str, device: &str) {
    let client = pool.get().await.unwrap();
    client
        .execute(
            "INSERT INTO images (user_id, deviceid, hash, name, ext, description, type, has_thumbnail) \
             VALUES ($1, $2, $3, $4, 'jpg', $5, 'camera', true)",
            &[&user_uuid(), &device, &hash, &name, &format!("description of {}", name)],
        )
        .await
        .expect("seed image");
}

async fn seed_video(pool: &deadpool_postgres::Pool, hash: &str, name: &str, device: &str) {
    let client = pool.get().await.unwrap();
    client
        .execute(
            "INSERT INTO videos (user_id, deviceid, hash, name, ext, description, type, has_thumbnail) \
             VALUES ($1, $2, $3, $4, 'mp4', $5, 'camera', true)",
            &[&user_uuid(), &device, &hash, &name, &format!("description of {}", name)],
        )
        .await
        .expect("seed video");
}

async fn seed_image_location(pool: &deadpool_postgres::Pool, hash: &str, lon: f64, lat: f64) {
    let client = pool.get().await.unwrap();
    client
        .execute(
            "INSERT INTO images (user_id, deviceid, hash, name, ext, description, type, has_thumbnail, location) \
             VALUES ($1, 'dev', $2, 'prague bridge', 'jpg', 'a sunny city', 'camera', true, ST_SetSRID(ST_MakePoint($3, $4), 4326))",
            &[&user_uuid(), &hash, &lon, &lat],
        )
        .await
        .expect("seed located image");
}

// Build the minimal app exposing just the search_images handler.
macro_rules! search_app {
    ($pool:expr, $config:expr) => {{
        let main_pool = common::utils::wrap_main_pool($pool);
        test::init_service(
            App::new()
                .app_data(web::Data::new(main_pool))
                .app_data(web::Data::new($config))
                .service(services::embedding::search_images)
        ).await
    }};
}

fn hashes(body: &serde_json::Value) -> Vec<String> {
    body["results"].as_array().unwrap().iter()
        .map(|r| r["hash"].as_str().unwrap().to_string())
        .collect()
}

#[actix_web::test]
#[serial]
async fn test_search_text_returns_matching_images() {
    let (pool, _db) = setup_test_database_with_instance().await;
    let config = common::utils::create_test_config();
    seed_image(&pool, "img_sunset", "golden sunset above hills", "dev").await;
    seed_image(&pool, "img_surf", "someone surfing in the waves", "dev").await;

    let app = search_app!(pool, config);
    let token = common::utils::create_test_jwt_token().await;

    let resp = authed_get!(&app, "/search/images?query=sunset&mode=text", &token).await;
    assert_eq!(resp.status(), http::StatusCode::OK);
    let body: serde_json::Value = test::read_body_json(resp).await;
    let hs = hashes(&body);
    assert!(hs.contains(&"img_sunset".to_string()), "sunset image found: {:?}", hs);
    assert!(!hs.contains(&"img_surf".to_string()), "surf image excluded: {:?}", hs);
}

#[actix_web::test]
#[serial]
async fn test_search_text_media_type_and_filters() {
    let (pool, _db) = setup_test_database_with_instance().await;
    let config = common::utils::create_test_config();
    seed_image(&pool, "img_trip", "memories of a mountain climbing trip", "devA").await;
    seed_video(&pool, "vid_trip", "mountain climbing trip movie", "devB").await;

    let app = search_app!(pool, config);
    let token = common::utils::create_test_jwt_token().await;

    let resp = authed_get!(&app, "/search/images?query=mountain&mode=text&media_type=video", &token).await;
    let body: serde_json::Value = test::read_body_json(resp).await;
    let hs = hashes(&body);
    assert!(hs.contains(&"vid_trip".to_string()) && !hs.contains(&"img_trip".to_string()), "media_type video: {:?}", hs);

    let resp = authed_get!(&app, "/search/images?query=mountain&mode=text&media_type=all&device_id=devA", &token).await;
    let body: serde_json::Value = test::read_body_json(resp).await;
    let hs = hashes(&body);
    assert!(hs.contains(&"img_trip".to_string()) && !hs.contains(&"vid_trip".to_string()), "device filter: {:?}", hs);
}

#[actix_web::test]
#[serial]
async fn test_search_text_location_radius() {
    common::init_log();
    let (pool, _db) = setup_test_database_with_instance().await;
    let config = common::utils::create_test_config();
    seed_image_location(&pool, "img_prague", 14.42, 50.08).await;

    let app = search_app!(pool, config);
    let token = common::utils::create_test_jwt_token().await;

    let resp = authed_get!(&app, "/search/images?query=prague&mode=text&location_lat=50.08&location_lon=14.42&location_radius_km=20", &token).await;
    let st = resp.status();
    let raw = test::read_body(resp).await;
    eprintln!("RADIUS-DIAG status={} body={:?}", st, String::from_utf8_lossy(&raw));
    let body: serde_json::Value = serde_json::from_slice(&raw).unwrap_or(serde_json::Value::Null);
    assert_eq!(st, http::StatusCode::OK, "radius search failed: {}", body);
    assert!(hashes(&body).contains(&"img_prague".to_string()), "radius filter found it: {:?}", body);

    let resp = authed_get!(&app, "/search/images?query=prague&mode=text&location_lat=60.0&location_lon=30.0&location_radius_km=20", &token).await;
    let body: serde_json::Value = test::read_body_json(resp).await;
    assert!(!hashes(&body).contains(&"img_prague".to_string()), "outside radius excluded: {:?}", body);
}

#[actix_web::test]
#[serial]
async fn test_search_validation_and_empty_results() {
    let (pool, _db) = setup_test_database_with_instance().await;
    let config = common::utils::create_test_config();
    let app = search_app!(pool.clone(), config);
    let token = common::utils::create_test_jwt_token().await;

    let resp = authed_get!(&app, "/search/images?query=%20%20&mode=text", &token).await;
    assert_eq!(resp.status(), http::StatusCode::BAD_REQUEST);

    let long: String = "a".repeat(501);
    let resp = authed_get!(&app, &format!("/search/images?query={}&mode=text", long), &token).await;
    assert_eq!(resp.status(), http::StatusCode::BAD_REQUEST);

    let resp = authed_get!(&app, "/search/images?query=x&mode=text&start_date=2024-06-01&end_date=2024-01-01", &token).await;
    assert_eq!(resp.status(), http::StatusCode::OK);
    let body: serde_json::Value = test::read_body_json(resp).await;
    assert_eq!(body["results"].as_array().unwrap().len(), 0);

    let req = test::TestRequest::get().uri("/search/images?query=x&mode=text").to_request();
    let resp = test::call_service(&app, req).await;
    assert_eq!(resp.status(), http::StatusCode::UNAUTHORIZED);
}

#[actix_web::test]
#[serial]
async fn test_search_semantic_hybrid_500_with_dead_ai() {
    let (pool, _db) = setup_test_database_with_instance().await;
    seed_image(&pool, "img_a", "whatever", "dev").await;
    let config = config_with_dead_ai();
    let app = search_app!(pool, config);
    let token = common::utils::create_test_jwt_token().await;

    let resp = authed_get!(&app, "/search/images?query=sunset&mode=semantic", &token).await;
    assert_eq!(resp.status(), http::StatusCode::INTERNAL_SERVER_ERROR);

    let resp = authed_get!(&app, "/search/images?query=sunset&mode=hybrid", &token).await;
    assert_eq!(resp.status(), http::StatusCode::INTERNAL_SERVER_ERROR);
}

#[actix_web::test]
#[serial]
async fn test_search_video_keyframes_validation_and_500() {
    let (pool, _db) = setup_test_database_with_instance().await;
    let config = common::utils::create_test_config();
    let main_pool = common::utils::wrap_main_pool(pool.clone());
    let app = test::init_service(
        App::new()
            .app_data(web::Data::new(main_pool))
            .app_data(web::Data::new(config.clone()))
            .service(services::embedding::search_video_keyframes)
    ).await;
    let token = common::utils::create_test_jwt_token().await;

    let resp = authed_get!(&app, "/search/video-keyframes?query=", &token).await;
    assert_eq!(resp.status(), http::StatusCode::BAD_REQUEST);

    let bad_pool = common::utils::wrap_main_pool(pool);
    let dead = config_with_dead_ai();
    let app2 = test::init_service(
        App::new()
            .app_data(web::Data::new(bad_pool))
            .app_data(web::Data::new(dead))
            .service(services::embedding::search_video_keyframes)
    ).await;
    let resp = authed_get!(&app2, "/search/video-keyframes?query=dog", &token).await;
    assert_eq!(resp.status(), http::StatusCode::INTERNAL_SERVER_ERROR);
}
