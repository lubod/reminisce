use actix_web::{http, test, web, App};
use reminisce::test_utils::setup_test_database_with_instance;
use reminisce::Claims;
use serial_test::serial;

mod common;

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

const ADMIN_UUID: &str = "550e8400-e29b-41d4-a716-446655440000";
const OTHER_UUID: &str = "aaaaaaaa-bbbb-cccc-dddd-eeeeeeeeeeee";

#[actix_web::test]
#[serial]
async fn test_list_users_admin() {
    common::init_log();
    let (pool, _db) = setup_test_database_with_instance().await;
    let main_pool = common::utils::wrap_main_pool(pool.clone());
    let config = common::utils::create_test_config();

    let app = test::init_service(
        App::new()
            .app_data(web::Data::new(main_pool))
            .app_data(web::Data::new(config))
            .service(reminisce::services::user_management::list_users),
    )
    .await;

    let token = make_user_token(ADMIN_UUID, "admin");
    let req = test::TestRequest::get()
        .uri("/users")
        .insert_header(("Authorization", format!("Bearer {}", token)))
        .to_request();
    let resp = test::call_service(&app, req).await;
    assert_eq!(resp.status(), http::StatusCode::OK);

    let body: serde_json::Value = test::read_body_json(resp).await;
    assert!(body.as_array().unwrap().len() >= 1);
}

#[actix_web::test]
#[serial]
async fn test_list_users_non_admin_forbidden() {
    common::init_log();
    let (pool, _db) = setup_test_database_with_instance().await;
    let main_pool = common::utils::wrap_main_pool(pool.clone());
    let config = common::utils::create_test_config();

    let app = test::init_service(
        App::new()
            .app_data(web::Data::new(main_pool))
            .app_data(web::Data::new(config))
            .service(reminisce::services::user_management::list_users),
    )
    .await;

    let token = make_user_token(OTHER_UUID, "user");
    let req = test::TestRequest::get()
        .uri("/users")
        .insert_header(("Authorization", format!("Bearer {}", token)))
        .to_request();
    let resp = test::call_service(&app, req).await;
    assert_eq!(resp.status(), http::StatusCode::FORBIDDEN);
}

#[actix_web::test]
#[serial]
async fn test_list_users_unauthenticated() {
    common::init_log();
    let (pool, _db) = setup_test_database_with_instance().await;
    let main_pool = common::utils::wrap_main_pool(pool.clone());
    let config = common::utils::create_test_config();

    let app = test::init_service(
        App::new()
            .app_data(web::Data::new(main_pool))
            .app_data(web::Data::new(config))
            .service(reminisce::services::user_management::list_users),
    )
    .await;

    let req = test::TestRequest::get().uri("/users").to_request();
    let resp = test::call_service(&app, req).await;
    assert_eq!(resp.status(), http::StatusCode::UNAUTHORIZED);
}

#[actix_web::test]
#[serial]
async fn test_create_user_admin() {
    common::init_log();
    let (pool, _db) = setup_test_database_with_instance().await;
    let main_pool = common::utils::wrap_main_pool(pool.clone());
    let config = common::utils::create_test_config();

    let app = test::init_service(
        App::new()
            .app_data(web::Data::new(main_pool))
            .app_data(web::Data::new(config))
            .service(reminisce::services::user_management::create_user),
    )
    .await;

    let token = make_user_token(ADMIN_UUID, "admin");
    let req = test::TestRequest::post()
        .uri("/users")
        .insert_header(("Authorization", format!("Bearer {}", token)))
        .set_json(&serde_json::json!({
            "username": "newuser",
            "password": "password123",
            "role": "user"
        }))
        .to_request();
    let resp = test::call_service(&app, req).await;
    assert_eq!(resp.status(), http::StatusCode::CREATED);

    let body: serde_json::Value = test::read_body_json(resp).await;
    assert_eq!(body["status"], "ok");
    assert!(body["user_id"].is_string());
}

#[actix_web::test]
#[serial]
async fn test_create_user_non_admin_forbidden() {
    common::init_log();
    let (pool, _db) = setup_test_database_with_instance().await;
    let main_pool = common::utils::wrap_main_pool(pool.clone());
    let config = common::utils::create_test_config();

    let app = test::init_service(
        App::new()
            .app_data(web::Data::new(main_pool))
            .app_data(web::Data::new(config))
            .service(reminisce::services::user_management::create_user),
    )
    .await;

    let token = make_user_token(OTHER_UUID, "user");
    let req = test::TestRequest::post()
        .uri("/users")
        .insert_header(("Authorization", format!("Bearer {}", token)))
        .set_json(&serde_json::json!({
            "username": "anotheruser",
            "password": "password123"
        }))
        .to_request();
    let resp = test::call_service(&app, req).await;
    assert_eq!(resp.status(), http::StatusCode::FORBIDDEN);
}

