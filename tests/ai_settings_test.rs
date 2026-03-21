use actix_web::{http, test, web, App};
use reminisce::test_utils::setup_test_database_with_instance;
use reminisce::Claims;
use serial_test::serial;

mod common;

const ADMIN_UUID: &str = "550e8400-e29b-41d4-a716-446655440000";

fn make_user_token(user_id: &str, role: &str) -> String {
    use jsonwebtoken::{encode, Algorithm, EncodingKey, Header};
    let claims = Claims {
        user_id: user_id.to_string(),
        username: "some-user".to_string(),
        email: "".to_string(),
        role: role.to_string(),
        exp: (chrono::Utc::now() + chrono::Duration::days(1)).timestamp() as usize,
    };
    encode(
        &Header::new(Algorithm::HS512),
        &claims,
        &EncodingKey::from_secret("test_secret".as_ref()),
    )
    .unwrap()
}

#[actix_web::test]
#[serial]
async fn test_get_ai_settings_admin() {
    common::init_log();
    let (pool, _db) = setup_test_database_with_instance().await;
    let main_pool = common::utils::wrap_main_pool(pool.clone());
    let config = common::utils::create_test_config();

    let app = test::init_service(
        App::new()
            .app_data(web::Data::new(main_pool))
            .app_data(web::Data::new(config))
            .service(reminisce::services::ai_settings::get_ai_settings),
    )
    .await;

    let token = make_user_token(ADMIN_UUID, "admin");
    let req = test::TestRequest::get()
        .uri("/ai-settings")
        .insert_header(("Authorization", format!("Bearer {}", token)))
        .to_request();
    let resp = test::call_service(&app, req).await;
    assert_eq!(resp.status(), http::StatusCode::OK);

    let body: serde_json::Value = test::read_body_json(resp).await;
    assert!(body["enable_ai_descriptions"].is_boolean());
    assert!(body["enable_embeddings"].is_boolean());
    assert!(body["embedding_parallel_count"].is_number());
    assert!(body["enable_face_detection"].is_boolean());
    assert!(body["face_detection_parallel_count"].is_number());
    assert!(body["enable_media_backup"].is_boolean());
}

#[actix_web::test]
#[serial]
async fn test_get_ai_settings_creates_defaults() {
    common::init_log();
    let (pool, _db) = setup_test_database_with_instance().await;
    let client = pool.get().await.unwrap();

    // Remove any existing ai_settings for test user to force default creation
    let user_uuid = uuid::Uuid::parse_str(ADMIN_UUID).unwrap();
    client.execute("DELETE FROM ai_settings WHERE user_id = $1", &[&user_uuid]).await.unwrap();

    let main_pool = common::utils::wrap_main_pool(pool.clone());
    let config = common::utils::create_test_config();

    let app = test::init_service(
        App::new()
            .app_data(web::Data::new(main_pool))
            .app_data(web::Data::new(config))
            .service(reminisce::services::ai_settings::get_ai_settings),
    )
    .await;

    let token = make_user_token(ADMIN_UUID, "admin");
    let req = test::TestRequest::get()
        .uri("/ai-settings")
        .insert_header(("Authorization", format!("Bearer {}", token)))
        .to_request();
    let resp = test::call_service(&app, req).await;
    assert_eq!(resp.status(), http::StatusCode::OK);

    let body: serde_json::Value = test::read_body_json(resp).await;
    // Defaults: enable_ai_descriptions=true, enable_embeddings=true, embedding_parallel_count=10
    assert_eq!(body["enable_ai_descriptions"], true);
    assert_eq!(body["enable_embeddings"], true);
    assert_eq!(body["embedding_parallel_count"], 10);
    assert_eq!(body["enable_face_detection"], true);
    assert_eq!(body["face_detection_parallel_count"], 10);
    assert_eq!(body["enable_media_backup"], false);

    // Verify row was created in DB
    let count: i64 = client
        .query_one("SELECT COUNT(*) FROM ai_settings WHERE user_id = $1", &[&user_uuid])
        .await
        .unwrap()
        .get(0);
    assert_eq!(count, 1);
}

#[actix_web::test]
#[serial]
async fn test_get_ai_settings_non_admin_forbidden() {
    common::init_log();
    let (pool, _db) = setup_test_database_with_instance().await;
    let main_pool = common::utils::wrap_main_pool(pool.clone());
    let config = common::utils::create_test_config();

    let app = test::init_service(
        App::new()
            .app_data(web::Data::new(main_pool))
            .app_data(web::Data::new(config))
            .service(reminisce::services::ai_settings::get_ai_settings),
    )
    .await;

    let token = make_user_token("aaaaaaaa-bbbb-cccc-dddd-eeeeeeeeeeee", "user");
    let req = test::TestRequest::get()
        .uri("/ai-settings")
        .insert_header(("Authorization", format!("Bearer {}", token)))
        .to_request();
    let resp = test::call_service(&app, req).await;
    assert_eq!(resp.status(), http::StatusCode::FORBIDDEN);
}

#[actix_web::test]
#[serial]
async fn test_get_ai_settings_unauthenticated() {
    common::init_log();
    let (pool, _db) = setup_test_database_with_instance().await;
    let main_pool = common::utils::wrap_main_pool(pool.clone());
    let config = common::utils::create_test_config();

    let app = test::init_service(
        App::new()
            .app_data(web::Data::new(main_pool))
            .app_data(web::Data::new(config))
            .service(reminisce::services::ai_settings::get_ai_settings),
    )
    .await;

    let req = test::TestRequest::get().uri("/ai-settings").to_request();
    let resp = test::call_service(&app, req).await;
    assert_eq!(resp.status(), http::StatusCode::UNAUTHORIZED);
}

