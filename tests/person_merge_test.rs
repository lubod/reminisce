use actix_web::{ http, test, web, App };
use pgvector::Vector;
use reminisce::*;
use reminisce::test_utils::setup_test_database_with_instance;
use serial_test::serial;

mod common;

const TEST_USER: &str = "550e8400-e29b-41d4-a716-446655440000";

fn user_uuid() -> uuid::Uuid {
    uuid::Uuid::parse_str(TEST_USER).unwrap()
}

macro_rules! authed {
    ($method:ident, $app:expr, $uri:expr, $token:expr, $json:expr) => {{
        let req = test::TestRequest::$method()
            .uri($uri)
            .insert_header(("Authorization", format!("Bearer {}", $token)))
            .set_json($json)
            .to_request();
        test::call_service($app, req)
    }};
}

async fn seed_image(pool: &deadpool_postgres::Pool, hash: &str) {
    let client = pool.get().await.unwrap();
    client
        .execute(
            "INSERT INTO images (user_id, deviceid, hash, name, ext, has_thumbnail) VALUES ($1, $2, $3, $4, $5, true)",
            &[&user_uuid(), &"dev", &hash, &format!("{}.jpg", hash), &"jpg"],
        )
        .await
        .expect("seed image failed");
}

async fn seed_person(pool: &deadpool_postgres::Pool) -> i64 {
    let client = pool.get().await.unwrap();
    client
        .query_one(
            "INSERT INTO persons (user_id, name, face_count) VALUES ($1, 'P', 0) RETURNING id",
            &[&user_uuid()],
        )
        .await
        .unwrap()
        .get(0)
}

async fn seed_face(pool: &deadpool_postgres::Pool, image_hash: &str, person_id: i64) -> i64 {
    let client = pool.get().await.unwrap();
    let emb = Vector::from(vec![0.1f32; 512]);
    client
        .query_one(
            "INSERT INTO faces (image_hash, image_user_id, user_id, bbox_x, bbox_y, bbox_width, bbox_height, embedding, confidence, person_id) \
             VALUES ($1, $2, $3, 0, 0, 100, 100, $4, 0.99, $5) RETURNING id",
            &[&image_hash, &user_uuid(), &user_uuid(), &emb, &person_id],
        )
        .await
        .unwrap()
        .get(0)
}

async fn seed_face_into_person(pool: &deadpool_postgres::Pool, image_hash: &str, person_id: i64) -> i64 {
    let face = seed_face(pool, image_hash, person_id).await;
    let client = pool.get().await.unwrap();
    client
        .execute("UPDATE persons SET face_count = (SELECT COUNT(*) FROM faces WHERE person_id = $1) WHERE id = $1", &[&person_id])
        .await
        .unwrap();
    face
}

#[actix_web::test]
#[serial]
async fn test_representative_face_and_merge() {
    let (pool, _test_db) = setup_test_database_with_instance().await;
    seed_image(&pool, "img1").await;
    let p1 = seed_person(&pool).await;
    let p2 = seed_person(&pool).await;
    let f1 = seed_face_into_person(&pool, "img1", p1).await;
    let _f2 = seed_face_into_person(&pool, "img1", p2).await;

    let config = common::utils::create_test_config();
    let main_pool = common::utils::wrap_main_pool(pool.clone());
    let app = test::init_service(
        App::new()
            .app_data(web::Data::new(main_pool))
            .app_data(web::Data::new(config.clone()))
            .service(services::person::set_representative_face)
            .service(services::person::merge_persons)
    ).await;
    let token = common::utils::create_test_jwt_token().await;

    // Set representative face on p1.
    let uri = format!("/persons/{}/representative_face", p1);
    let resp = authed!(put, &app, &uri, &token, serde_json::json!({ "face_id": f1 })).await;
    assert_eq!(resp.status(), http::StatusCode::OK);
    let body: serde_json::Value = test::read_body_json(resp).await;
    assert_eq!(body["success"], true);

    // A face that belongs to a different person must 404.
    let uri = format!("/persons/{}/representative_face", p2);
    let resp = authed!(put, &app, &uri, &token, serde_json::json!({ "face_id": f1 })).await;
    assert_eq!(resp.status(), http::StatusCode::NOT_FOUND);

    // Merge p2 into p1.
    let resp = authed!(
        post,
        &app,
        "/persons/merge",
        &token,
        serde_json::json!({ "source_person_ids": [p2], "target_person_id": p1 })
    ).await;
    assert_eq!(resp.status(), http::StatusCode::OK, "merge must succeed");

    // p2 must be gone; its face moved to p1; p1 face_count == 2 and has a rep.
    let client = pool.get().await.unwrap();
    let p2_count: i64 = client
        .query_one("SELECT COUNT(*) FROM persons WHERE id = $1", &[&p2])
        .await
        .unwrap()
        .get(0);
    assert_eq!(p2_count, 0, "source person deleted");

    let p1_faces: i64 = client
        .query_one("SELECT face_count::bigint FROM persons WHERE id = $1", &[&p1])
        .await
        .unwrap()
        .get(0);
    assert_eq!(p1_faces, 2, "target absorbed both faces");

    let orphaned: i64 = client
        .query_one("SELECT COUNT(*) FROM faces WHERE person_id = $1", &[&p1])
        .await
        .unwrap()
        .get(0);
    assert_eq!(orphaned, 2, "both faces point at target");

    let rep: Option<i64> = client
        .query_one("SELECT representative_face_id FROM persons WHERE id = $1", &[&p1])
        .await
        .unwrap()
        .get(0);
    assert!(rep.is_some(), "merge picked a representative face");
}

#[actix_web::test]
#[serial]
async fn test_merge_validation_errors() {
    let (pool, _test_db) = setup_test_database_with_instance().await;
    seed_image(&pool, "img1").await;
    let p1 = seed_person(&pool).await;
    let p2 = seed_person(&pool).await;

    let config = common::utils::create_test_config();
    let main_pool = common::utils::wrap_main_pool(pool.clone());
    let app = test::init_service(
        App::new()
            .app_data(web::Data::new(main_pool))
            .app_data(web::Data::new(config.clone()))
            .service(services::person::merge_persons)
    ).await;
    let token = common::utils::create_test_jwt_token().await;

    // Empty source list.
    let resp = authed!(post, &app, "/persons/merge", &token, serde_json::json!({ "source_person_ids": [], "target_person_id": p1 })).await;
    assert_eq!(resp.status(), http::StatusCode::BAD_REQUEST);

    // Source == target.
    let resp = authed!(post, &app, "/persons/merge", &token, serde_json::json!({ "source_person_ids": [p1], "target_person_id": p1 })).await;
    assert_eq!(resp.status(), http::StatusCode::BAD_REQUEST);

    // Nonexistent person.
    let resp = authed!(post, &app, "/persons/merge", &token, serde_json::json!({ "source_person_ids": [999999, p2], "target_person_id": p1 })).await;
    assert_eq!(resp.status(), http::StatusCode::BAD_REQUEST);

    // Unauthenticated.
    let req = test::TestRequest::post()
        .uri("/persons/merge")
        .set_json(serde_json::json!({ "source_person_ids": [p2], "target_person_id": p1 }))
        .to_request();
    let resp = test::call_service(&app, req).await;
    assert_eq!(resp.status(), http::StatusCode::UNAUTHORIZED);
}
