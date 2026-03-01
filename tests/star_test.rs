use actix_web::{ http, test, web, App };
use reminisce::*;
use reminisce::test_utils::setup_test_database_with_instance;
use serial_test::serial;
use std::fs;
use std::path::Path;
use serde_json::Value;

mod common;

#[actix_web::test]
#[serial]
async fn test_star_image() {
    common::init_log();
    let (pool, _test_db) = setup_test_database_with_instance().await;

    let config = common::utils::create_test_config();

    // Wrap pools for dependency injection
    let main_pool = common::utils::wrap_main_pool(pool.clone());
    let geotagging_pool = common::utils::create_geotagging_pool().await;

    let app = test::init_service(
        App::new()
            .app_data(web::Data::new(main_pool.clone()))
            .app_data(web::Data::new(geotagging_pool.clone()))
            .app_data(web::Data::new(config.clone()))
            .service(upload_image)
            .service(toggle_image_star)
            .service(get_image_metadata)
    ).await;

    let token = common::utils::create_test_jwt_token().await;

    // First upload an image
    let image_bytes = fs::read("tests/test_image.jpg").unwrap();
    let thumbnail_bytes = fs::read("tests/test_thumbnail.jpg").unwrap();

    let (form, content_type) = common::multipart_builder::create_multipart_payload(
        common::TEST_IMAGE_HASH,
        common::TEST_IMAGE_NAME,
        &image_bytes,
        &thumbnail_bytes
    );

    let upload_req = test::TestRequest
        ::post()
        .uri("/upload/image")
        .insert_header(("Authorization", format!("Bearer {}", token)))
        .insert_header(("Content-Type", content_type))
        .set_payload(form)
        .to_request();

    let upload_response = test::call_service(&app, upload_req).await;
    assert_eq!(upload_response.status(), http::StatusCode::CREATED);

    // Now star the image
    let star_req = test::TestRequest
        ::post()
        .uri(&format!("/image/{}/star", common::TEST_IMAGE_HASH))
        .insert_header(("Authorization", format!("Bearer {}", token)))
        .to_request();

    let star_response = test::call_service(&app, star_req).await;
    assert_eq!(star_response.status(), http::StatusCode::OK);

    // Verify response body
    let response_body = test::read_body(star_response).await;
    let json: Value = serde_json::from_slice(&response_body).unwrap();
    assert_eq!(json["hash"], common::TEST_IMAGE_HASH);
    assert_eq!(json["starred"], true);

    // Verify in database
    let client = pool.get().await.expect("Failed to get client from pool");
    let row = client
        .query_opt("SELECT 1 FROM starred_images WHERE hash = $1", &[&common::TEST_IMAGE_HASH]).await
        .expect("Failed to query database");
    assert!(row.is_some());

    // Verify metadata endpoint returns starred status
    let metadata_req = test::TestRequest
        ::get()
        .uri(&format!("/image/{}/metadata", common::TEST_IMAGE_HASH))
        .insert_header(("Authorization", format!("Bearer {}", token)))
        .to_request();

    let metadata_response = test::call_service(&app, metadata_req).await;
    assert_eq!(metadata_response.status(), http::StatusCode::OK);

    let metadata_body = test::read_body(metadata_response).await;
    let metadata_json: Value = serde_json::from_slice(&metadata_body).unwrap();
    assert_eq!(metadata_json["starred"], true);

    // Clean up
    let subdir = &common::TEST_IMAGE_HASH[..2];
    let file_path = Path::new(common::TEST_UPLOAD_DIR)
        .join(subdir)
        .join(format!("{}.jpg", common::TEST_IMAGE_HASH));
    if file_path.exists() {
        fs::remove_file(&file_path).unwrap();
    }

    client
        .execute("DELETE FROM starred_images WHERE hash = $1", &[&common::TEST_IMAGE_HASH]).await
        .expect("Failed to clean up starred_images");
    client
        .execute("DELETE FROM images WHERE hash = $1", &[&common::TEST_IMAGE_HASH]).await
        .expect("Failed to clean up images");
}