#[actix_web::test]
#[serial]
async fn test_update_ai_settings() {
    common::init_log();
    let (pool, _db) = setup_test_database_with_instance().await;
    let main_pool = common::utils::wrap_main_pool(pool.clone());
    let config = common::utils::create_test_config();

    let app = test::init_service(
        App::new()
            .app_data(web::Data::new(main_pool))
            .app_data(web::Data::new(config))
            .service(reminisce::services::ai_settings::update_ai_settings),
    )
    .await;

    let token = make_user_token(ADMIN_UUID, "admin");
    let req = test::TestRequest::put()
        .uri("/ai-settings")
        .insert_header(("Authorization", format!("Bearer {}", token)))
        .set_json(&serde_json::json!({
            "enable_ai_descriptions": false,
            "enable_embeddings": false,
            "embedding_parallel_count": 5
        }))
        .to_request();
    let resp = test::call_service(&app, req).await;
    assert_eq!(resp.status(), http::StatusCode::OK);

    let body: serde_json::Value = test::read_body_json(resp).await;
    assert_eq!(body["enable_ai_descriptions"], false);
    assert_eq!(body["enable_embeddings"], false);
    assert_eq!(body["embedding_parallel_count"], 5);
}

#[actix_web::test]
#[serial]
async fn test_update_ai_settings_invalid_parallel_count_zero() {
    common::init_log();
    let (pool, _db) = setup_test_database_with_instance().await;
    let main_pool = common::utils::wrap_main_pool(pool.clone());
    let config = common::utils::create_test_config();

    let app = test::init_service(
        App::new()
            .app_data(web::Data::new(main_pool))
            .app_data(web::Data::new(config))
            .service(reminisce::services::ai_settings::update_ai_settings),
    )
    .await;

    let token = make_user_token(ADMIN_UUID, "admin");
    let req = test::TestRequest::put()
        .uri("/ai-settings")
        .insert_header(("Authorization", format!("Bearer {}", token)))
        .set_json(&serde_json::json!({"embedding_parallel_count": 0}))
        .to_request();
    let resp = test::call_service(&app, req).await;
    assert_eq!(resp.status(), http::StatusCode::BAD_REQUEST);
}

#[actix_web::test]
#[serial]
async fn test_update_ai_settings_parallel_count_too_high() {
    common::init_log();
    let (pool, _db) = setup_test_database_with_instance().await;
    let main_pool = common::utils::wrap_main_pool(pool.clone());
    let config = common::utils::create_test_config();

    let app = test::init_service(
        App::new()
            .app_data(web::Data::new(main_pool))
            .app_data(web::Data::new(config))
            .service(reminisce::services::ai_settings::update_ai_settings),
    )
    .await;

    let token = make_user_token(ADMIN_UUID, "admin");
    let req = test::TestRequest::put()
        .uri("/ai-settings")
        .insert_header(("Authorization", format!("Bearer {}", token)))
        .set_json(&serde_json::json!({"face_detection_parallel_count": 101}))
        .to_request();
    let resp = test::call_service(&app, req).await;
    assert_eq!(resp.status(), http::StatusCode::BAD_REQUEST);
}

#[actix_web::test]
#[serial]
async fn test_update_ai_settings_non_admin_forbidden() {
    common::init_log();
    let (pool, _db) = setup_test_database_with_instance().await;
    let main_pool = common::utils::wrap_main_pool(pool.clone());
    let config = common::utils::create_test_config();

    let app = test::init_service(
        App::new()
            .app_data(web::Data::new(main_pool))
            .app_data(web::Data::new(config))
            .service(reminisce::services::ai_settings::update_ai_settings),
    )
    .await;

    let token = make_user_token("aaaaaaaa-bbbb-cccc-dddd-eeeeeeeeeeee", "user");
    let req = test::TestRequest::put()
        .uri("/ai-settings")
        .insert_header(("Authorization", format!("Bearer {}", token)))
        .set_json(&serde_json::json!({"enable_embeddings": false}))
        .to_request();
    let resp = test::call_service(&app, req).await;
    assert_eq!(resp.status(), http::StatusCode::FORBIDDEN);
}

#[actix_web::test]
#[serial]
async fn test_update_ai_settings_partial_update() {
    common::init_log();
    let (pool, _db) = setup_test_database_with_instance().await;
    let client = pool.get().await.unwrap();
    let user_uuid = uuid::Uuid::parse_str(ADMIN_UUID).unwrap();

    // Ensure settings exist with known values
    client.execute(
        "INSERT INTO ai_settings (user_id, enable_media_backup, enable_face_detection)
         VALUES ($1, false, true)
         ON CONFLICT (user_id) DO UPDATE SET enable_media_backup = false, enable_face_detection = true",
        &[&user_uuid],
    ).await.unwrap();

    let main_pool = common::utils::wrap_main_pool(pool.clone());
    let config = common::utils::create_test_config();

    let app = test::init_service(
        App::new()
            .app_data(web::Data::new(main_pool))
            .app_data(web::Data::new(config))
            .service(reminisce::services::ai_settings::update_ai_settings),
    )
    .await;

    let token = make_user_token(ADMIN_UUID, "admin");
    // Only update enable_media_backup — face_detection should remain unchanged
    let req = test::TestRequest::put()
        .uri("/ai-settings")
        .insert_header(("Authorization", format!("Bearer {}", token)))
        .set_json(&serde_json::json!({"enable_media_backup": true}))
        .to_request();
    let resp = test::call_service(&app, req).await;
    assert_eq!(resp.status(), http::StatusCode::OK);

    let body: serde_json::Value = test::read_body_json(resp).await;
    assert_eq!(body["enable_media_backup"], true);
    assert_eq!(body["enable_face_detection"], true); // unchanged
}
