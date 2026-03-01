use actix_web::{http, test, web, App};
use reminisce::*;
use reminisce::test_utils::setup_test_database_with_instance;
use chrono;
use serial_test::serial;
mod common;
mod utils;

const TEST_IMAGE_HASH: &str = "test_image_hash_metadata";
const TEST_IMAGE_NAME: &str = "test_image_name_metadata.jpg";

#[actix_web::test]
#[serial]
async fn test_upload_image_metadata_success() {
    common::init_log();
    let (pool, _test_db) = setup_test_database_with_instance().await;
    let client = pool.get().await.expect("Failed to get client from pool");

    let config = utils::create_test_config();

    // Insert a dummy image record with verification_status = 1 (verified)
    client
        .execute(
            "INSERT INTO images (hash, name, exif, created_at, type, deviceid, ext, has_thumbnail, verification_status) VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9)",
            &[
                &TEST_IMAGE_HASH,
                &TEST_IMAGE_NAME,
                &None::<&str>,
                &chrono::Utc::now(),
                &"camera",
                &"original_device_id",
                &"jpg",
                &true,
                &1i32,
            ],
        )
        .await
        .expect("Failed to insert test data");


    // Wrap pools for dependency injection

    let main_pool = utils::wrap_main_pool(pool.clone());

    let geotagging_pool = utils::create_geotagging_pool().await;


    let app = test::init_service(
        App::new()
            .app_data(web::Data::new(main_pool.clone()))
            .app_data(web::Data::new(geotagging_pool.clone()))
            .app_data(web::Data::new(config.clone()))
            .service(upload_image_metadata),
    )
    .await;

    let token = utils::create_test_jwt_token().await;
    let new_name = "new_image_name.jpg";

    let request_body = serde_json::json!({
        "deviceid": "new_device_id",
        "hash": TEST_IMAGE_HASH,
        "name": new_name,
        "ext": "jpg",
    });

    let req = test::TestRequest::post()
        .uri("/upload/image/metadata")
        .insert_header(("Authorization", format!("Bearer {}", token)))
        .set_json(&request_body)
        .to_request();

    let response = test::call_service(&app, req).await;

    assert_eq!(response.status(), http::StatusCode::OK);

    let body: serde_json::Value = test::read_body_json(response).await;
    assert_eq!(body["status"], "success");

    // Verify that a new record was created for the new device
    let rows = client
        .query(
            "SELECT name, deviceid FROM images WHERE hash = $1",
            &[&TEST_IMAGE_HASH],
        )
        .await
        .expect("Failed to query database");

    assert_eq!(rows.len(), 2);

    let new_device_row = rows.iter().find(|row| row.get::<_, String>("deviceid") == "new_device_id").unwrap();
    assert_eq!(new_device_row.get::<_, &str>("name"), new_name);

    // Clean up
    client
        .execute("DELETE FROM images WHERE hash = $1", &[&TEST_IMAGE_HASH])
        .await
        .expect("Failed to clean up database");
}