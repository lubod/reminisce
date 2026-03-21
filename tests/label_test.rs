use actix_web::{http, test, web, App};
use reminisce::test_utils::setup_test_database_with_instance;
use serial_test::serial;

mod common;

const ADMIN_UUID: &str = "550e8400-e29b-41d4-a716-446655440000";

async fn admin_token() -> String {
    common::utils::create_test_jwt_token().await
}

fn build_app_data(
    main_pool: reminisce::db::MainDbPool,
    config: reminisce::config::Config,
) -> actix_web::App<
    impl actix_web::dev::ServiceFactory<
        actix_web::dev::ServiceRequest,
        Config = (),
        Response = actix_web::dev::ServiceResponse,
        Error = actix_web::Error,
        InitError = (),
    >,
> {
    App::new()
        .app_data(web::Data::new(main_pool))
        .app_data(web::Data::new(config))
        .service(reminisce::services::label::get_labels)
        .service(reminisce::services::label::create_label)
        .service(reminisce::services::label::delete_label)
        .service(reminisce::services::label::get_image_labels)
        .service(reminisce::services::label::add_image_label)
        .service(reminisce::services::label::remove_image_label)
        .service(reminisce::services::label::get_video_labels)
        .service(reminisce::services::label::add_video_label)
        .service(reminisce::services::label::remove_video_label)
}

#[actix_web::test]
#[serial]
async fn test_get_labels_empty() {
    common::init_log();
    let (pool, _db) = setup_test_database_with_instance().await;
    let client = pool.get().await.unwrap();
    // Clean up any existing labels for test user
    let user_uuid = uuid::Uuid::parse_str(ADMIN_UUID).unwrap();
    client.execute("DELETE FROM labels WHERE user_id = $1", &[&user_uuid]).await.unwrap();

    let main_pool = common::utils::wrap_main_pool(pool.clone());
    let config = common::utils::create_test_config();
    let app = test::init_service(build_app_data(main_pool, config)).await;

    let token = admin_token().await;
    let req = test::TestRequest::get()
        .uri("/labels")
        .insert_header(("Authorization", format!("Bearer {}", token)))
        .to_request();
    let resp = test::call_service(&app, req).await;
    assert_eq!(resp.status(), http::StatusCode::OK);
    let body: serde_json::Value = test::read_body_json(resp).await;
    assert_eq!(body["labels"].as_array().unwrap().len(), 0);
}

#[actix_web::test]
#[serial]
async fn test_create_label() {
    common::init_log();
    let (pool, _db) = setup_test_database_with_instance().await;
    let client = pool.get().await.unwrap();
    let user_uuid = uuid::Uuid::parse_str(ADMIN_UUID).unwrap();
    client.execute("DELETE FROM labels WHERE user_id = $1", &[&user_uuid]).await.unwrap();

    let main_pool = common::utils::wrap_main_pool(pool.clone());
    let config = common::utils::create_test_config();
    let app = test::init_service(build_app_data(main_pool, config)).await;

    let token = admin_token().await;
    let req = test::TestRequest::post()
        .uri("/labels")
        .insert_header(("Authorization", format!("Bearer {}", token)))
        .set_json(&serde_json::json!({"name": "Vacation", "color": "#FF0000"}))
        .to_request();
    let resp = test::call_service(&app, req).await;
    assert_eq!(resp.status(), http::StatusCode::OK);
    let body: serde_json::Value = test::read_body_json(resp).await;
    assert_eq!(body["name"], "Vacation");
    assert_eq!(body["color"], "#FF0000");
    assert!(body["id"].is_number());
}

#[actix_web::test]
#[serial]
async fn test_create_label_default_color() {
    common::init_log();
    let (pool, _db) = setup_test_database_with_instance().await;
    let client = pool.get().await.unwrap();
    let user_uuid = uuid::Uuid::parse_str(ADMIN_UUID).unwrap();
    client.execute("DELETE FROM labels WHERE user_id = $1 AND name = 'DefaultColor'", &[&user_uuid]).await.unwrap();

    let main_pool = common::utils::wrap_main_pool(pool.clone());
    let config = common::utils::create_test_config();
    let app = test::init_service(build_app_data(main_pool, config)).await;

    let token = admin_token().await;
    let req = test::TestRequest::post()
        .uri("/labels")
        .insert_header(("Authorization", format!("Bearer {}", token)))
        .set_json(&serde_json::json!({"name": "DefaultColor"}))
        .to_request();
    let resp = test::call_service(&app, req).await;
    assert_eq!(resp.status(), http::StatusCode::OK);
    let body: serde_json::Value = test::read_body_json(resp).await;
    assert_eq!(body["color"], "#3B82F6");
}