#[actix_web::test]
#[serial]
async fn test_toggle_star_image() {
    common::init_log();
    let (pool, _test_db) = setup_test_database_with_instance().await;

    let config = common::utils::create_test_config();

    // Wrap pools for dependency injection
    let main_pool = common::utils::wrap_main_pool(pool.clone());
    let geotagging_pool = common::utils::create_geotagging_pool().await;

    let app = test::init_service(
        App::new()
            .app_data(web::Data::new(main_pool.clone()))
            .app_data(web::Data::new(geotagging_pool.clone()))
            .app_data(web::Data::new(config.clone()))
            .service(upload_image)
            .service(toggle_image_star)
    ).await;

    let token = common::utils::create_test_jwt_token().await;

    // First upload an image
    let image_bytes = fs::read("tests/test_image.jpg").unwrap();
    let thumbnail_bytes = fs::read("tests/test_thumbnail.jpg").unwrap();

    let (form, content_type) = common::multipart_builder::create_multipart_payload(
        common::TEST_IMAGE_HASH,
        common::TEST_IMAGE_NAME,
        &image_bytes,
        &thumbnail_bytes
    );

    let upload_req = test::TestRequest
        ::post()
        .uri("/upload/image")
        .insert_header(("Authorization", format!("Bearer {}", token)))
        .insert_header(("Content-Type", content_type))
        .set_payload(form)
        .to_request();

    let upload_response = test::call_service(&app, upload_req).await;
    assert_eq!(upload_response.status(), http::StatusCode::CREATED);

    // Star the image (first toggle)
    let star_req1 = test::TestRequest
        ::post()
        .uri(&format!("/image/{}/star", common::TEST_IMAGE_HASH))
        .insert_header(("Authorization", format!("Bearer {}", token)))
        .to_request();

    let star_response1 = test::call_service(&app, star_req1).await;
    assert_eq!(star_response1.status(), http::StatusCode::OK);

    let response_body1 = test::read_body(star_response1).await;
    let json1: Value = serde_json::from_slice(&response_body1).unwrap();
    assert_eq!(json1["starred"], true);

    // Unstar the image (second toggle)
    let star_req2 = test::TestRequest
        ::post()
        .uri(&format!("/image/{}/star", common::TEST_IMAGE_HASH))
        .insert_header(("Authorization", format!("Bearer {}", token)))
        .to_request();

    let star_response2 = test::call_service(&app, star_req2).await;
    assert_eq!(star_response2.status(), http::StatusCode::OK);

    let response_body2 = test::read_body(star_response2).await;
    let json2: Value = serde_json::from_slice(&response_body2).unwrap();
    assert_eq!(json2["starred"], false);

    // Verify removed from database
    let client = pool.get().await.expect("Failed to get client from pool");
    let row = client
        .query_opt("SELECT 1 FROM starred_images WHERE hash = $1", &[&common::TEST_IMAGE_HASH]).await
        .expect("Failed to query database");
    assert!(row.is_none());

    // Clean up
    let subdir = &common::TEST_IMAGE_HASH[..2];
    let file_path = Path::new(common::TEST_UPLOAD_DIR)
        .join(subdir)
        .join(format!("{}.jpg", common::TEST_IMAGE_HASH));
    if file_path.exists() {
        fs::remove_file(&file_path).unwrap();
    }

    client
        .execute("DELETE FROM images WHERE hash = $1", &[&common::TEST_IMAGE_HASH]).await
        .expect("Failed to clean up images");
}

