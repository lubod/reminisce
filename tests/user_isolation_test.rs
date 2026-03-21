use reminisce::services::thumbnail::ThumbnailsResponse;
use tokio::fs;
use reminisce::services::auth::UserLoginRequest;
use actix_web::{test, App, web};

mod common;
use common::utils;
use common::multipart_builder;

#[tokio::test]
async fn test_user_isolation_workflow() {
    common::init_log();
    let config = utils::create_test_config();
    let (pool, _db_instance) = reminisce::test_utils::setup_empty_test_database_with_instance().await;
    let main_pool = utils::wrap_main_pool(pool.clone());
    let geo_pool = utils::create_mock_geotagging_pool(pool.clone());

    let app = test::init_service(
        App::new()
            .app_data(web::Data::new(main_pool.clone()))
            .app_data(web::Data::new(geo_pool.clone()))
            .app_data(web::Data::new(config.clone()))
            .service(reminisce::services::auth::setup_admin)
            .service(reminisce::services::auth::user_login)
            .service(reminisce::services::user_management::create_user)
            .service(reminisce::services::upload::upload_image)
            .service(reminisce::services::thumbnail::list_all_media_thumbnails)
            .service(reminisce::services::media::get_image_metadata)
    ).await;

    // 1. Create User A via setup (first admin), then User B via admin create_user
    let req = test::TestRequest::post()
        .uri("/auth/setup")
        .set_json(&serde_json::json!({"username": "user_a", "password": "password123"}))
        .to_request();
    let resp = test::call_service(&app, req).await;
    assert_eq!(resp.status(), actix_web::http::StatusCode::CREATED);

    // Login as user_a (admin) to get token for creating user_b
    let req = test::TestRequest::post()
        .uri("/auth/user-login")
        .set_json(&UserLoginRequest { username: "user_a".to_string(), password: "password123".to_string() })
        .to_request();
    let resp = test::call_service(&app, req).await;
    let body_a: serde_json::Value = test::read_body_json(resp).await;
    let token_a = body_a["access_token"].as_str().unwrap().to_string();

    // Create user_b via admin endpoint
    let req = test::TestRequest::post()
        .uri("/users")
        .insert_header(("Authorization", format!("Bearer {}", token_a)))
        .set_json(&serde_json::json!({"username": "user_b", "password": "password123", "role": "user"}))
        .to_request();
    let resp = test::call_service(&app, req).await;
    assert_eq!(resp.status(), actix_web::http::StatusCode::CREATED);

    // Login as user_b
    let req = test::TestRequest::post()
        .uri("/auth/user-login")
        .set_json(&UserLoginRequest { username: "user_b".to_string(), password: "password123".to_string() })
        .to_request();
    let resp = test::call_service(&app, req).await;
    let body_b: serde_json::Value = test::read_body_json(resp).await;
    let token_b = body_b["access_token"].as_str().unwrap().to_string();

    // 2. User A uploads an image
    let image_data = fs::read("tests/test_image.jpg").await.unwrap();
    let (payload, content_type) = multipart_builder::create_multipart_payload_without_thumbnail(
        common::TEST_IMAGE_HASH,
        common::TEST_IMAGE_NAME,
        &image_data
    );

    let req = test::TestRequest::post()
        .uri("/upload/image")
        .insert_header(("Authorization", format!("Bearer {}", token_a)))
        .insert_header(("Content-Type", content_type))
        .set_payload(payload)
        .to_request();

    let resp = test::call_service(&app, req).await;
    assert_eq!(resp.status(), actix_web::http::StatusCode::CREATED);

    // Manually set has_thumbnail = true to simulate verification worker
    let client = pool.get().await.unwrap();
    client.execute("UPDATE images SET has_thumbnail = true WHERE hash = $1", &[&common::TEST_IMAGE_HASH]).await.unwrap();

    // 3. Verify User A can see it
    let req = test::TestRequest::get()
        .uri("/media_thumbnails")
        .insert_header(("Authorization", format!("Bearer {}", token_a)))
        .to_request();
    let resp = test::call_service(&app, req).await;
    let body: ThumbnailsResponse = test::read_body_json(resp).await;
    assert_eq!(body.total, 1);

    // 4. Verify User B CANNOT see it
    let req = test::TestRequest::get()
        .uri("/media_thumbnails")
        .insert_header(("Authorization", format!("Bearer {}", token_b)))
        .to_request();
    let resp = test::call_service(&app, req).await;
    let body: ThumbnailsResponse = test::read_body_json(resp).await;
    assert_eq!(body.total, 0);

    // 5. Verify User B CANNOT access User A's image metadata directly
    let req = test::TestRequest::get()
        .uri(&format!("/image/{}/metadata", common::TEST_IMAGE_HASH))
        .insert_header(("Authorization", format!("Bearer {}", token_b)))
        .to_request();
    let resp = test::call_service(&app, req).await;
    assert_eq!(resp.status(), actix_web::http::StatusCode::NOT_FOUND);

    // Cleanup
    let _ = fs::remove_dir_all("uploaded_images_test").await;
}
