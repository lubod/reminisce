use actix_web::{http, test, web, App};
use reminisce::*;
use reminisce::test_utils::setup_test_database_with_instance;
use serial_test::serial;
use std::fs;
use std::path::Path;

mod common;

/// Complex test that demonstrates the full deduplication workflow across devices:
/// 1. Device1 uploads an image with full file data
/// 2. Image is marked as verified in the database (simulating verification worker)
/// 3. Device2 checks if image exists - should get exists=false, exists_without_deviceid=true
/// 4. Device2 uploads only metadata (not the full file)
/// 5. Verify both device1 and device2 have entries in the database with the same hash
#[actix_web::test]
#[serial]
async fn test_cross_device_deduplication_workflow() {
    common::init_log();
    let (pool, _test_db) = setup_test_database_with_instance().await;
    let config = common::utils::create_test_config();

    // Wrap pools for dependency injection
    let main_pool = common::utils::wrap_main_pool(pool.clone());
    let geotagging_pool = common::utils::create_geotagging_pool().await;

    // Create test app with all necessary services
    let app = test::init_service(
        App::new()
            .app_data(web::Data::new(main_pool))
            .app_data(web::Data::new(geotagging_pool))
            .app_data(web::Data::new(config.clone()))
            .service(check_image_exists)
            .service(upload_image)
            .service(upload_image_metadata)
    )
    .await;

    let device1 = "device1_test";
    let device2 = "device2_test";
    // Use actual BLAKE3 hash of tests/test_image.jpg
    let test_hash = "af29ca6fd22f34f3c51c3dc5326ff277b80ad6344a3a9af35bb5548ccf8cdb16";
    let test_name_device1 = "IMG_20231222_101010.jpg";
    let test_name_device2 = "/sdcard/DCIM/Camera/IMG_20231222_101010.jpg";

    // ============================================================
    // STEP 1: Device1 uploads the full image file with thumbnail
    // ============================================================
    log::info!("STEP 1: Device1 uploading full image file");

    let token_device1 = common::utils::create_test_jwt_token().await;
    let image_bytes = fs::read("tests/test_image.jpg").unwrap();
    let thumbnail_bytes = fs::read("tests/test_thumbnail.jpg").unwrap();

    let (form, content_type) = common::multipart_builder::create_multipart_payload_with_device_id(
        test_hash,
        test_name_device1,
        &image_bytes,
        &thumbnail_bytes,
        device1,
    );

    let upload_req = test::TestRequest::post()
        .uri("/upload/image")
        .insert_header(("Authorization", format!("Bearer {}", token_device1)))
        .insert_header(("Content-Type", content_type))
        .set_payload(form)
        .to_request();

    let upload_response = test::call_service(&app, upload_req).await;
    assert_eq!(
        upload_response.status(),
        http::StatusCode::CREATED,
        "Device1 image upload should succeed"
    );

    log::info!("Device1 upload successful");

    // Verify the file was saved to disk
    let subdir = &test_hash[..2];
    let file_path = Path::new(common::TEST_UPLOAD_DIR)
        .join(subdir)
        .join(format!("{}.jpg", test_hash));
    assert!(file_path.exists(), "Image file should exist on disk");

    // ============================================================
    // STEP 2: Simulate verification worker - mark image as verified
    // ============================================================
    log::info!("STEP 2: Marking image as verified (simulating verification worker)");

    let client = pool.get().await.expect("Failed to get database client");
    client
        .execute(
            "UPDATE images SET verification_status = 1, last_verified_at = NOW() WHERE hash = $1 AND deviceid = $2",
            &[&test_hash, &device1],
        )
        .await
        .expect("Failed to update verification status");

    // Verify the database entry for device1
    let row = client
        .query_one(
            "SELECT deviceid, name, ext, verification_status FROM images WHERE hash = $1 AND deviceid = $2",
            &[&test_hash, &device1],
        )
        .await
        .expect("Failed to query database for device1");

    let db_deviceid: &str = row.get(0);
    let db_name: &str = row.get(1);
    let db_ext: &str = row.get(2);
    let db_verification: i32 = row.get(3);

    assert_eq!(db_deviceid, device1);
    assert_eq!(db_name, test_name_device1);
    assert_eq!(db_ext, "jpg");
    assert_eq!(db_verification, 1, "Verification status should be 1 (verified)");

    log::info!("Image marked as verified for device1");

    // ============================================================
    // STEP 3: Device2 checks if image exists by hash
    // ============================================================
    log::info!("STEP 3: Device2 checking if image exists");

    let token_device2 = common::utils::create_test_jwt_token().await;

    let check_req = test::TestRequest::get()
        .uri(&format!("/check_image_exists?hash={}&device_id={}", test_hash, device2))
        .insert_header(("Authorization", format!("Bearer {}", token_device2)))
        .to_request();

    let check_response = test::call_service(&app, check_req).await;
    assert_eq!(
        check_response.status(),
        http::StatusCode::OK,
        "Check existence should succeed"
    );

    let check_body: serde_json::Value = test::read_body_json(check_response).await;

    // This is the key assertion - file exists for another device but not for this device
    assert_eq!(
        check_body["exists"], false,
        "Image should NOT exist for device2 (exists field should be false)"
    );
    assert_eq!(
        check_body["exists_without_deviceid"], true,
        "Image SHOULD exist without device ID (another verified device has it)"
    );

    log::info!(
        "Existence check result - exists: {}, exists_without_deviceid: {}",
        check_body["exists"],
        check_body["exists_without_deviceid"]
    );

    // ============================================================
    // STEP 4: Device2 uploads only metadata (not the full file)
    // ============================================================
    log::info!("STEP 4: Device2 uploading metadata only");

    let metadata_body = serde_json::json!({
        "deviceid": device2,
        "hash": test_hash,
        "name": test_name_device2,
        "ext": "jpg",
    });

    let metadata_req = test::TestRequest::post()
        .uri("/upload/image/metadata")
        .insert_header(("Authorization", format!("Bearer {}", token_device2)))
        .set_json(&metadata_body)
        .to_request();

    let metadata_response = test::call_service(&app, metadata_req).await;
    assert_eq!(
        metadata_response.status(),
        http::StatusCode::OK,
        "Metadata upload should succeed"
    );

    let metadata_response_body: serde_json::Value = test::read_body_json(metadata_response).await;
    assert_eq!(metadata_response_body["status"], "success");

    log::info!("Metadata upload successful for device2");

    // ============================================================
    // STEP 5: Verify both entries exist in database
    // ============================================================
    log::info!("STEP 5: Verifying both device entries exist in database");

    let all_rows = client
        .query(
            "SELECT deviceid, name, ext, verification_status FROM images WHERE hash = $1 ORDER BY deviceid",
            &[&test_hash],
        )
        .await
        .expect("Failed to query all entries for hash");

    assert_eq!(
        all_rows.len(),
        2,
        "Should have exactly 2 entries in database (one per device)"
    );

    // Verify device1 entry
    let device1_row = all_rows
        .iter()
        .find(|row| row.get::<_, &str>("deviceid") == device1)
        .expect("Device1 entry should exist");

    assert_eq!(device1_row.get::<_, &str>("name"), test_name_device1);
    assert_eq!(device1_row.get::<_, &str>("ext"), "jpg");
    assert_eq!(device1_row.get::<_, i32>("verification_status"), 1);

    // Verify device2 entry
    let device2_row = all_rows
        .iter()
        .find(|row| row.get::<_, &str>("deviceid") == device2)
        .expect("Device2 entry should exist");

    assert_eq!(device2_row.get::<_, &str>("name"), test_name_device2);
    assert_eq!(device2_row.get::<_, &str>("ext"), "jpg");
    // Device2's verification status should be 0 (pending) since it only uploaded metadata
    assert_eq!(device2_row.get::<_, i32>("verification_status"), 0);

    log::info!("✅ SUCCESS: Both device entries verified in database");
    log::info!("  - Device1: {} (verified)", test_name_device1);
    log::info!("  - Device2: {} (pending verification)", test_name_device2);

    // ============================================================
    // STEP 6: Device2 checks existence again - should now return true
    // ============================================================
    log::info!("STEP 6: Device2 checking existence again (should now exist)");

    let check_req2 = test::TestRequest::get()
        .uri(&format!("/check_image_exists?hash={}&device_id={}", test_hash, device2))
        .insert_header(("Authorization", format!("Bearer {}", token_device2)))
        .to_request();

    let check_response2 = test::call_service(&app, check_req2).await;
    assert_eq!(check_response2.status(), http::StatusCode::OK);

    let check_body2: serde_json::Value = test::read_body_json(check_response2).await;

    assert_eq!(
        check_body2["exists"], true,
        "Image should NOW exist for device2 after metadata upload"
    );
    assert_eq!(
        check_body2["exists_without_deviceid"], true,
        "Image should still exist without device ID"
    );

    log::info!("✅ Final existence check confirms device2 now has the image");

    // ============================================================
    // CLEANUP
    // ============================================================
    log::info!("Cleaning up test data");

    // Clean up database entries
    client
        .execute("DELETE FROM images WHERE hash = $1", &[&test_hash])
        .await
        .expect("Failed to clean up database");

    // Clean up file on disk
    if file_path.exists() {
        fs::remove_file(&file_path).unwrap();
    }

    // Clean up thumbnail
    let thumb_path = Path::new(common::TEST_UPLOAD_DIR)
        .join(subdir)
        .join(format!("{}.thumb.jpg", test_hash));
    if thumb_path.exists() {
        fs::remove_file(&thumb_path).unwrap();
    }

    log::info!("Test completed successfully! ✅");
}