#[actix_web::test]
#[serial]
async fn test_create_user_short_password() {
    common::init_log();
    let (pool, _db) = setup_test_database_with_instance().await;
    let main_pool = common::utils::wrap_main_pool(pool.clone());
    let config = common::utils::create_test_config();

    let app = test::init_service(
        App::new()
            .app_data(web::Data::new(main_pool))
            .app_data(web::Data::new(config))
            .service(reminisce::services::user_management::create_user),
    )
    .await;

    let token = make_user_token(ADMIN_UUID, "admin");
    let req = test::TestRequest::post()
        .uri("/users")
        .insert_header(("Authorization", format!("Bearer {}", token)))
        .set_json(&serde_json::json!({
            "username": "someuser",
            "password": "short"
        }))
        .to_request();
    let resp = test::call_service(&app, req).await;
    assert_eq!(resp.status(), http::StatusCode::BAD_REQUEST);
}

#[actix_web::test]
#[serial]
async fn test_create_user_invalid_role() {
    common::init_log();
    let (pool, _db) = setup_test_database_with_instance().await;
    let main_pool = common::utils::wrap_main_pool(pool.clone());
    let config = common::utils::create_test_config();

    let app = test::init_service(
        App::new()
            .app_data(web::Data::new(main_pool))
            .app_data(web::Data::new(config))
            .service(reminisce::services::user_management::create_user),
    )
    .await;

    let token = make_user_token(ADMIN_UUID, "admin");
    let req = test::TestRequest::post()
        .uri("/users")
        .insert_header(("Authorization", format!("Bearer {}", token)))
        .set_json(&serde_json::json!({
            "username": "someuser",
            "password": "password123",
            "role": "superuser"
        }))
        .to_request();
    let resp = test::call_service(&app, req).await;
    assert_eq!(resp.status(), http::StatusCode::BAD_REQUEST);
}

#[actix_web::test]
#[serial]
async fn test_create_user_duplicate_username() {
    common::init_log();
    let (pool, _db) = setup_test_database_with_instance().await;
    let main_pool = common::utils::wrap_main_pool(pool.clone());
    let config = common::utils::create_test_config();

    let app = test::init_service(
        App::new()
            .app_data(web::Data::new(main_pool))
            .app_data(web::Data::new(config))
            .service(reminisce::services::user_management::create_user),
    )
    .await;

    let token = make_user_token(ADMIN_UUID, "admin");
    // First create succeeds
    let req = test::TestRequest::post()
        .uri("/users")
        .insert_header(("Authorization", format!("Bearer {}", token.clone())))
        .set_json(&serde_json::json!({"username": "dupuser", "password": "password123"}))
        .to_request();
    let resp = test::call_service(&app, req).await;
    assert_eq!(resp.status(), http::StatusCode::CREATED);

    // Second create with same username fails
    let req = test::TestRequest::post()
        .uri("/users")
        .insert_header(("Authorization", format!("Bearer {}", token)))
        .set_json(&serde_json::json!({"username": "dupuser", "password": "password456"}))
        .to_request();
    let resp = test::call_service(&app, req).await;
    assert_eq!(resp.status(), http::StatusCode::BAD_REQUEST);
    let body: serde_json::Value = test::read_body_json(resp).await;
    assert_eq!(body["message"], "Username already exists");
}

#[actix_web::test]
#[serial]
async fn test_update_user_role() {
    common::init_log();
    let (pool, _db) = setup_test_database_with_instance().await;
    let client = pool.get().await.unwrap();

    // Insert a target user
    let target_id: uuid::Uuid = client
        .query_one(
            "INSERT INTO users (username, email, password_hash, role) VALUES ('target', 'target@local', 'hash', 'user') RETURNING id",
            &[],
        )
        .await
        .unwrap()
        .get(0);

    let main_pool = common::utils::wrap_main_pool(pool.clone());
    let config = common::utils::create_test_config();

    let app = test::init_service(
        App::new()
            .app_data(web::Data::new(main_pool))
            .app_data(web::Data::new(config))
            .service(reminisce::services::user_management::update_user),
    )
    .await;

    let token = make_user_token(ADMIN_UUID, "admin");
    let req = test::TestRequest::patch()
        .uri(&format!("/users/{}", target_id))
        .insert_header(("Authorization", format!("Bearer {}", token)))
        .set_json(&serde_json::json!({"role": "viewer"}))
        .to_request();
    let resp = test::call_service(&app, req).await;
    assert_eq!(resp.status(), http::StatusCode::OK);

    // Verify role changed
    let row = client
        .query_one("SELECT role FROM users WHERE id = $1", &[&target_id])
        .await
        .unwrap();
    let role: &str = row.get(0);
    assert_eq!(role, "viewer");
}