#[actix_web::test]
#[serial]
async fn test_star_nonexistent_image() {
    common::init_log();
    let (pool, _test_db) = setup_test_database_with_instance().await;

    let config = common::utils::create_test_config();

    // Wrap pools for dependency injection
    let main_pool = common::utils::wrap_main_pool(pool.clone());
    let geotagging_pool = common::utils::create_geotagging_pool().await;

    let app = test::init_service(
        App::new()
            .app_data(web::Data::new(main_pool.clone()))
            .app_data(web::Data::new(geotagging_pool.clone()))
            .app_data(web::Data::new(config.clone()))
            .service(toggle_image_star)
    ).await;

    let token = common::utils::create_test_jwt_token().await;

    // Try to star a non-existent image
    let star_req = test::TestRequest
        ::post()
        .uri("/image/nonexistent_hash/star")
        .insert_header(("Authorization", format!("Bearer {}", token)))
        .to_request();

    let star_response = test::call_service(&app, star_req).await;
    assert_eq!(star_response.status(), http::StatusCode::NOT_FOUND);
}

#[actix_web::test]
#[serial]
async fn test_star_image_no_auth() {
    let (pool, _test_db) = setup_test_database_with_instance().await;

    let config = common::utils::create_test_config();

    // Wrap pools for dependency injection
    let main_pool = common::utils::wrap_main_pool(pool.clone());
    let geotagging_pool = common::utils::create_geotagging_pool().await;

    let app = test::init_service(
        App::new()
            .app_data(web::Data::new(main_pool.clone()))
            .app_data(web::Data::new(geotagging_pool.clone()))
            .app_data(web::Data::new(config.clone()))
            .service(toggle_image_star)
    ).await;

    let req = test::TestRequest::post().uri("/image/some_hash/star").to_request();

    let response = test::call_service(&app, req).await;
    assert_eq!(response.status(), http::StatusCode::UNAUTHORIZED);
}

