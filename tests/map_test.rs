use actix_web::{ http, test, web, App };
use reminisce::*;
use reminisce::test_utils::setup_test_database_with_instance;
use serial_test::serial;

mod common;

const TEST_USER: &str = "550e8400-e29b-41d4-a716-446655440000";
// A second, unrelated user — used to prove cross-user isolation of map points.
const OTHER_USER: &str = "6f2c7777-0000-4000-8000-000000000000";

fn user_uuid() -> uuid::Uuid {
    uuid::Uuid::parse_str(TEST_USER).unwrap()
}

fn other_user_uuid() -> uuid::Uuid {
    uuid::Uuid::parse_str(OTHER_USER).unwrap()
}

/// Insert a geotagged image owned by `user` at an explicit time.
#[allow(clippy::too_many_arguments)]
async fn insert_image_at(
    pool: &deadpool_postgres::Pool,
    hash: &str,
    user: uuid::Uuid,
    device: &str,
    lon: f64,
    lat: f64,
    starred: bool,
    label: Option<i32>,
    created_at: &str,
) {
    let client = pool.get().await.unwrap();
    let when: chrono::DateTime<chrono::Utc> = created_at.parse().expect("valid timestamp");
    client
        .execute(
            "INSERT INTO images (user_id, deviceid, hash, name, ext, created_at, has_thumbnail, location) \
             VALUES ($1, $2, $3, $4, $5, $6, true, ST_SetSRID(ST_MakePoint($7, $8), 4326))",
            &[&user, &device, &hash, &format!("{}.jpg", hash), &"jpg", &when, &lon, &lat],
        )
        .await
        .expect("insert image failed");
    if starred {
        client
            .execute("INSERT INTO starred_images (user_id, hash) VALUES ($1, $2)", &[&user, &hash])
            .await
            .unwrap();
    }
    if let Some(lid) = label {
        client
            .execute(
                "INSERT INTO image_labels (image_hash, image_user_id, label_id) VALUES ($1, $2, $3)",
                &[&hash, &user, &lid],
            )
            .await
            .unwrap();
    }
}

async fn insert_image(
    pool: &deadpool_postgres::Pool,
    hash: &str,
    device: &str,
    lon: f64,
    lat: f64,
    starred: bool,
    label: Option<i32>,
) {
    insert_image_at(pool, hash, user_uuid(), device, lon, lat, starred, label, "2024-01-15T10:00:00Z").await;
}

/// Insert a non-geotagged image (must never appear in map points).
async fn insert_no_geo(pool: &deadpool_postgres::Pool, hash: &str, device: &str, user: uuid::Uuid) {
    let client = pool.get().await.unwrap();
    client
        .execute(
            "INSERT INTO images (user_id, deviceid, hash, name, ext, has_thumbnail) VALUES ($1, $2, $3, $4, $5, true)",
            &[&user, &device, &hash, &format!("{}.jpg", hash), &"jpg"],
        )
        .await
        .unwrap();
}

/// Insert a soft-deleted geotagged image (must never appear in map points).
async fn insert_deleted(pool: &deadpool_postgres::Pool, hash: &str, device: &str) {
    let client = pool.get().await.unwrap();
    let when: chrono::DateTime<chrono::Utc> = "2024-03-01T00:00:00Z".parse().unwrap();
    client
        .execute(
            "INSERT INTO images (user_id, deviceid, hash, name, ext, has_thumbnail, location, deleted_at) \
             VALUES ($1, $2, $3, $4, $5, true, ST_SetSRID(ST_MakePoint(11.0, 11.0), 4326), $6)",
            &[&user_uuid(), &device, &hash, &format!("{}.jpg", hash), &"jpg", &when],
        )
        .await
        .unwrap();
}

/// Insert a video (videos carry no location column — must be excluded entirely).
async fn insert_video(pool: &deadpool_postgres::Pool, hash: &str, device: &str) {
    let client = pool.get().await.unwrap();
    client
        .execute(
            "INSERT INTO videos (user_id, deviceid, hash, name, ext, has_thumbnail) VALUES ($1, $2, $3, $4, $5, true)",
            &[&user_uuid(), &device, &hash, &format!("{}.mp4", hash), &"mp4"],
        )
        .await
        .unwrap();
}

