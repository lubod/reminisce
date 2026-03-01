use reminisce::services::thumbnail::ThumbnailsResponse;
use tokio::fs;
use reminisce::services::auth::{RegisterRequest, UserLoginRequest};
use actix_web::{test, App, web};

mod common;
use common::utils;
use common::multipart_builder;

#[tokio::test]
async fn test_user_isolation_workflow() {
    common::init_log();
    let config = utils::create_test_config();
    let (pool, _db_instance) = reminisce::test_utils::setup_test_database_with_instance().await;
    let main_pool = utils::wrap_main_pool(pool.clone());
    let geo_pool = utils::create_mock_geotagging_pool(pool.clone());
    
    let app = test::init_service(
        App::new()
            .app_data(web::Data::new(main_pool.clone()))
            .app_data(web::Data::new(geo_pool.clone()))
            .app_data(web::Data::new(config.clone()))
            .service(reminisce::services::auth::register_user)
            .service(reminisce::services::auth::user_login)
            .service(reminisce::services::upload::upload_image)
            .service(reminisce::services::thumbnail::list_all_media_thumbnails)
            .service(reminisce::services::media::get_image_metadata)
    ).await;

    // 1. Register User A and User B
    let user_a_creds = RegisterRequest {
        username: "user_a".to_string(),
        email: "user_a@example.com".to_string(),
        password: "password123".to_string(),
    };
    let user_b_creds = RegisterRequest {
        username: "user_b".to_string(),
        email: "user_b@example.com".to_string(),
        password: "password123".to_string(),
    };

    let req = test::TestRequest::post()
        .uri("/auth/register")
        .set_json(&user_a_creds)
        .to_request();
    let resp = test::call_service(&app, req).await;
    assert_eq!(resp.status(), actix_web::http::StatusCode::CREATED);

    let req = test::TestRequest::post()
        .uri("/auth/register")
        .set_json(&user_b_creds)
        .to_request();
    let resp = test::call_service(&app, req).await;
    assert_eq!(resp.status(), actix_web::http::StatusCode::CREATED);

    // 2. Login to get tokens
    let login_a = UserLoginRequest {
        username: "user_a".to_string(),
        password: "password123".to_string(),
    };
    let req = test::TestRequest::post()
        .uri("/auth/user-login")
        .set_json(&login_a)
        .to_request();
    let resp = test::call_service(&app, req).await;
    let body_a: serde_json::Value = test::read_body_json(resp).await;
    let token_a = body_a["access_token"].as_str().unwrap();

    let login_b = UserLoginRequest {
        username: "user_b".to_string(),
        password: "password123".to_string(),
    };
    let req = test::TestRequest::post()
        .uri("/auth/user-login")
        .set_json(&login_b)
        .to_request();
    let resp = test::call_service(&app, req).await;
    let body_b: serde_json::Value = test::read_body_json(resp).await;
    let token_b = body_b["access_token"].as_str().unwrap();

    // 3. User A uploads an image
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

    // Manually set has_thumbnail = true to simulate verification worker for thumbnail listing
    let client = pool.get().await.unwrap();
    client.execute("UPDATE images SET has_thumbnail = true WHERE hash = $1", &[&common::TEST_IMAGE_HASH]).await.unwrap();

    // 4. Verify User A can see it
    let req = test::TestRequest::get()
        .uri("/media_thumbnails")
        .insert_header(("Authorization", format!("Bearer {}", token_a)))
        .to_request();
    let resp = test::call_service(&app, req).await;
    let body: ThumbnailsResponse = test::read_body_json(resp).await;
    assert_eq!(body.total, 1);

    // 5. Verify User B CANNOT see it
    let req = test::TestRequest::get()
        .uri("/media_thumbnails")
        .insert_header(("Authorization", format!("Bearer {}", token_b)))
        .to_request();
    let resp = test::call_service(&app, req).await;
    let body: ThumbnailsResponse = test::read_body_json(resp).await;
    assert_eq!(body.total, 0);

    // 6. Verify User B CANNOT access User A's image metadata directly
    let req = test::TestRequest::get()
        .uri(&format!("/image/{}/metadata", common::TEST_IMAGE_HASH))
        .insert_header(("Authorization", format!("Bearer {}", token_b)))
        .to_request();
    let resp = test::call_service(&app, req).await;
    // Non-admin User B shouldn't be able to find it because it filters by their deviceid
    assert_eq!(resp.status(), actix_web::http::StatusCode::NOT_FOUND);

    // Cleanup
    let _ = fs::remove_dir_all("uploaded_images_test").await;
}