#[actix_web::test]
#[serial]
async fn test_create_label_upsert_on_duplicate() {
    common::init_log();
    let (pool, _db) = setup_test_database_with_instance().await;
    let client = pool.get().await.unwrap();
    let user_uuid = uuid::Uuid::parse_str(ADMIN_UUID).unwrap();
    client.execute("DELETE FROM labels WHERE user_id = $1 AND name = 'Upsert'", &[&user_uuid]).await.unwrap();

    let main_pool = common::utils::wrap_main_pool(pool.clone());
    let config = common::utils::create_test_config();
    let app = test::init_service(build_app_data(main_pool, config)).await;

    let token = admin_token().await;
    // Create with original color
    let req = test::TestRequest::post()
        .uri("/labels")
        .insert_header(("Authorization", format!("Bearer {}", token.clone())))
        .set_json(&serde_json::json!({"name": "Upsert", "color": "#AAAAAA"}))
        .to_request();
    let resp = test::call_service(&app, req).await;
    let original: serde_json::Value = test::read_body_json(resp).await;
    let original_id = original["id"].as_i64().unwrap();

    // Create again with new color — should upsert
    let req = test::TestRequest::post()
        .uri("/labels")
        .insert_header(("Authorization", format!("Bearer {}", token)))
        .set_json(&serde_json::json!({"name": "Upsert", "color": "#BBBBBB"}))
        .to_request();
    let resp = test::call_service(&app, req).await;
    assert_eq!(resp.status(), http::StatusCode::OK);
    let updated: serde_json::Value = test::read_body_json(resp).await;
    assert_eq!(updated["id"].as_i64().unwrap(), original_id);
    assert_eq!(updated["color"], "#BBBBBB");
}

#[actix_web::test]
#[serial]
async fn test_create_label_empty_name() {
    common::init_log();
    let (pool, _db) = setup_test_database_with_instance().await;
    let main_pool = common::utils::wrap_main_pool(pool.clone());
    let config = common::utils::create_test_config();
    let app = test::init_service(build_app_data(main_pool, config)).await;

    let token = admin_token().await;
    let req = test::TestRequest::post()
        .uri("/labels")
        .insert_header(("Authorization", format!("Bearer {}", token)))
        .set_json(&serde_json::json!({"name": "   "}))
        .to_request();
    let resp = test::call_service(&app, req).await;
    assert_eq!(resp.status(), http::StatusCode::BAD_REQUEST);
}

#[actix_web::test]
#[serial]
async fn test_delete_label() {
    common::init_log();
    let (pool, _db) = setup_test_database_with_instance().await;
    let client = pool.get().await.unwrap();
    let user_uuid = uuid::Uuid::parse_str(ADMIN_UUID).unwrap();

    let label_id: i32 = client
        .query_one(
            "INSERT INTO labels (user_id, name, color) VALUES ($1, 'ToDelete', '#123456') RETURNING id",
            &[&user_uuid],
        )
        .await
        .unwrap()
        .get(0);

    let main_pool = common::utils::wrap_main_pool(pool.clone());
    let config = common::utils::create_test_config();
    let app = test::init_service(build_app_data(main_pool, config)).await;

    let token = admin_token().await;
    let req = test::TestRequest::delete()
        .uri(&format!("/labels/{}", label_id))
        .insert_header(("Authorization", format!("Bearer {}", token)))
        .to_request();
    let resp = test::call_service(&app, req).await;
    assert_eq!(resp.status(), http::StatusCode::OK);

    let count: i64 = client
        .query_one("SELECT COUNT(*) FROM labels WHERE id = $1", &[&label_id])
        .await
        .unwrap()
        .get(0);
    assert_eq!(count, 0);
}