#[actix_web::test]
#[serial]
async fn test_starred_filter() {
    common::init_log();
    let (pool, _test_db) = setup_test_database_with_instance().await;

    let config = common::utils::create_test_config();

    // Wrap pools for dependency injection
    let main_pool = common::utils::wrap_main_pool(pool.clone());
    let geotagging_pool = common::utils::create_geotagging_pool().await;

    let app = test::init_service(
        App::new()
            .app_data(web::Data::new(main_pool.clone()))
            .app_data(web::Data::new(geotagging_pool.clone()))
            .app_data(web::Data::new(config.clone()))
            .service(upload_image)
            .service(toggle_image_star)
            .service(list_image_thumbnails)
    ).await;

    let token = common::utils::create_test_jwt_token().await;

    // Upload two images
    let image_bytes = fs::read("tests/test_image.jpg").unwrap();
    let thumbnail_bytes = fs::read("tests/test_thumbnail.jpg").unwrap();

    // Upload first image
    let (form1, content_type1) = common::multipart_builder::create_multipart_payload(
        common::TEST_IMAGE_HASH,
        common::TEST_IMAGE_NAME,
        &image_bytes,
        &thumbnail_bytes
    );

    let upload_req1 = test::TestRequest
        ::post()
        .uri("/upload/image")
        .insert_header(("Authorization", format!("Bearer {}", token)))
        .insert_header(("Content-Type", content_type1))
        .set_payload(form1)
        .to_request();

    test::call_service(&app, upload_req1).await;

    // Upload second image
    let image_bytes2 = fs::read("tests/test_image2.jpg").unwrap();
    let (form2, content_type2) = common::multipart_builder::create_multipart_payload(
        common::TEST_IMAGE_HASH2,
        common::TEST_IMAGE_NAME2,
        &image_bytes2,
        &thumbnail_bytes
    );

    let upload_req2 = test::TestRequest
        ::post()
        .uri("/upload/image")
        .insert_header(("Authorization", format!("Bearer {}", token)))
        .insert_header(("Content-Type", content_type2))
        .set_payload(form2)
        .to_request();

    test::call_service(&app, upload_req2).await;

    // Star only the first image
    let star_req = test::TestRequest
        ::post()
        .uri(&format!("/image/{}/star", common::TEST_IMAGE_HASH))
        .insert_header(("Authorization", format!("Bearer {}", token)))
        .to_request();

    test::call_service(&app, star_req).await;

    // Get all thumbnails (starred_only=false)
    let list_all_req = test::TestRequest
        ::get()
        .uri("/image_thumbnails?page=1&limit=50&starred_only=false")
        .insert_header(("Authorization", format!("Bearer {}", token)))
        .to_request();

    let list_all_response = test::call_service(&app, list_all_req).await;
    let all_body = test::read_body(list_all_response).await;
    let all_json: Value = serde_json::from_slice(&all_body).unwrap();
    let all_count = all_json["thumbnails"].as_array().unwrap().len();
    assert!(all_count >= 2); // At least our 2 test images

    // Get only starred thumbnails
    let list_starred_req = test::TestRequest
        ::get()
        .uri("/image_thumbnails?page=1&limit=50&starred_only=true")
        .insert_header(("Authorization", format!("Bearer {}", token)))
        .to_request();

    let list_starred_response = test::call_service(&app, list_starred_req).await;
    let starred_body = test::read_body(list_starred_response).await;
    let starred_json: Value = serde_json::from_slice(&starred_body).unwrap();
    let starred_thumbnails = starred_json["thumbnails"].as_array().unwrap();

    // Should have exactly 1 starred image
    assert!(starred_thumbnails.len() >= 1);

    // Verify the starred image is in the results
    let has_starred_image = starred_thumbnails.iter().any(|item| {
        item["hash"] == common::TEST_IMAGE_HASH && item["starred"] == true
    });
    assert!(has_starred_image);

    // Clean up
    let client = pool.get().await.expect("Failed to get client from pool");

    let subdir1 = &common::TEST_IMAGE_HASH[..2];
    let file_path1 = Path::new(common::TEST_UPLOAD_DIR)
        .join(subdir1)
        .join(format!("{}.jpg", common::TEST_IMAGE_HASH));
    if file_path1.exists() {
        fs::remove_file(&file_path1).unwrap();
    }

    let subdir2 = &common::TEST_IMAGE_HASH2[..2];
    let file_path2 = Path::new(common::TEST_UPLOAD_DIR)
        .join(subdir2)
        .join(format!("{}.jpg", common::TEST_IMAGE_HASH2));
    if file_path2.exists() {
        fs::remove_file(&file_path2).unwrap();
    }

    client
        .execute("DELETE FROM starred_images WHERE hash = $1", &[&common::TEST_IMAGE_HASH]).await
        .expect("Failed to clean up starred_images");
    client
        .execute("DELETE FROM images WHERE hash = $1", &[&common::TEST_IMAGE_HASH]).await
        .expect("Failed to clean up images");
    client
        .execute("DELETE FROM images WHERE hash = $1", &[&common::TEST_IMAGE_HASH2]).await
        .expect("Failed to clean up images");
}

