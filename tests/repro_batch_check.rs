use actix_web::{http, test, web, App};
use reminisce::*;
use reminisce::test_utils::setup_test_database_with_instance;
use serial_test::serial;

mod common;

/// Reproduction test: Verify batch_check_images correctly reports existing hashes across devices
#[actix_web::test]
#[serial]
async fn test_repro_deduplication_across_devices() {
    common::init_log();
    let (pool, _test_db) = setup_test_database_with_instance().await;
    let client = pool.get().await.expect("Failed to get client from pool");
    let config = common::utils::create_test_config();

    // 1. Simulate Device A uploading a file (verified)
    let hash = "hash_xyz_123";
    let user_uuid = uuid::Uuid::parse_str("550e8400-e29b-41d4-a716-446655440000").unwrap();
    let device_a = "device_A";
    let device_b = "device_B";

    client
        .execute(
            "INSERT INTO images (user_id, hash, name, exif, created_at, type, deviceid, ext, verification_status) VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9)",
            &[
                &user_uuid,
                &hash,
                &"IMG_origin.jpg",
                &None::<&str>,
                &chrono::Utc::now(),
                &"camera",
                &device_a,
                &"jpg",
                &1i32,  // Verified!
            ]
        ).await
        .expect("Failed to insert initial data for Device A");

    let main_pool = common::utils::wrap_main_pool(pool.clone());
    let geotagging_pool = common::utils::create_geotagging_pool().await;

    let app = test::init_service(
        App::new()
            .app_data(web::Data::new(main_pool))
            .app_data(web::Data::new(geotagging_pool))
            .app_data(web::Data::new(config.clone()))
            .service(batch_check_images)
    ).await;

    // 2. Device B calls batch_check_images for the SAME hash
    let token_b = common::utils::create_test_jwt_token().await;

    let request_body = serde_json::json!({
        "device_id": device_b,
        "hashes": [hash]
    });

    let req = test::TestRequest::post()
        .uri("/upload/batch-check-images")
        .insert_header(("Authorization", format!("Bearer {}", token_b)))
        .insert_header(("Content-Type", "application/json"))
        .set_json(&request_body)
        .to_request();

    let response = test::call_service(&app, req).await;
    assert_eq!(response.status(), http::StatusCode::OK);

    let body: serde_json::Value = test::read_body_json(response).await;

    println!("Response Body: {:?}", body);

    // Under user-based scoping, batch_check returns hashes existing for the user across devices
    let existing = body["existing_hashes"].as_array().unwrap();
    assert_eq!(existing.len(), 1, "Should exist for user");

    // 3. Now insert a media source record for Device B and check again
    client
        .execute(
            "INSERT INTO media_sources (user_id, hash, media_type, device_id, uploaded_at) VALUES ($1, $2, 'image', $3, NOW()) ON CONFLICT DO NOTHING",
            &[&user_uuid, &hash, &device_b],
        ).await
        .expect("Failed to insert media source for Device B");

    let req2 = test::TestRequest::post()
        .uri("/upload/batch-check-images")
        .insert_header(("Authorization", format!("Bearer {}", token_b)))
        .insert_header(("Content-Type", "application/json"))
        .set_json(&request_body)
        .to_request();

    let response2 = test::call_service(&app, req2).await;
    assert_eq!(response2.status(), http::StatusCode::OK);

    let body2: serde_json::Value = test::read_body_json(response2).await;
    let existing2 = body2["existing_hashes"].as_array().unwrap();
    assert_eq!(existing2.len(), 1, "Should now exist for Device B");
    assert!(existing2.contains(&serde_json::json!(hash)));
}