#[actix_web::test]
#[serial]
async fn test_update_user_cannot_demote_self() {
    common::init_log();
    let (pool, _db) = setup_test_database_with_instance().await;
    let main_pool = common::utils::wrap_main_pool(pool.clone());
    let config = common::utils::create_test_config();

    let app = test::init_service(
        App::new()
            .app_data(web::Data::new(main_pool))
            .app_data(web::Data::new(config))
            .service(reminisce::services::user_management::update_user),
    )
    .await;

    let token = make_user_token(ADMIN_UUID, "admin");
    let req = test::TestRequest::patch()
        .uri(&format!("/users/{}", ADMIN_UUID))
        .insert_header(("Authorization", format!("Bearer {}", token)))
        .set_json(&serde_json::json!({"role": "user"}))
        .to_request();
    let resp = test::call_service(&app, req).await;
    assert_eq!(resp.status(), http::StatusCode::BAD_REQUEST);
    let body: serde_json::Value = test::read_body_json(resp).await;
    assert_eq!(body["message"], "Cannot remove your own admin role");
}

#[actix_web::test]
#[serial]
async fn test_update_user_cannot_deactivate_self() {
    common::init_log();
    let (pool, _db) = setup_test_database_with_instance().await;
    let main_pool = common::utils::wrap_main_pool(pool.clone());
    let config = common::utils::create_test_config();

    let app = test::init_service(
        App::new()
            .app_data(web::Data::new(main_pool))
            .app_data(web::Data::new(config))
            .service(reminisce::services::user_management::update_user),
    )
    .await;

    let token = make_user_token(ADMIN_UUID, "admin");
    let req = test::TestRequest::patch()
        .uri(&format!("/users/{}", ADMIN_UUID))
        .insert_header(("Authorization", format!("Bearer {}", token)))
        .set_json(&serde_json::json!({"is_active": false}))
        .to_request();
    let resp = test::call_service(&app, req).await;
    assert_eq!(resp.status(), http::StatusCode::BAD_REQUEST);
    let body: serde_json::Value = test::read_body_json(resp).await;
    assert_eq!(body["message"], "Cannot deactivate your own account");
}

#[actix_web::test]
#[serial]
async fn test_delete_user() {
    common::init_log();
    let (pool, _db) = setup_test_database_with_instance().await;
    let client = pool.get().await.unwrap();

    let target_id: uuid::Uuid = client
        .query_one(
            "INSERT INTO users (username, email, password_hash, role) VALUES ('todelete', 'todelete@local', 'hash', 'user') RETURNING id",
            &[],
        )
        .await
        .unwrap()
        .get(0);

    let main_pool = common::utils::wrap_main_pool(pool.clone());
    let config = common::utils::create_test_config();

    let app = test::init_service(
        App::new()
            .app_data(web::Data::new(main_pool))
            .app_data(web::Data::new(config))
            .service(reminisce::services::user_management::delete_user),
    )
    .await;

    let token = make_user_token(ADMIN_UUID, "admin");
    let req = test::TestRequest::delete()
        .uri(&format!("/users/{}", target_id))
        .insert_header(("Authorization", format!("Bearer {}", token)))
        .to_request();
    let resp = test::call_service(&app, req).await;
    assert_eq!(resp.status(), http::StatusCode::OK);

    // Verify deleted
    let count: i64 = client
        .query_one("SELECT COUNT(*) FROM users WHERE id = $1", &[&target_id])
        .await
        .unwrap()
        .get(0);
    assert_eq!(count, 0);
}

#[actix_web::test]
#[serial]
async fn test_delete_user_not_found() {
    common::init_log();
    let (pool, _db) = setup_test_database_with_instance().await;
    let main_pool = common::utils::wrap_main_pool(pool.clone());
    let config = common::utils::create_test_config();

    let app = test::init_service(
        App::new()
            .app_data(web::Data::new(main_pool))
            .app_data(web::Data::new(config))
            .service(reminisce::services::user_management::delete_user),
    )
    .await;

    let token = make_user_token(ADMIN_UUID, "admin");
    let req = test::TestRequest::delete()
        .uri("/users/00000000-0000-0000-0000-000000000001")
        .insert_header(("Authorization", format!("Bearer {}", token)))
        .to_request();
    let resp = test::call_service(&app, req).await;
    assert_eq!(resp.status(), http::StatusCode::NOT_FOUND);
}

#[actix_web::test]
#[serial]
async fn test_delete_user_cannot_delete_self() {
    common::init_log();
    let (pool, _db) = setup_test_database_with_instance().await;
    let main_pool = common::utils::wrap_main_pool(pool.clone());
    let config = common::utils::create_test_config();

    let app = test::init_service(
        App::new()
            .app_data(web::Data::new(main_pool))
            .app_data(web::Data::new(config))
            .service(reminisce::services::user_management::delete_user),
    )
    .await;

    let token = make_user_token(ADMIN_UUID, "admin");
    let req = test::TestRequest::delete()
        .uri(&format!("/users/{}", ADMIN_UUID))
        .insert_header(("Authorization", format!("Bearer {}", token)))
        .to_request();
    let resp = test::call_service(&app, req).await;
    assert_eq!(resp.status(), http::StatusCode::BAD_REQUEST);
    let body: serde_json::Value = test::read_body_json(resp).await;
    assert_eq!(body["message"], "Cannot delete your own account");
}