#[actix_web::test]
#[serial]
async fn test_delete_label_not_found() {
    common::init_log();
    let (pool, _db) = setup_test_database_with_instance().await;
    let main_pool = common::utils::wrap_main_pool(pool.clone());
    let config = common::utils::create_test_config();
    let app = test::init_service(build_app_data(main_pool, config)).await;

    let token = admin_token().await;
    let req = test::TestRequest::delete()
        .uri("/labels/999999")
        .insert_header(("Authorization", format!("Bearer {}", token)))
        .to_request();
    let resp = test::call_service(&app, req).await;
    assert_eq!(resp.status(), http::StatusCode::NOT_FOUND);
}

#[actix_web::test]
#[serial]
async fn test_get_labels_unauthenticated() {
    common::init_log();
    let (pool, _db) = setup_test_database_with_instance().await;
    let main_pool = common::utils::wrap_main_pool(pool.clone());
    let config = common::utils::create_test_config();
    let app = test::init_service(build_app_data(main_pool, config)).await;

    let req = test::TestRequest::get().uri("/labels").to_request();
    let resp = test::call_service(&app, req).await;
    assert_eq!(resp.status(), http::StatusCode::UNAUTHORIZED);
}

#[actix_web::test]
#[serial]
async fn test_add_image_label_and_get() {
    common::init_log();
    let (pool, _db) = setup_test_database_with_instance().await;
    let client = pool.get().await.unwrap();
    let user_uuid = uuid::Uuid::parse_str(ADMIN_UUID).unwrap();

    // Insert a test image owned by the test user
    let hash = "label_test_image_hash_001";
    client.execute(
        "INSERT INTO images (deviceid, hash, user_id, name, ext) VALUES ('dev1', $1, $2, 'test.jpg', 'jpg') ON CONFLICT DO NOTHING",
        &[&hash, &user_uuid],
    ).await.unwrap();

    // Insert a label
    let label_id: i32 = client
        .query_one(
            "INSERT INTO labels (user_id, name, color) VALUES ($1, 'LabelForImage', '#ABCDEF') RETURNING id",
            &[&user_uuid],
        )
        .await
        .unwrap()
        .get(0);

    let main_pool = common::utils::wrap_main_pool(pool.clone());
    let config = common::utils::create_test_config();
    let app = test::init_service(build_app_data(main_pool, config)).await;

    let token = admin_token().await;

    // Add label to image
    let req = test::TestRequest::post()
        .uri(&format!("/images/{}/labels", hash))
        .insert_header(("Authorization", format!("Bearer {}", token.clone())))
        .set_json(&serde_json::json!({"label_id": label_id}))
        .to_request();
    let resp = test::call_service(&app, req).await;
    assert_eq!(resp.status(), http::StatusCode::OK);

    // Get labels for image
    let req = test::TestRequest::get()
        .uri(&format!("/images/{}/labels", hash))
        .insert_header(("Authorization", format!("Bearer {}", token)))
        .to_request();
    let resp = test::call_service(&app, req).await;
    assert_eq!(resp.status(), http::StatusCode::OK);
    let body: serde_json::Value = test::read_body_json(resp).await;
    let labels = body["labels"].as_array().unwrap();
    assert_eq!(labels.len(), 1);
    assert_eq!(labels[0]["id"], label_id);
    assert_eq!(labels[0]["name"], "LabelForImage");

    // Cleanup
    client.execute("DELETE FROM images WHERE hash = $1", &[&hash]).await.unwrap();
}

#[actix_web::test]
#[serial]
async fn test_add_image_label_image_not_found() {
    common::init_log();
    let (pool, _db) = setup_test_database_with_instance().await;
    let client = pool.get().await.unwrap();
    let user_uuid = uuid::Uuid::parse_str(ADMIN_UUID).unwrap();

    let label_id: i32 = client
        .query_one(
            "INSERT INTO labels (user_id, name, color) VALUES ($1, 'Orphan', '#000000') RETURNING id",
            &[&user_uuid],
        )
        .await
        .unwrap()
        .get(0);

    let main_pool = common::utils::wrap_main_pool(pool.clone());
    let config = common::utils::create_test_config();
    let app = test::init_service(build_app_data(main_pool, config)).await;

    let token = admin_token().await;
    let req = test::TestRequest::post()
        .uri("/images/nonexistent_hash_xyz/labels")
        .insert_header(("Authorization", format!("Bearer {}", token)))
        .set_json(&serde_json::json!({"label_id": label_id}))
        .to_request();
    let resp = test::call_service(&app, req).await;
    assert_eq!(resp.status(), http::StatusCode::NOT_FOUND);
}