#[actix_web::test]
#[serial]
async fn test_starred_video_filter() {
    common::init_log();
    let (pool, _test_db) = setup_test_database_with_instance().await;

    let config = common::utils::create_test_config();

    // Wrap pools for dependency injection
    let main_pool = common::utils::wrap_main_pool(pool.clone());
    let geotagging_pool = common::utils::create_geotagging_pool().await;

    let app = test::init_service(
        App::new()
            .app_data(web::Data::new(main_pool.clone()))
            .app_data(web::Data::new(geotagging_pool.clone()))
            .app_data(web::Data::new(config.clone()))
            .service(upload_video)
            .service(toggle_video_star)
            .service(list_video_thumbnails)
    ).await;

    let token = common::utils::create_test_jwt_token().await;

    // Upload two videos
    let video_bytes = fs::read("tests/test_video.mp4").unwrap();
    let thumbnail_bytes = fs::read("tests/test_thumbnail.jpg").unwrap();

    // Upload first video
    let (form1, content_type1) = common::multipart_builder::create_video_multipart_payload(
        common::TEST_VIDEO_HASH,
        common::TEST_VIDEO_NAME,
        &video_bytes,
        &thumbnail_bytes
    );

    let upload_req1 = test::TestRequest
        ::post()
        .uri("/upload/video")
        .insert_header(("Authorization", format!("Bearer {}", token)))
        .insert_header(("Content-Type", content_type1))
        .set_payload(form1)
        .to_request();

    test::call_service(&app, upload_req1).await;

    // Upload second video
    let video_bytes2 = fs::read("tests/test_video2.mp4").unwrap();
    let (form2, content_type2) = common::multipart_builder::create_video_multipart_payload(
        common::TEST_VIDEO_HASH2,
        common::TEST_VIDEO_NAME2,
        &video_bytes2,
        &thumbnail_bytes
    );

    let upload_req2 = test::TestRequest
        ::post()
        .uri("/upload/video")
        .insert_header(("Authorization", format!("Bearer {}", token)))
        .insert_header(("Content-Type", content_type2))
        .set_payload(form2)
        .to_request();

    test::call_service(&app, upload_req2).await;

    // Star only the first video
    let star_req = test::TestRequest
        ::post()
        .uri(&format!("/video/{}/star", common::TEST_VIDEO_HASH))
        .insert_header(("Authorization", format!("Bearer {}", token)))
        .to_request();

    test::call_service(&app, star_req).await;

    // Get all thumbnails (starred_only=false)
    let list_all_req = test::TestRequest
        ::get()
        .uri("/video_thumbnails?page=1&limit=50&starred_only=false")
        .insert_header(("Authorization", format!("Bearer {}", token)))
        .to_request();

    let list_all_response = test::call_service(&app, list_all_req).await;
    let all_body = test::read_body(list_all_response).await;
    let all_json: Value = serde_json::from_slice(&all_body).unwrap();
    let all_count = all_json["thumbnails"].as_array().unwrap().len();
    assert!(all_count >= 2); // At least our 2 test videos

    // Get only starred thumbnails
    let list_starred_req = test::TestRequest
        ::get()
        .uri("/video_thumbnails?page=1&limit=50&starred_only=true")
        .insert_header(("Authorization", format!("Bearer {}", token)))
        .to_request();

    let list_starred_response = test::call_service(&app, list_starred_req).await;
    let starred_body = test::read_body(list_starred_response).await;
    let starred_json: Value = serde_json::from_slice(&starred_body).unwrap();
    let starred_thumbnails = starred_json["thumbnails"].as_array().unwrap();
    
    assert!(starred_thumbnails.len() >= 1);

    // Verify the starred video is in the results
    let has_starred_video = starred_thumbnails.iter().any(|item| {
        item["hash"] == common::TEST_VIDEO_HASH && item["starred"] == true
    });
    assert!(has_starred_video);

    // Clean up
    let client = pool.get().await.expect("Failed to get client from pool");

    let subdir1 = &common::TEST_VIDEO_HASH[..2];
    let file_path1 = Path::new(common::TEST_VIDEOS_DIR)
        .join(subdir1)
        .join(format!("{}.mp4", common::TEST_VIDEO_HASH));
    if file_path1.exists() {
        fs::remove_file(&file_path1).unwrap();
    }

    let subdir2 = &common::TEST_VIDEO_HASH2[..2];
    let file_path2 = Path::new(common::TEST_VIDEOS_DIR)
        .join(subdir2)
        .join(format!("{}.mp4", common::TEST_VIDEO_HASH2));
    if file_path2.exists() {
        fs::remove_file(&file_path2).unwrap();
    }

    client
        .execute("DELETE FROM starred_videos WHERE hash = $1", &[&common::TEST_VIDEO_HASH]).await
        .expect("Failed to clean up starred_videos");
    client
        .execute("DELETE FROM videos WHERE hash = $1", &[&common::TEST_VIDEO_HASH]).await
        .expect("Failed to clean up videos");
    client
        .execute("DELETE FROM videos WHERE hash = $1", &[&common::TEST_VIDEO_HASH2]).await
        .expect("Failed to clean up videos");
}

