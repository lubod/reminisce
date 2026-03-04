use actix_web::{http, test, web, App};
use reminisce::services::import_dir::import_directory;
use reminisce::test_utils::setup_test_database_with_instance;
use serial_test::serial;
use std::fs;
use std::time::{SystemTime, Duration};
use chrono::{DateTime, Utc};
use uuid::Uuid;

mod common;

#[actix_web::test]
#[serial]
async fn test_import_directory_success() {
    common::init_log();
    let (pool, _test_db) = setup_test_database_with_instance().await;
    let config = common::utils::create_test_config();

    // Wrap pools
    let main_pool = common::utils::wrap_main_pool(pool.clone());
    let geotagging_pool = common::utils::create_geotagging_pool().await;

    let app = test::init_service(
        App::new()
            .app_data(web::Data::new(main_pool.clone()))
            .app_data(web::Data::new(geotagging_pool.clone()))
            .app_data(web::Data::new(config.clone()))
            .service(import_directory)
    ).await;

    // Create a temporary directory for import
    let import_base_path = std::env::temp_dir().join(format!("reminisce_test_import_{}", Uuid::new_v4()));
    fs::create_dir_all(&import_base_path).unwrap();

    // Create a dummy image file
    let image_filename = "test_import.jpg";
    let image_path = import_base_path.join(image_filename);
    
    // Use the test image from the project root
    let source_image = fs::read("tests/test_image.jpg").expect("Failed to read test image");
    fs::write(&image_path, &source_image).unwrap();

    // Authenticate
    let token = common::utils::create_test_jwt_token().await;

    // Prepare Request
    let req = test::TestRequest::post()
        .uri("/import_directory")
        .insert_header(("Authorization", format!("Bearer {}", token)))
        .set_json(serde_json::json!({
            "path": import_base_path.to_string_lossy(),
            "recursive": true
        }))
        .to_request();

    // Execute Request
    let response = test::call_service(&app, req).await;
    assert_eq!(response.status(), http::StatusCode::OK);

    let result: serde_json::Value = test::read_body_json(response).await;
    assert_eq!(result["scanned"], 1);
    assert_eq!(result["imported"], 1);
    assert_eq!(result["failed"], 0);

    // Verify DB
    let client = pool.get().await.expect("Failed to get client");
    let rows = client.query("SELECT hash, name FROM images WHERE name = $1", &[&image_filename]).await.unwrap();
    assert_eq!(rows.len(), 1);
    
    // Cleanup
    fs::remove_dir_all(&import_base_path).unwrap();
    client.execute("DELETE FROM images WHERE name = $1", &[&image_filename]).await.ok();
}

#[actix_web::test]
#[serial]
async fn test_import_directory_not_found() {
    common::init_log();
    let (pool, _test_db) = setup_test_database_with_instance().await;
    let config = common::utils::create_test_config();

    let main_pool = common::utils::wrap_main_pool(pool.clone());
    let geotagging_pool = common::utils::create_geotagging_pool().await;

    let app = test::init_service(
        App::new()
            .app_data(web::Data::new(main_pool.clone()))
            .app_data(web::Data::new(geotagging_pool.clone()))
            .app_data(web::Data::new(config.clone()))
            .service(import_directory)
    ).await;

    let token = common::utils::create_test_jwt_token().await;

    let req = test::TestRequest::post()
        .uri("/import_directory")
        .insert_header(("Authorization", format!("Bearer {}", token)))
        .set_json(serde_json::json!({
            "path": "/non/existent/path/for/sure/12345",
            "recursive": true
        }))
        .to_request();

    let response = test::call_service(&app, req).await;
    assert_eq!(response.status(), http::StatusCode::BAD_REQUEST);
}

/// Import a file whose name has no parseable date and no EXIF —
/// the server should fall back to the file's mtime rather than upload time.
#[actix_web::test]
#[serial]
async fn test_import_directory_uses_file_mtime_as_fallback() {
    common::init_log();
    let (pool, _test_db) = setup_test_database_with_instance().await;
    let config = common::utils::create_test_config();

    let main_pool = common::utils::wrap_main_pool(pool.clone());
    let geotagging_pool = common::utils::create_geotagging_pool().await;

    let app = test::init_service(
        App::new()
            .app_data(web::Data::new(main_pool.clone()))
            .app_data(web::Data::new(geotagging_pool.clone()))
            .app_data(web::Data::new(config.clone()))
            .service(import_directory)
    ).await;

    let import_base_path = std::env::temp_dir().join(format!("reminisce_test_import_mtime_{}", Uuid::new_v4()));
    fs::create_dir_all(&import_base_path).unwrap();

    // Use a filename with no date pattern and EXIF-stripped image
    let image_filename = "photo_backup.jpg";
    let image_path = import_base_path.join(image_filename);
    let source_image = fs::read("tests/IMG-20210615-WA0000.jpg").expect("Failed to read test image");
    fs::write(&image_path, &source_image).unwrap();

    // Set the file's mtime to a known past date: 2018-04-20 10:00:00 UTC
    let known_mtime = SystemTime::UNIX_EPOCH + Duration::from_secs(1524218400); // 2018-04-20 10:00:00 UTC
    filetime::set_file_mtime(&image_path, filetime::FileTime::from_system_time(known_mtime)).unwrap();

    let token = common::utils::create_test_jwt_token().await;

    let req = test::TestRequest::post()
        .uri("/import_directory")
        .insert_header(("Authorization", format!("Bearer {}", token)))
        .set_json(serde_json::json!({
            "path": import_base_path.to_string_lossy(),
            "recursive": false
        }))
        .to_request();

    let response = test::call_service(&app, req).await;
    assert_eq!(response.status(), http::StatusCode::OK);

    let result: serde_json::Value = test::read_body_json(response).await;
    assert_eq!(result["imported"], 1, "Expected 1 imported file");

    // Verify created_at was set from mtime, not upload time
    let client = pool.get().await.expect("Failed to get client");
    let row = client.query_one("SELECT created_at FROM images WHERE name = $1", &[&image_filename])
        .await.expect("Failed to query");
    let created_at: DateTime<Utc> = row.get(0);
    let expected: DateTime<Utc> = DateTime::from(known_mtime);
    let diff = (created_at - expected).num_seconds().abs();
    assert!(diff <= 2, "created_at should come from file mtime, got {} expected ~{}", created_at, expected);

    fs::remove_dir_all(&import_base_path).unwrap();
    client.execute("DELETE FROM images WHERE name = $1", &[&image_filename]).await.ok();
}