#[actix_web::test]
#[serial]
async fn test_remove_image_label() {
    common::init_log();
    let (pool, _db) = setup_test_database_with_instance().await;
    let client = pool.get().await.unwrap();
    let user_uuid = uuid::Uuid::parse_str(ADMIN_UUID).unwrap();

    let hash = "label_remove_test_hash_001";
    client.execute(
        "INSERT INTO images (deviceid, hash, user_id, name, ext) VALUES ('dev1', $1, $2, 'test.jpg', 'jpg') ON CONFLICT DO NOTHING",
        &[&hash, &user_uuid],
    ).await.unwrap();

    let label_id: i32 = client
        .query_one(
            "INSERT INTO labels (user_id, name, color) VALUES ($1, 'ToRemove', '#FFFFFF') RETURNING id",
            &[&user_uuid],
        )
        .await
        .unwrap()
        .get(0);

    // Add label directly in DB
    client.execute(
        "INSERT INTO image_labels (image_hash, image_deviceid, label_id) VALUES ($1, 'dev1', $2) ON CONFLICT DO NOTHING",
        &[&hash, &label_id],
    ).await.unwrap();

    let main_pool = common::utils::wrap_main_pool(pool.clone());
    let config = common::utils::create_test_config();
    let app = test::init_service(build_app_data(main_pool, config)).await;

    let token = admin_token().await;
    let req = test::TestRequest::delete()
        .uri(&format!("/images/{}/labels/{}", hash, label_id))
        .insert_header(("Authorization", format!("Bearer {}", token)))
        .to_request();
    let resp = test::call_service(&app, req).await;
    assert_eq!(resp.status(), http::StatusCode::OK);

    let count: i64 = client
        .query_one(
            "SELECT COUNT(*) FROM image_labels WHERE image_hash = $1 AND label_id = $2",
            &[&hash, &label_id],
        )
        .await
        .unwrap()
        .get(0);
    assert_eq!(count, 0);

    // Cleanup
    client.execute("DELETE FROM images WHERE hash = $1", &[&hash]).await.unwrap();
}

#[actix_web::test]
#[serial]
async fn test_add_video_label_and_get() {
    common::init_log();
    let (pool, _db) = setup_test_database_with_instance().await;
    let client = pool.get().await.unwrap();
    let user_uuid = uuid::Uuid::parse_str(ADMIN_UUID).unwrap();

    let hash = "label_test_video_hash_001";
    client.execute(
        "INSERT INTO videos (deviceid, hash, user_id, name, ext) VALUES ('dev1', $1, $2, 'test.mp4', 'mp4') ON CONFLICT DO NOTHING",
        &[&hash, &user_uuid],
    ).await.unwrap();

    let label_id: i32 = client
        .query_one(
            "INSERT INTO labels (user_id, name, color) VALUES ($1, 'VideoLabel', '#FFAA00') RETURNING id",
            &[&user_uuid],
        )
        .await
        .unwrap()
        .get(0);

    let main_pool = common::utils::wrap_main_pool(pool.clone());
    let config = common::utils::create_test_config();
    let app = test::init_service(build_app_data(main_pool, config)).await;

    let token = admin_token().await;

    // Add label to video
    let req = test::TestRequest::post()
        .uri(&format!("/videos/{}/labels", hash))
        .insert_header(("Authorization", format!("Bearer {}", token.clone())))
        .set_json(&serde_json::json!({"label_id": label_id}))
        .to_request();
    let resp = test::call_service(&app, req).await;
    assert_eq!(resp.status(), http::StatusCode::OK);

    // Get labels for video
    let req = test::TestRequest::get()
        .uri(&format!("/videos/{}/labels", hash))
        .insert_header(("Authorization", format!("Bearer {}", token)))
        .to_request();
    let resp = test::call_service(&app, req).await;
    assert_eq!(resp.status(), http::StatusCode::OK);
    let body: serde_json::Value = test::read_body_json(resp).await;
    let labels = body["labels"].as_array().unwrap();
    assert_eq!(labels.len(), 1);
    assert_eq!(labels[0]["name"], "VideoLabel");

    // Cleanup
    client.execute("DELETE FROM videos WHERE hash = $1", &[&hash]).await.unwrap();
}