/// GET a map/media URL with a bearer token and parse the JSON body,
/// asserting a 200 status. A macro avoids naming the (unnameable) app type.
macro_rules! get_points {
    ($app:expr, $uri:expr, $token:expr) => {{
        let req = test::TestRequest::get()
            .uri($uri)
            .insert_header(("Authorization", format!("Bearer {}", $token)))
            .to_request();
        let response = test::call_service($app, req).await;
        assert_eq!(response.status(), http::StatusCode::OK, "uri {} should be 200", $uri);
        test::read_body_json(response).await
    }};
}

#[actix_web::test]
#[serial]
async fn test_map_points_basic_and_filters() {
    let (pool, _test_db) = setup_test_database_with_instance().await;
    let client = pool.get().await.unwrap();
    let label_id: i32 = client
        .query_one("INSERT INTO labels (user_id, name) VALUES ($1, 'trip') RETURNING id", &[&user_uuid()])
        .await
        .unwrap()
        .get(0);

    // Geotagged: dev_a (starred, label), dev_a plain, dev_b plain
    insert_image(&pool, "geo_a", "dev_a", 14.42, 50.08, true, Some(label_id)).await;
    insert_image(&pool, "geo_b", "dev_a", 13.38, 52.52, false, None).await;
    insert_image(&pool, "geo_c", "dev_b", -0.13, 51.51, false, None).await;
    // Non-geotagged must be excluded
    insert_no_geo(&pool, "no_geo", "dev_a", user_uuid()).await;

    let config = common::utils::create_test_config();
    let main_pool = common::utils::wrap_main_pool(pool.clone());
    let geotagging_pool = common::utils::create_geotagging_pool().await;
    let app = test::init_service(
        App::new()
            .app_data(web::Data::new(main_pool.clone()))
            .app_data(web::Data::new(geotagging_pool.clone()))
            .app_data(web::Data::new(config.clone()))
            .service(services::map::get_map_points)
    ).await;
    let token = common::utils::create_test_jwt_token().await;

    // All: the 3 geotagged images (no_geo excluded).
    let body: serde_json::Value = get_points!(&app, "/map/media?page=1", &token);
    let points = body["points"].as_array().unwrap().clone();
    assert_eq!(points.len(), 3, "should return only geotagged media: {:?}", body);
    let geo_a = points.iter().find(|p| p["hash"] == "geo_a").expect("geo_a missing");
    assert!((geo_a["lon"].as_f64().unwrap() - 14.42).abs() < 1e-6, "lon mismatch: {}", geo_a["lon"]);
    assert!((geo_a["lat"].as_f64().unwrap() - 50.08).abs() < 1e-6, "lat mismatch: {}", geo_a["lat"]);
    assert_eq!(geo_a["starred"], true);
    assert_eq!(geo_a["has_thumbnail"], true);
    assert_eq!(geo_a["device_id"], "dev_a");
    assert_eq!(body["total"].as_u64().unwrap(), 3);

    // starred_only
    let body: serde_json::Value = get_points!(&app, "/map/media?page=1&starred_only=true", &token);
    let pts = body["points"].as_array().unwrap().clone();
    assert_eq!(pts.len(), 1, "only the starred image");
    assert_eq!(pts[0]["hash"], "geo_a");

    // device filter
    let body: serde_json::Value = get_points!(&app, "/map/media?page=1&device_id=dev_b", &token);
    let pts = body["points"].as_array().unwrap().clone();
    assert_eq!(pts.len(), 1);
    assert_eq!(pts[0]["hash"], "geo_c");

    // label filter
    let uri = format!("/map/media?page=1&label_id={}", label_id);
    let body: serde_json::Value = get_points!(&app, &uri, &token);
    let pts = body["points"].as_array().unwrap().clone();
    assert_eq!(pts.len(), 1);
    assert_eq!(pts[0]["hash"], "geo_a");

    // Combined: starred_only + label still yields geo_a only.
    let uri = format!("/map/media?page=1&starred_only=true&label_id={}", label_id);
    let body: serde_json::Value = get_points!(&app, &uri, &token);
    let pts = body["points"].as_array().unwrap().clone();
    assert_eq!(pts.len(), 1);
    assert_eq!(pts[0]["hash"], "geo_a");

    // Combined: device + starred matches only the starred image on that device.
    let body: serde_json::Value = get_points!(&app, "/map/media?page=1&starred_only=true&device_id=dev_a", &token);
    let pts = body["points"].as_array().unwrap().clone();
    assert_eq!(pts.len(), 1);
    assert_eq!(pts[0]["hash"], "geo_a");
}

