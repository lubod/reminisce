use actix_web::{http, test, web, App};
use reminisce::test_utils::setup_test_database_with_instance;
use serial_test::serial;

mod common;

/// Test user registration with valid credentials
#[actix_web::test]
#[serial]
async fn test_register_user_success() {
    common::init_log();
    let (pool, _test_db) = setup_test_database_with_instance().await;

    let main_pool = common::utils::wrap_main_pool(pool.clone());

    let app = test::init_service(
        App::new()
            .app_data(web::Data::new(main_pool))
            .service(reminisce::services::auth::register_user)
    ).await;

    let request_body = serde_json::json!({
        "username": "testuser_register",
        "email": "testuser_register@example.com",
        "password": "securepassword123"
    });

    let req = test::TestRequest::post()
        .uri("/auth/register")
        .insert_header(("Content-Type", "application/json"))
        .set_json(&request_body)
        .to_request();

    let response = test::call_service(&app, req).await;
    assert_eq!(response.status(), http::StatusCode::CREATED);

    let body: serde_json::Value = test::read_body_json(response).await;
    assert_eq!(body["status"], "success");
    assert!(body["user_id"].is_string());

    // Clean up
    let client = pool.get().await.expect("Failed to get client");
    client.execute("DELETE FROM users WHERE username = $1", &[&"testuser_register"]).await
        .expect("Failed to clean up");
}

/// Test user registration with invalid email
#[actix_web::test]
#[serial]
async fn test_register_user_invalid_email() {
    common::init_log();
    let (pool, _test_db) = setup_test_database_with_instance().await;

    let main_pool = common::utils::wrap_main_pool(pool.clone());

    let app = test::init_service(
        App::new()
            .app_data(web::Data::new(main_pool))
            .service(reminisce::services::auth::register_user)
    ).await;

    let request_body = serde_json::json!({
        "username": "testuser",
        "email": "invalid-email",  // Invalid email format
        "password": "securepassword123"
    });

    let req = test::TestRequest::post()
        .uri("/auth/register")
        .insert_header(("Content-Type", "application/json"))
        .set_json(&request_body)
        .to_request();

    let response = test::call_service(&app, req).await;
    assert_eq!(response.status(), http::StatusCode::BAD_REQUEST);
}

/// Test user registration with short password
#[actix_web::test]
#[serial]
async fn test_register_user_short_password() {
    common::init_log();
    let (pool, _test_db) = setup_test_database_with_instance().await;

    let main_pool = common::utils::wrap_main_pool(pool.clone());

    let app = test::init_service(
        App::new()
            .app_data(web::Data::new(main_pool))
            .service(reminisce::services::auth::register_user)
    ).await;

    let request_body = serde_json::json!({
        "username": "testuser",
        "email": "test@example.com",
        "password": "short"  // Less than 8 characters
    });

    let req = test::TestRequest::post()
        .uri("/auth/register")
        .insert_header(("Content-Type", "application/json"))
        .set_json(&request_body)
        .to_request();

    let response = test::call_service(&app, req).await;
    assert_eq!(response.status(), http::StatusCode::BAD_REQUEST);
}

/// Test user registration with short username
#[actix_web::test]
#[serial]
async fn test_register_user_short_username() {
    common::init_log();
    let (pool, _test_db) = setup_test_database_with_instance().await;

    let main_pool = common::utils::wrap_main_pool(pool.clone());

    let app = test::init_service(
        App::new()
            .app_data(web::Data::new(main_pool))
            .service(reminisce::services::auth::register_user)
    ).await;

    let request_body = serde_json::json!({
        "username": "ab",  // Less than 3 characters
        "email": "test@example.com",
        "password": "securepassword123"
    });

    let req = test::TestRequest::post()
        .uri("/auth/register")
        .insert_header(("Content-Type", "application/json"))
        .set_json(&request_body)
        .to_request();

    let response = test::call_service(&app, req).await;
    assert_eq!(response.status(), http::StatusCode::BAD_REQUEST);
}

