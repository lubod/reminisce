use actix_web::{http, test, web, App};
use reminisce::test_utils::{setup_test_database_with_instance, setup_empty_test_database_with_instance};
use serial_test::serial;

mod common;

/// Registration is disabled — endpoint must always return 403.
#[actix_web::test]
#[serial]
async fn test_register_user_disabled() {
    common::init_log();
    let (pool, _test_db) = setup_test_database_with_instance().await;
    let main_pool = common::utils::wrap_main_pool(pool.clone());

    let app = test::init_service(
        App::new()
            .app_data(web::Data::new(main_pool))
            .service(reminisce::services::auth::register_user)
    ).await;

    let req = test::TestRequest::post()
        .uri("/auth/register")
        .insert_header(("Content-Type", "application/json"))
        .set_json(serde_json::json!({
            "username": "someuser",
            "email": "someuser@example.com",
            "password": "securepassword123"
        }))
        .to_request();

    let response = test::call_service(&app, req).await;
    assert_eq!(response.status(), http::StatusCode::FORBIDDEN);
}

/// setup_status returns needs_setup=true on a fresh (empty) database.
#[actix_web::test]
#[serial]
async fn test_setup_status_needs_setup() {
    common::init_log();
    let (pool, _test_db) = setup_empty_test_database_with_instance().await;
    let main_pool = common::utils::wrap_main_pool(pool.clone());

    let app = test::init_service(
        App::new()
            .app_data(web::Data::new(main_pool))
            .service(reminisce::services::auth::setup_status)
    ).await;

    let req = test::TestRequest::get()
        .uri("/auth/setup-status")
        .to_request();

    let response = test::call_service(&app, req).await;
    assert_eq!(response.status(), http::StatusCode::OK);
    let body: serde_json::Value = test::read_body_json(response).await;
    assert_eq!(body["needs_setup"], true);
}

/// setup_admin creates the first admin on a fresh database.
#[actix_web::test]
#[serial]
async fn test_setup_admin_success() {
    common::init_log();
    let (pool, _test_db) = setup_empty_test_database_with_instance().await;
    let main_pool = common::utils::wrap_main_pool(pool.clone());

    let app = test::init_service(
        App::new()
            .app_data(web::Data::new(main_pool))
            .service(reminisce::services::auth::setup_admin)
    ).await;

    let req = test::TestRequest::post()
        .uri("/auth/setup")
        .insert_header(("Content-Type", "application/json"))
        .set_json(serde_json::json!({
            "username": "myadmin",
            "password": "adminpassword123"
        }))
        .to_request();

    let response = test::call_service(&app, req).await;
    assert_eq!(response.status(), http::StatusCode::CREATED);
    let body: serde_json::Value = test::read_body_json(response).await;
    assert_eq!(body["status"], "ok");
}

/// setup_admin returns 403 when users already exist.
#[actix_web::test]
#[serial]
async fn test_setup_admin_already_exists() {
    common::init_log();
    let (pool, _test_db) = setup_empty_test_database_with_instance().await;
    let main_pool = common::utils::wrap_main_pool(pool.clone());

    let app = test::init_service(
        App::new()
            .app_data(web::Data::new(main_pool))
            .service(reminisce::services::auth::setup_admin)
    ).await;

    // First setup succeeds
    let req1 = test::TestRequest::post()
        .uri("/auth/setup")
        .insert_header(("Content-Type", "application/json"))
        .set_json(serde_json::json!({"username": "admin1", "password": "adminpassword123"}))
        .to_request();
    let r1 = test::call_service(&app, req1).await;
    assert_eq!(r1.status(), http::StatusCode::CREATED);

    // Second setup is rejected
    let req2 = test::TestRequest::post()
        .uri("/auth/setup")
        .insert_header(("Content-Type", "application/json"))
        .set_json(serde_json::json!({"username": "admin2", "password": "adminpassword456"}))
        .to_request();
    let r2 = test::call_service(&app, req2).await;
    assert_eq!(r2.status(), http::StatusCode::FORBIDDEN);
}