#[actix_web::test]
#[serial]
async fn test_map_points_ordering_desc() {
    let (pool, _test_db) = setup_test_database_with_instance().await;
    insert_image_at(&pool, "old", user_uuid(), "dev_a", 1.0, 1.0, false, None, "2024-01-01T00:00:00Z").await;
    insert_image_at(&pool, "new", user_uuid(), "dev_a", 2.0, 2.0, false, None, "2024-06-01T00:00:00Z").await;
    insert_image_at(&pool, "mid", user_uuid(), "dev_a", 3.0, 3.0, false, None, "2024-03-01T00:00:00Z").await;

    let config = common::utils::create_test_config();
    let main_pool = common::utils::wrap_main_pool(pool.clone());
    let geotagging_pool = common::utils::create_geotagging_pool().await;
    let app = test::init_service(
        App::new()
            .app_data(web::Data::new(main_pool.clone()))
            .app_data(web::Data::new(geotagging_pool.clone()))
            .app_data(web::Data::new(config.clone()))
            .service(services::map::get_map_points)
    ).await;
    let token = common::utils::create_test_jwt_token().await;
    let body: serde_json::Value = get_points!(&app, "/map/media", &token);
    let pts = body["points"].as_array().unwrap().clone();
    assert_eq!(pts.len(), 3);
    let order: Vec<&str> = pts.iter().map(|p| p["hash"].as_str().unwrap()).collect();
    assert_eq!(order, vec!["new", "mid", "old"], "should sort by created_at DESC: {:?}", order);
}

#[actix_web::test]
#[serial]
async fn test_map_points_pagination_and_clamping() {
    let (pool, _test_db) = setup_test_database_with_instance().await;
    insert_image_at(&pool, "p1", user_uuid(), "dev_a", 1.0, 1.0, false, None, "2024-01-01T00:00:00Z").await;
    insert_image_at(&pool, "p2", user_uuid(), "dev_a", 2.0, 2.0, false, None, "2024-01-02T00:00:00Z").await;
    insert_image_at(&pool, "p3", user_uuid(), "dev_a", 3.0, 3.0, false, None, "2024-01-03T00:00:00Z").await;

    let config = common::utils::create_test_config();
    let main_pool = common::utils::wrap_main_pool(pool.clone());
    let geotagging_pool = common::utils::create_geotagging_pool().await;
    let app = test::init_service(
        App::new()
            .app_data(web::Data::new(main_pool.clone()))
            .app_data(web::Data::new(geotagging_pool.clone()))
            .app_data(web::Data::new(config.clone()))
            .service(services::map::get_map_points)
    ).await;
    let token = common::utils::create_test_jwt_token().await;

    // Page 1 of 2 → newest 2, total reports the full count.
    let body: serde_json::Value = get_points!(&app, "/map/media?page=1&limit=2", &token);
    let pts = body["points"].as_array().unwrap().clone();
    assert_eq!(pts.len(), 2);
    assert_eq!(body["total"].as_u64().unwrap(), 3);
    let order: Vec<&str> = pts.iter().map(|p| p["hash"].as_str().unwrap()).collect();
    assert_eq!(order, vec!["p3", "p2"]);

    // Page 2 of 2 → the remaining oldest point.
    let body: serde_json::Value = get_points!(&app, "/map/media?page=2&limit=2", &token);
    let pts = body["points"].as_array().unwrap().clone();
    assert_eq!(pts.len(), 1);
    assert_eq!(pts[0]["hash"], "p1");

    // Page past the end → empty set, total still correct.
    let body: serde_json::Value = get_points!(&app, "/map/media?page=99&limit=2", &token);
    let pts = body["points"].as_array().unwrap().clone();
    assert_eq!(pts.len(), 0, "out-of-range page must be empty");
    assert_eq!(body["total"].as_u64().unwrap(), 3);

    // Clamping: page=0 is treated as page 1, and limit=0 is clamped up to 1.
    let body: serde_json::Value = get_points!(&app, "/map/media?page=0&limit=0", &token);
    let pts = body["points"].as_array().unwrap().clone();
    assert_eq!(pts.len(), 1, "limit=0 must be clamped to 1: {:?}", body);
    assert_eq!(pts[0]["hash"], "p3");
}