/// Test user registration with duplicate username
#[actix_web::test]
#[serial]
async fn test_register_user_duplicate_username() {
    common::init_log();
    let (pool, _test_db) = setup_test_database_with_instance().await;

    let main_pool = common::utils::wrap_main_pool(pool.clone());

    let app = test::init_service(
        App::new()
            .app_data(web::Data::new(main_pool))
            .service(reminisce::services::auth::register_user)
    ).await;

    // First registration
    let request_body = serde_json::json!({
        "username": "duplicate_user",
        "email": "duplicate1@example.com",
        "password": "securepassword123"
    });

    let req = test::TestRequest::post()
        .uri("/auth/register")
        .insert_header(("Content-Type", "application/json"))
        .set_json(&request_body)
        .to_request();

    let response = test::call_service(&app, req).await;
    assert_eq!(response.status(), http::StatusCode::CREATED);

    // Second registration with same username
    let request_body2 = serde_json::json!({
        "username": "duplicate_user",
        "email": "duplicate2@example.com",
        "password": "securepassword456"
    });

    let req2 = test::TestRequest::post()
        .uri("/auth/register")
        .insert_header(("Content-Type", "application/json"))
        .set_json(&request_body2)
        .to_request();

    let response2 = test::call_service(&app, req2).await;
    assert_eq!(response2.status(), http::StatusCode::BAD_REQUEST);

    let body: serde_json::Value = test::read_body_json(response2).await;
    assert_eq!(body["status"], "error");
    assert!(body["message"].as_str().unwrap().contains("already exists"));

    // Clean up
    let client = pool.get().await.expect("Failed to get client");
    client.execute("DELETE FROM users WHERE username = $1", &[&"duplicate_user"]).await
        .expect("Failed to clean up");
}

/// Test user login with valid credentials
#[actix_web::test]
#[serial]
async fn test_user_login_success() {
    common::init_log();
    let (pool, _test_db) = setup_test_database_with_instance().await;
    let config = common::utils::create_test_config();

    let main_pool = common::utils::wrap_main_pool(pool.clone());

    let app = test::init_service(
        App::new()
            .app_data(web::Data::new(main_pool))
            .app_data(web::Data::new(config.clone()))
            .service(reminisce::services::auth::register_user)
            .service(reminisce::services::auth::user_login)
    ).await;

    // First register a user
    let register_body = serde_json::json!({
        "username": "login_test_user",
        "email": "login_test@example.com",
        "password": "securepassword123"
    });

    let reg_req = test::TestRequest::post()
        .uri("/auth/register")
        .insert_header(("Content-Type", "application/json"))
        .set_json(&register_body)
        .to_request();

    let reg_response = test::call_service(&app, reg_req).await;
    assert_eq!(reg_response.status(), http::StatusCode::CREATED);

    // Now login
    let login_body = serde_json::json!({
        "username": "login_test_user",
        "password": "securepassword123"
    });

    let login_req = test::TestRequest::post()
        .uri("/auth/user-login")
        .insert_header(("Content-Type", "application/json"))
        .set_json(&login_body)
        .to_request();

    let login_response = test::call_service(&app, login_req).await;
    assert_eq!(login_response.status(), http::StatusCode::OK);

    let body: serde_json::Value = test::read_body_json(login_response).await;
    assert!(body["access_token"].is_string(), "Response should have access_token");
    assert!(body["user"].is_object(), "Response should have user object");
    assert!(body["user"]["id"].is_string(), "User should have id");
    assert_eq!(body["user"]["username"], "login_test_user");

    // Clean up
    let client = pool.get().await.expect("Failed to get client");
    client.execute("DELETE FROM users WHERE username = $1", &[&"login_test_user"]).await
        .expect("Failed to clean up");
}