/// Login with valid credentials works after setup.
#[actix_web::test]
#[serial]
async fn test_user_login_success() {
    common::init_log();
    let (pool, _test_db) = setup_empty_test_database_with_instance().await;
    let config = common::utils::create_test_config();
    let main_pool = common::utils::wrap_main_pool(pool.clone());

    let app = test::init_service(
        App::new()
            .app_data(web::Data::new(main_pool))
            .app_data(web::Data::new(config.clone()))
            .service(reminisce::services::auth::setup_admin)
            .service(reminisce::services::auth::user_login)
    ).await;

    // Create admin via setup
    let setup_req = test::TestRequest::post()
        .uri("/auth/setup")
        .insert_header(("Content-Type", "application/json"))
        .set_json(serde_json::json!({"username": "login_test_user", "password": "securepassword123"}))
        .to_request();
    let setup_resp = test::call_service(&app, setup_req).await;
    assert_eq!(setup_resp.status(), http::StatusCode::CREATED);

    // Login
    let login_req = test::TestRequest::post()
        .uri("/auth/user-login")
        .insert_header(("Content-Type", "application/json"))
        .set_json(serde_json::json!({"username": "login_test_user", "password": "securepassword123"}))
        .to_request();

    let login_response = test::call_service(&app, login_req).await;
    assert_eq!(login_response.status(), http::StatusCode::OK);

    let body: serde_json::Value = test::read_body_json(login_response).await;
    assert!(body["access_token"].is_string(), "Response should have access_token");
    assert!(body["user"].is_object(), "Response should have user object");
    assert_eq!(body["user"]["username"], "login_test_user");
    assert_eq!(body["user"]["role"], "admin");
}

/// Login with wrong password returns 401.
#[actix_web::test]
#[serial]
async fn test_user_login_wrong_password() {
    common::init_log();
    let (pool, _test_db) = setup_empty_test_database_with_instance().await;
    let config = common::utils::create_test_config();
    let main_pool = common::utils::wrap_main_pool(pool.clone());

    let app = test::init_service(
        App::new()
            .app_data(web::Data::new(main_pool))
            .app_data(web::Data::new(config.clone()))
            .service(reminisce::services::auth::setup_admin)
            .service(reminisce::services::auth::user_login)
    ).await;

    let setup_req = test::TestRequest::post()
        .uri("/auth/setup")
        .insert_header(("Content-Type", "application/json"))
        .set_json(serde_json::json!({"username": "wrong_pass_user", "password": "correctpassword123"}))
        .to_request();
    test::call_service(&app, setup_req).await;

    let login_req = test::TestRequest::post()
        .uri("/auth/user-login")
        .insert_header(("Content-Type", "application/json"))
        .set_json(serde_json::json!({"username": "wrong_pass_user", "password": "wrongpassword123"}))
        .to_request();

    let response = test::call_service(&app, login_req).await;
    assert_eq!(response.status(), http::StatusCode::UNAUTHORIZED);
}

/// Login with non-existent user returns 401.
#[actix_web::test]
#[serial]
async fn test_user_login_nonexistent_user() {
    common::init_log();
    let (pool, _test_db) = setup_test_database_with_instance().await;
    let config = common::utils::create_test_config();
    let main_pool = common::utils::wrap_main_pool(pool.clone());

    let app = test::init_service(
        App::new()
            .app_data(web::Data::new(main_pool))
            .app_data(web::Data::new(config.clone()))
            .service(reminisce::services::auth::user_login)
    ).await;

    let login_req = test::TestRequest::post()
        .uri("/auth/user-login")
        .insert_header(("Content-Type", "application/json"))
        .set_json(serde_json::json!({"username": "nonexistent_user", "password": "somepassword123"}))
        .to_request();

    let response = test::call_service(&app, login_req).await;
    assert_eq!(response.status(), http::StatusCode::UNAUTHORIZED);
}

/// Login without device_id (optional field) still works.
#[actix_web::test]
#[serial]
async fn test_user_login_minimal_body() {
    common::init_log();
    let (pool, _test_db) = setup_empty_test_database_with_instance().await;
    let config = common::utils::create_test_config();
    let main_pool = common::utils::wrap_main_pool(pool.clone());

    let app = test::init_service(
        App::new()
            .app_data(web::Data::new(main_pool))
            .app_data(web::Data::new(config.clone()))
            .service(reminisce::services::auth::setup_admin)
            .service(reminisce::services::auth::user_login)
    ).await;

    let setup_req = test::TestRequest::post()
        .uri("/auth/setup")
        .insert_header(("Content-Type", "application/json"))
        .set_json(serde_json::json!({"username": "default_device_user", "password": "securepassword123"}))
        .to_request();
    test::call_service(&app, setup_req).await;

    let login_req = test::TestRequest::post()
        .uri("/auth/user-login")
        .insert_header(("Content-Type", "application/json"))
        .set_json(serde_json::json!({"username": "default_device_user", "password": "securepassword123"}))
        .to_request();

    let response = test::call_service(&app, login_req).await;
    assert_eq!(response.status(), http::StatusCode::OK);

    let body: serde_json::Value = test::read_body_json(response).await;
    assert!(body["access_token"].is_string());
    assert!(body["user"].is_object());
}

