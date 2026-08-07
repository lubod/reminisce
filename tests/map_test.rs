use actix_web::{ http, test, web, App };
use reminisce::*;
use reminisce::test_utils::setup_test_database_with_instance;
use serial_test::serial;

mod common;

const TEST_USER: &str = "550e8400-e29b-41d4-a716-446655440000";

fn user_uuid() -> uuid::Uuid {
    uuid::Uuid::parse_str(TEST_USER).unwrap()
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
    let client = pool.get().await.unwrap();
    client
        .execute(
            "INSERT INTO images (user_id, deviceid, hash, name, ext, has_thumbnail, location) \
             VALUES ($1, $2, $3, $4, $5, true, ST_SetSRID(ST_MakePoint($6, $7), 4326))",
            &[&user_uuid(), &device, &hash, &format!("{}.jpg", hash), &"jpg", &lon, &lat],
        )
        .await
        .expect("insert image failed");
    if starred {
        client
            .execute("INSERT INTO starred_images (user_id, hash) VALUES ($1, $2)", &[&user_uuid(), &hash])
            .await
            .unwrap();
    }
    if let Some(lid) = label {
        client
            .execute("INSERT INTO image_labels (image_hash, image_user_id, label_id) VALUES ($1, $2, $3)", &[&hash, &user_uuid(), &lid])
            .await
            .unwrap();
    }
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
    client
        .execute(
            "INSERT INTO images (user_id, deviceid, hash, name, ext, has_thumbnail) VALUES ($1, $2, $3, $4, $5, true)",
            &[&user_uuid(), &"dev_a", &"no_geo", &"no_geo.jpg", &"jpg"],
        )
        .await
        .unwrap();

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
    let req = test::TestRequest::get()
        .uri("/map/media?page=1")
        .insert_header(("Authorization", format!("Bearer {}", token)))
        .to_request();
    let response = test::call_service(&app, req).await;
    assert_eq!(response.status(), http::StatusCode::OK);
    let body: serde_json::Value = test::read_body_json(response).await;
    let points = body["points"].as_array().unwrap().clone();
    assert_eq!(points.len(), 3, "should return only geotagged media: {:?}", body);
    let geo_a = points.iter().find(|p| p["hash"] == "geo_a").expect("geo_a missing");
    assert!((geo_a["lon"].as_f64().unwrap() - 14.42).abs() < 1e-6, "lon mismatch: {}", geo_a["lon"]);
    assert!((geo_a["lat"].as_f64().unwrap() - 50.08).abs() < 1e-6, "lat mismatch: {}", geo_a["lat"]);
    assert_eq!(geo_a["starred"], true);
    assert_eq!(body["total"].as_u64().unwrap(), 3);

    // starred_only
    let req = test::TestRequest::get()
        .uri("/map/media?page=1&starred_only=true")
        .insert_header(("Authorization", format!("Bearer {}", token)))
        .to_request();
    let response = test::call_service(&app, req).await;
    assert_eq!(response.status(), http::StatusCode::OK);
    let body: serde_json::Value = test::read_body_json(response).await;
    let pts = body["points"].as_array().unwrap().clone();
    assert_eq!(pts.len(), 1, "only the starred image");
    assert_eq!(pts[0]["hash"], "geo_a");

    // device filter
    let req = test::TestRequest::get()
        .uri("/map/media?page=1&device_id=dev_b")
        .insert_header(("Authorization", format!("Bearer {}", token)))
        .to_request();
    let response = test::call_service(&app, req).await;
    assert_eq!(response.status(), http::StatusCode::OK);
    let body: serde_json::Value = test::read_body_json(response).await;
    let pts = body["points"].as_array().unwrap().clone();
    assert_eq!(pts.len(), 1);
    assert_eq!(pts[0]["hash"], "geo_c");

    // label filter
    let uri = format!("/map/media?page=1&label_id={}", label_id);
    let req = test::TestRequest::get()
        .uri(&uri)
        .insert_header(("Authorization", format!("Bearer {}", token)))
        .to_request();
    let response = test::call_service(&app, req).await;
    assert_eq!(response.status(), http::StatusCode::OK);
    let body: serde_json::Value = test::read_body_json(response).await;
    let pts = body["points"].as_array().unwrap().clone();
    assert_eq!(pts.len(), 1);
    assert_eq!(pts[0]["hash"], "geo_a");
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