/// Test user login with wrong password
#[actix_web::test]
#[serial]
async fn test_user_login_wrong_password() {
    common::init_log();
    let (pool, _test_db) = setup_test_database_with_instance().await;
    let config = common::utils::create_test_config();

    let main_pool = common::utils::wrap_main_pool(pool.clone());

    let app = test::init_service(
        App::new()
            .app_data(web::Data::new(main_pool))
            .app_data(web::Data::new(config.clone()))
            .service(reminisce::services::auth::register_user)
            .service(reminisce::services::auth::user_login)
    ).await;

    // First register a user
    let register_body = serde_json::json!({
        "username": "wrong_pass_user",
        "email": "wrong_pass@example.com",
        "password": "correctpassword123"
    });

    let reg_req = test::TestRequest::post()
        .uri("/auth/register")
        .insert_header(("Content-Type", "application/json"))
        .set_json(&register_body)
        .to_request();

    test::call_service(&app, reg_req).await;

    // Try login with wrong password
    let login_body = serde_json::json!({
        "username": "wrong_pass_user",
        "password": "wrongpassword123"
    });

    let login_req = test::TestRequest::post()
        .uri("/auth/user-login")
        .insert_header(("Content-Type", "application/json"))
        .set_json(&login_body)
        .to_request();

    let login_response = test::call_service(&app, login_req).await;
    assert_eq!(login_response.status(), http::StatusCode::UNAUTHORIZED);

    // Clean up
    let client = pool.get().await.expect("Failed to get client");
    client.execute("DELETE FROM users WHERE username = $1", &[&"wrong_pass_user"]).await
        .expect("Failed to clean up");
}

/// Test user login with non-existent user
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

    let login_body = serde_json::json!({
        "username": "nonexistent_user",
        "password": "somepassword123"
    });

    let login_req = test::TestRequest::post()
        .uri("/auth/user-login")
        .insert_header(("Content-Type", "application/json"))
        .set_json(&login_body)
        .to_request();

    let login_response = test::call_service(&app, login_req).await;
    assert_eq!(login_response.status(), http::StatusCode::UNAUTHORIZED);
}

/// Test user login without optional fields
#[actix_web::test]
#[serial]
async fn test_user_login_minimal_body() {
    common::init_log();
    let (pool, _test_db) = setup_test_database_with_instance().await;
    let config = common::utils::create_test_config();

    let main_pool = common::utils::wrap_main_pool(pool.clone());

    let app = test::init_service(
        App::new()
            .app_data(web::Data::new(main_pool))
            .app_data(web::Data::new(config.clone()))
            .service(reminisce::services::auth::register_user)
            .service(reminisce::services::auth::user_login)
    ).await;

    // First register a user
    let register_body = serde_json::json!({
        "username": "default_device_user",
        "email": "default_device@example.com",
        "password": "securepassword123"
    });

    let reg_req = test::TestRequest::post()
        .uri("/auth/register")
        .insert_header(("Content-Type", "application/json"))
        .set_json(&register_body)
        .to_request();

    test::call_service(&app, reg_req).await;

    // Login without specifying device_id (should use default)
    let login_body = serde_json::json!({
        "username": "default_device_user",
        "password": "securepassword123"
    });

    let login_req = test::TestRequest::post()
        .uri("/auth/user-login")
        .insert_header(("Content-Type", "application/json"))
        .set_json(&login_body)
        .to_request();

    let login_response = test::call_service(&app, login_req).await;
    assert_eq!(login_response.status(), http::StatusCode::OK);

    let body: serde_json::Value = test::read_body_json(login_response).await;
    assert!(body["access_token"].is_string(), "Response should have access_token");
    assert!(body["user"].is_object(), "Response should have user object");

    // Clean up
    let client = pool.get().await.expect("Failed to get client");
    client.execute("DELETE FROM users WHERE username = $1", &[&"default_device_user"]).await
        .expect("Failed to clean up");
}