/// get_me returns the current authenticated user (extracted from the JWT).
#[actix_web::test]
#[serial]
async fn test_get_me_returns_current_user() {
    common::init_log();
    let (pool, _test_db) = setup_test_database_with_instance().await;
    let config = common::utils::create_test_config();
    let main_pool = common::utils::wrap_main_pool(pool.clone());

    let app = test::init_service(
        App::new()
            .app_data(web::Data::new(main_pool))
            .app_data(web::Data::new(config.clone()))
            .service(reminisce::services::auth::get_me)
    ).await;
    let token = common::utils::create_test_jwt_token().await;

    let req = test::TestRequest::get()
        .uri("/auth/me")
        .insert_header(("Authorization", format!("Bearer {}", token)))
        .to_request();
    let response = test::call_service(&app, req).await;
    assert_eq!(response.status(), http::StatusCode::OK);
    let body: serde_json::Value = test::read_body_json(response).await;
    assert_eq!(body["username"], "test-user");
    assert_eq!(body["role"], "admin");
    assert!(body["access_token"].is_string(), "access_token echo");
    assert!(body["image_token"].is_string(), "image_token issued for media_read");

    // No token -> 401 (the Claims extractor rejects the request).
    let req_anon = test::TestRequest::get().uri("/auth/me").to_request();
    let response_anon = test::call_service(&app, req_anon).await;
    assert_eq!(response_anon.status(), http::StatusCode::UNAUTHORIZED);
}

/// user_logout clears the session cookie.
#[actix_web::test]
#[serial]
async fn test_user_logout_clears_cookie() {
    common::init_log();
    let (pool, _test_db) = setup_test_database_with_instance().await;
    let config = common::utils::create_test_config();
    let main_pool = common::utils::wrap_main_pool(pool.clone());

    let app = test::init_service(
        App::new()
            .app_data(web::Data::new(main_pool))
            .app_data(web::Data::new(config.clone()))
            .service(reminisce::services::auth::user_logout)
    ).await;

    let req = test::TestRequest::post().uri("/auth/logout").to_request();
    let response = test::call_service(&app, req).await;
    assert_eq!(response.status(), http::StatusCode::OK);
    let set_cookie = response.headers().get("set-cookie").and_then(|v| v.to_str().ok()).unwrap_or("");
    assert!(set_cookie.contains("access_token="), "logout must clear access_token cookie: {}", set_cookie);
    assert!(set_cookie.contains("Max-Age=0"), "cookie must be expired immediately");
}

/// user_login_form redirects on success and failure (native form login).
#[actix_web::test]
#[serial]
async fn test_user_login_form_redirects() {
    common::init_log();
    let (pool, _test_db) = setup_test_database_with_instance().await;
    let config = common::utils::create_test_config();
    let main_pool = common::utils::wrap_main_pool(pool.clone());

    let app = test::init_service(
        App::new()
            .app_data(web::Data::new(main_pool))
            .app_data(web::Data::new(config.clone()))
            .service(reminisce::services::auth::user_login_form)
    ).await;

    // Correct credentials -> 303 to / with a session cookie (seed password admin123).
    let ok_form = std::collections::HashMap::from([
        ("username".to_string(), "test-user".to_string()),
        ("password".to_string(), "admin123".to_string()),
    ]);
    let req = test::TestRequest::post()
        .uri("/auth/user-login-form")
        .set_form(&ok_form)
        .to_request();
    let response = test::call_service(&app, req).await;
    assert_eq!(response.status(), http::StatusCode::FOUND);
    assert!(response.headers().get("set-cookie").is_some(), "successful login issues a cookie");
    assert_eq!(response.headers().get("location").and_then(|v| v.to_str().ok()), Some("/"));

    // Wrong password -> 303 to /login?error=1 and no session cookie.
    let bad_form = std::collections::HashMap::from([
        ("username".to_string(), "test-user".to_string()),
        ("password".to_string(), "wrongpass".to_string()),
    ]);
    let req = test::TestRequest::post()
        .uri("/auth/user-login-form")
        .set_form(&bad_form)
        .to_request();
    let response = test::call_service(&app, req).await;
    assert_eq!(response.status(), http::StatusCode::FOUND);
    assert_eq!(response.headers().get("location").and_then(|v| v.to_str().ok()), Some("/login?error=1"));
    assert!(response.headers().get("set-cookie").is_none(), "failed login must not issue a cookie");
}