#[actix_web::test]
#[serial]
async fn test_map_points_keyset_cursor_matches_offset_paging() {
    // Keyset pagination (after_created_at + after_hash) must yield the exact same
    // set and order as OFFSET pagination, and be robust to identical created_at
    // values (the hash tiebreaker decides ordering deterministically).
    let (pool, _test_db) = setup_test_database_with_instance().await;
    // Two points share the same created_at to force the hash tiebreaker.
    insert_image_at(&pool, "k1", user_uuid(), "dev_a", 1.0, 1.0, false, None, "2024-05-01T00:00:00Z").await;
    insert_image_at(&pool, "k2", user_uuid(), "dev_a", 2.0, 2.0, false, None, "2024-05-01T00:00:00Z").await;
    insert_image_at(&pool, "k3", user_uuid(), "dev_a", 3.0, 3.0, false, None, "2024-05-02T00:00:00Z").await;

    let config = common::utils::create_test_config();
    let main_pool = common::utils::wrap_main_pool(pool.clone());
    let geotagging_pool = common::utils::create_geotagging_pool().await;
    let app = test::init_service(
        App::new()
            .app_data(web::Data::new(main_pool.clone()))
            .app_data(web::Data::new(geotagging_pool.clone()))
            .app_data(web::Data::new(config.clone()))
            .service(services::map::get_map_points)
    ).await;
    let token = common::utils::create_test_jwt_token().await;

    // Full set from OFFSET paging (limit 100 → one page).
    let body: serde_json::Value = get_points!(&app, "/map/media?page=1&limit=100", &token);
    let offset_order: Vec<String> = body["points"].as_array().unwrap()
        .iter().map(|p| p["hash"].as_str().unwrap().to_string()).collect();
    assert_eq!(offset_order, vec!["k3", "k2", "k1"]); // k3 newest; k2/k1 tie on hash DESC

    // Re-derive the same set via keyset cursor page-by-page (limit 1).
    let mut cursor_collected: Vec<String> = Vec::new();
    let mut after: Option<(String, String)> = None; // (created_at, hash)
    loop {
        let uri = match &after {
            Some((ts, h)) => format!("/map/media?limit=1&after_created_at={}&after_hash={}", ts, h),
            None => "/map/media?limit=1".to_string(),
        };
        let body: serde_json::Value = get_points!(&app, &uri, &token);
        let pts = body["points"].as_array().unwrap();
        if pts.is_empty() { break; }
        let last = pts.last().unwrap();
        cursor_collected.push(last["hash"].as_str().unwrap().to_string());
        after = Some((
            last["created_at"].as_str().unwrap().to_string(),
            last["hash"].as_str().unwrap().to_string(),
        ));
    }

    assert_eq!(cursor_collected, offset_order, "keyset cursor paging must match OFFSET paging order exactly");
}

#[actix_web::test]
#[serial]
async fn test_map_points_date_filters_and_empty() {
    let (pool, _test_db) = setup_test_database_with_instance().await;
    insert_image_at(&pool, "jan", user_uuid(), "dev_a", 1.0, 1.0, false, None, "2024-01-01T00:00:00Z").await;
    insert_image_at(&pool, "feb", user_uuid(), "dev_a", 2.0, 2.0, false, None, "2024-02-01T00:00:00Z").await;

    let config = common::utils::create_test_config();
    let main_pool = common::utils::wrap_main_pool(pool.clone());
    let geotagging_pool = common::utils::create_geotagging_pool().await;
    let app = test::init_service(
        App::new()
            .app_data(web::Data::new(main_pool.clone()))
            .app_data(web::Data::new(geotagging_pool.clone()))
            .app_data(web::Data::new(config.clone()))
            .service(services::map::get_map_points)
    ).await;
    let token = common::utils::create_test_jwt_token().await;

    // start_date excludes January.
    let body: serde_json::Value = get_points!(&app, "/map/media?start_date=2024-01-15", &token);
    let pts = body["points"].as_array().unwrap().clone();
    assert_eq!(pts.len(), 1);
    assert_eq!(pts[0]["hash"], "feb");

    // end_date excludes February (end is exclusive of the +1d boundary).
    let body: serde_json::Value = get_points!(&app, "/map/media?end_date=2024-01-31", &token);
    let pts = body["points"].as_array().unwrap().clone();
    assert_eq!(pts.len(), 1);
    assert_eq!(pts[0]["hash"], "jan");

    // Both bounds applied together select January (end_date is inclusive).
    let body: serde_json::Value = get_points!(&app, "/map/media?start_date=2024-01-01&end_date=2024-01-31", &token);
    let pts = body["points"].as_array().unwrap().clone();
    assert_eq!(pts.len(), 1);
    assert_eq!(pts[0]["hash"], "jan");

    // Combined date + device filters.
    let body: serde_json::Value = get_points!(&app, "/map/media?start_date=2024-01-01&end_date=2024-01-31&device_id=dev_a", &token);
    let pts = body["points"].as_array().unwrap().clone();
    assert_eq!(pts.len(), 1);
    assert_eq!(pts[0]["hash"], "jan");

    // Empty result: a filter combination matching nothing still returns 200.
    let body: serde_json::Value = get_points!(&app, "/map/media?start_date=2099-01-01", &token);
    let pts = body["points"].as_array().unwrap().clone();
    assert_eq!(pts.len(), 0, "no points in 2099");
    assert_eq!(body["total"].as_u64().unwrap(), 0);

    // Malformed dates must be ignored (200 with all points), not 500.
    let body: serde_json::Value = get_points!(&app, "/map/media?start_date=not-a-date&end_date=also-bad", &token);
    let pts = body["points"].as_array().unwrap().clone();
    assert_eq!(pts.len(), 2, "malformed dates are silently ignored");
}

#[actix_web::test]
#[serial]
async fn test_map_points_excludes_videos_deleted_and_other_users() {
    let (pool, _test_db) = setup_test_database_with_instance().await;
    insert_image(&pool, "mine", "dev_a", 1.0, 1.0, false, None).await;
    insert_no_geo(&pool, "nogeo", "dev_a", user_uuid()).await;
    insert_deleted(&pool, "deleted", "dev_a").await;
    insert_video(&pool, "vid_hash", "dev_a").await;
    // Another user's geotagged image must never appear for this user.
    {
        let c = pool.get().await.unwrap();
        c.execute(
            "INSERT INTO users (id, username, email, password_hash, role) VALUES ($1, $2, $3, 'x', 'viewer')",
            &[&other_user_uuid(), &"other-user", &"other@localhost"],
        )
        .await
        .expect("insert other user failed");
    }
    insert_image_at(&pool, "theirs", other_user_uuid(), "dev_b", 9.0, 9.0, false, None, "2024-02-01T00:00:00Z").await;

    let config = common::utils::create_test_config();
    let main_pool = common::utils::wrap_main_pool(pool.clone());
    let geotagging_pool = common::utils::create_geotagging_pool().await;
    let app = test::init_service(
        App::new()
            .app_data(web::Data::new(main_pool.clone()))
            .app_data(web::Data::new(geotagging_pool.clone()))
            .app_data(web::Data::new(config.clone()))
            .service(services::map::get_map_points)
    ).await;
    let token = common::utils::create_test_jwt_token().await;
    let body: serde_json::Value = get_points!(&app, "/map/media", &token);
    let pts = body["points"].as_array().unwrap().clone();
    let hashes: Vec<&str> = pts.iter().map(|p| p["hash"].as_str().unwrap()).collect();
    assert_eq!(pts.len(), 1, "only our single live geotagged image: {:?}", hashes);
    assert_eq!(pts[0]["hash"], "mine");
    assert_eq!(body["total"].as_u64().unwrap(), 1);
}

#[actix_web::test]
#[serial]
async fn test_map_points_requires_auth() {
    let (pool, _test_db) = setup_test_database_with_instance().await;
    let config = common::utils::create_test_config();
    let main_pool = common::utils::wrap_main_pool(pool.clone());
    let geotagging_pool = common::utils::create_geotagging_pool().await;
    let app = test::init_service(
        App::new()
            .app_data(web::Data::new(main_pool.clone()))
            .app_data(web::Data::new(geotagging_pool.clone()))
            .app_data(web::Data::new(config.clone()))
            .service(services::map::get_map_points)
    ).await;

    let req = test::TestRequest::get().uri("/map/media?page=1").to_request();
    let response = test::call_service(&app, req).await;
    assert_eq!(response.status(), http::StatusCode::UNAUTHORIZED);
}

#[actix_web::test]
#[serial]
async fn test_map_points_empty_library_returns_empty_page() {
    let (pool, _test_db) = setup_test_database_with_instance().await;
    let config = common::utils::create_test_config();
    let main_pool = common::utils::wrap_main_pool(pool.clone());
    let geotagging_pool = common::utils::create_geotagging_pool().await;
    let app = test::init_service(
        App::new()
            .app_data(web::Data::new(main_pool.clone()))
            .app_data(web::Data::new(geotagging_pool.clone()))
            .app_data(web::Data::new(config.clone()))
            .service(services::map::get_map_points)
    ).await;
    let token = common::utils::create_test_jwt_token().await;
    let body: serde_json::Value = get_points!(&app, "/map/media", &token);
    let pts = body["points"].as_array().unwrap().clone();
    assert_eq!(pts.len(), 0, "empty library gives an empty page");
    assert_eq!(body["total"].as_u64().unwrap(), 0);
}
