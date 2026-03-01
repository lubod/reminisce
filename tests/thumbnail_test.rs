use actix_web::{ http, test, web, App };
use reminisce::*;
use reminisce::test_utils::setup_test_database_with_instance;
use chrono;
use serial_test::serial;
use std::fs;
use std::path::Path;

mod common;

#[actix_web::test]
#[serial]
async fn test_list_image_thumbnails() {
    let (pool, _test_db) = setup_test_database_with_instance().await;
    let client = pool.get().await.expect("Failed to get client from pool");

    let config = common::utils::create_test_config();
    // Clean up any existing test data first
    client.execute("DELETE FROM images WHERE hash = $1", &[&common::TEST_THUMBNAILS_HASH]).await.ok();

    // Insert test data
    client
        .execute(
            "INSERT INTO images (hash, name, exif, created_at, type, deviceid, ext, has_thumbnail) VALUES ($1, $2, $3, $4, $5, $6, $7, $8)",
            &[
                &common::TEST_THUMBNAILS_HASH,
                &common::TEST_IMAGE_NAME,
                &None::<&str>,
                &chrono::Utc::now(),
                &"camera",
                &"test_device_id",
                &"jpg",
                &true,
            ]
        ).await
        .expect("Failed to insert test data");

    // Create dummy thumbnail file
    let subdir = &common::TEST_THUMBNAILS_HASH[..2];
    let sub_dir_path = Path::new(config.get_images_dir()).join(subdir);
    tokio::fs::create_dir_all(&sub_dir_path).await.unwrap();
    let thumb_path = format!(
        "{}/{}/{}.thumb.jpg",
        config.get_images_dir(),
        subdir,
        common::TEST_THUMBNAILS_HASH
    );
    let test_image_data = fs::read("tests/test_image.jpg").unwrap();
    fs::write(&thumb_path, &test_image_data).unwrap();


    // Wrap pools for dependency injection

    let main_pool = common::utils::wrap_main_pool(pool.clone());

    let geotagging_pool = common::utils::create_geotagging_pool().await;


    let app = test::init_service(
        App::new()
            .app_data(web::Data::new(main_pool.clone()))
            .app_data(web::Data::new(geotagging_pool.clone()))
            .app_data(web::Data::new(config.clone()))
            .service(list_image_thumbnails)
    ).await;

    let token = common::utils::create_test_jwt_token().await;

    let req = test::TestRequest
        ::get()
        .uri("/image_thumbnails?page=1&limit=10&image_type=camera")
        .insert_header(("Authorization", format!("Bearer {}", token)))
        .to_request();

    let response = test::call_service(&app, req).await;
    assert_eq!(response.status(), http::StatusCode::OK);

    let body: serde_json::Value = test::read_body_json(response).await;
    assert_eq!(body["total"], 1);
    assert_eq!(body["page"], 1);
    assert_eq!(body["limit"], 10);
    assert!(body["thumbnails"].is_array());
    let thumbnails = body["thumbnails"].as_array().unwrap();
    assert_eq!(thumbnails.len(), 1);

    // Verify the thumbnail item has hash and created_at
    let thumbnail_item = &thumbnails[0];
    assert!(thumbnail_item["hash"].is_string());
    assert!(thumbnail_item["created_at"].is_string());

    // Clean up
    fs::remove_file(&thumb_path).unwrap();
    client
        .execute("DELETE FROM images WHERE hash = $1", &[&common::TEST_THUMBNAILS_HASH]).await
        .expect("Failed to clean up database");
}

#[actix_web::test]
#[serial]
async fn test_list_image_thumbnails_invalid_token() {
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
            .service(list_image_thumbnails)
    ).await;

    let req = test::TestRequest
        ::get()
        .uri("/image_thumbnails?page=1&limit=10&image_type=camera")
        .insert_header(("Authorization", "Bearer invalid_token"))
        .to_request();

    let response = test::call_service(&app, req).await;
    assert_eq!(response.status(), http::StatusCode::UNAUTHORIZED);
}

#[actix_web::test]
#[serial]
async fn test_get_thumbnail_success() {
    let config = common::utils::create_test_config();
    // Create thumbnail file for test
    let subdir = &common::TEST_IMAGE_HASH[..2];
    let sub_dir_path = Path::new(config.get_images_dir()).join(subdir);
    tokio::fs::create_dir_all(&sub_dir_path).await.unwrap();
    let thumb_path = format!("{}/{}/{}.thumb.jpg", config.get_images_dir(), subdir, common::TEST_IMAGE_HASH);
    let test_image_data = fs::read("tests/test_image.jpg").unwrap();
    fs::write(&thumb_path, &test_image_data).unwrap();

    let app = test::init_service(
        App::new().app_data(web::Data::new(config.clone())).service(get_thumbnail)
    ).await;

    let token = common::utils::create_test_jwt_token().await;

    let req = test::TestRequest
        ::get()
        .uri(&format!("/thumbnail/{}", common::TEST_IMAGE_HASH))
        .insert_header(("Authorization", format!("Bearer {}", token)))
        .to_request();

    let response = test::call_service(&app, req).await;
    assert_eq!(response.status(), http::StatusCode::OK);

    let content_type = response.headers().get("content-type").unwrap().to_str().unwrap();
    assert_eq!(content_type, "image/jpeg");

    // Clean up
    fs::remove_file(&thumb_path).unwrap();
}

#[actix_web::test]
#[serial]
async fn test_get_thumbnail_not_found() {
    let config = common::utils::create_test_config();

    let app = test::init_service(
        App::new().app_data(web::Data::new(config.clone())).service(get_thumbnail)
    ).await;

    let token = common::utils::create_test_jwt_token().await;

    let req = test::TestRequest
        ::get()
        .uri("/thumbnail/nonexistent_hash")
        .insert_header(("Authorization", format!("Bearer {}", token)))
        .to_request();

    let response = test::call_service(&app, req).await;
    assert_eq!(response.status(), http::StatusCode::NOT_FOUND);
}

#[actix_web::test]
#[serial]
async fn test_get_thumbnail_invalid_token() {
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
            .service(get_thumbnail)
    ).await;

    let req = test::TestRequest
        ::get()
        .uri("/thumbnail/some_hash")
        .insert_header(("Authorization", "Bearer invalid_token"))
        .to_request();

    let response = test::call_service(&app, req).await;
    assert_eq!(response.status(), http::StatusCode::UNAUTHORIZED);
}

#[actix_web::test]
#[serial]
async fn test_list_video_thumbnails() {
    let (pool, _test_db) = setup_test_database_with_instance().await;
    let client = pool.get().await.expect("Failed to get client from pool");

    let config = common::utils::create_test_config();
    // Clean up any existing test data first
    client.execute("DELETE FROM videos WHERE hash = $1", &[&common::TEST_VIDEO_HASH]).await.ok();

    // Insert test data
    client
        .execute(
            "INSERT INTO videos (hash, name, metadata, created_at, type, deviceid, ext, has_thumbnail) VALUES ($1, $2, $3, $4, $5, $6, $7, $8)",
            &[
                &common::TEST_VIDEO_HASH,
                &common::TEST_VIDEO_NAME,
                &None::<&str>,
                &chrono::Utc::now(),
                &"camera",
                &"test_device_id",
                &"mp4",
                &true,
            ]
        ).await
        .expect("Failed to insert test data");

    // Create dummy thumbnail file
    let subdir = &common::TEST_VIDEO_HASH[..2];
    let sub_dir_path = Path::new(config.get_videos_dir()).join(subdir);
    tokio::fs::create_dir_all(&sub_dir_path).await.unwrap();
    let thumb_path = format!("{}/{}/{}.thumb.jpg", config.get_videos_dir(), subdir, common::TEST_VIDEO_HASH);
    let test_video_data = fs::read("tests/test_video.mp4").unwrap();
    fs::write(&thumb_path, &test_video_data).unwrap();


    // Wrap pools for dependency injection

    let main_pool = common::utils::wrap_main_pool(pool.clone());

    let geotagging_pool = common::utils::create_geotagging_pool().await;


    let app = test::init_service(
        App::new()
            .app_data(web::Data::new(main_pool.clone()))
            .app_data(web::Data::new(geotagging_pool.clone()))
            .app_data(web::Data::new(config.clone()))
            .service(list_video_thumbnails)
    ).await;

    let token = common::utils::create_test_jwt_token().await;

    let req = test::TestRequest
        ::get()
        .uri("/video_thumbnails?page=1&limit=10&image_type=camera")
        .insert_header(("Authorization", format!("Bearer {}", token)))
        .to_request();

    let response = test::call_service(&app, req).await;
    assert_eq!(response.status(), http::StatusCode::OK);

    let body: serde_json::Value = test::read_body_json(response).await;
    assert_eq!(body["total"], 1);
    assert_eq!(body["page"], 1);
    assert_eq!(body["limit"], 10);
    assert!(body["thumbnails"].is_array());
    let thumbnails = body["thumbnails"].as_array().unwrap();
    assert_eq!(thumbnails.len(), 1);

    // Verify the thumbnail item has hash and created_at
    let thumbnail_item = &thumbnails[0];
    assert!(thumbnail_item["hash"].is_string());
    assert!(thumbnail_item["created_at"].is_string());
    assert_eq!(thumbnail_item["hash"], common::TEST_VIDEO_HASH);

    // Clean up
    fs::remove_file(&thumb_path).unwrap();
    client
        .execute("DELETE FROM videos WHERE hash = $1", &[&common::TEST_VIDEO_HASH]).await
        .expect("Failed to clean up database");
}

#[actix_web::test]
#[serial]
async fn test_list_video_thumbnails_empty() {
    let (pool, _test_db) = setup_test_database_with_instance().await;
    let client = pool.get().await.expect("Failed to get client from pool");

    let config = common::utils::create_test_config();
    // Ensure there are no videos in the database
    client
        .execute("DELETE FROM videos WHERE deviceid = $1", &[&"test_device_id"]).await
        .expect("Failed to clean up videos");


    // Wrap pools for dependency injection

    let main_pool = common::utils::wrap_main_pool(pool.clone());

    let geotagging_pool = common::utils::create_geotagging_pool().await;


    let app = test::init_service(
        App::new()
            .app_data(web::Data::new(main_pool.clone()))
            .app_data(web::Data::new(geotagging_pool.clone()))
            .app_data(web::Data::new(config.clone()))
            .service(list_video_thumbnails)
    ).await;

    let token = common::utils::create_test_jwt_token().await;

    let req = test::TestRequest
        ::get()
        .uri("/video_thumbnails?page=1&limit=10&image_type=all")
        .insert_header(("Authorization", format!("Bearer {}", token)))
        .to_request();

    let response = test::call_service(&app, req).await;
    assert_eq!(response.status(), http::StatusCode::OK);

    let body: serde_json::Value = test::read_body_json(response).await;

    // Verify empty response structure
    assert_eq!(body["total"], 0);
    assert_eq!(body["page"], 1);
    assert_eq!(body["limit"], 10);
    assert!(body["thumbnails"].is_array());
    let thumbnails = body["thumbnails"].as_array().unwrap();
    assert_eq!(thumbnails.len(), 0);
}

#[actix_web::test]
#[serial]
async fn test_list_video_thumbnails_invalid_token() {
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
            .service(list_video_thumbnails)
    ).await;

    let req = test::TestRequest
        ::get()
        .uri("/video_thumbnails?page=1&limit=10&image_type=camera")
        .insert_header(("Authorization", "Bearer invalid_token"))
        .to_request();

    let response = test::call_service(&app, req).await;
    assert_eq!(response.status(), http::StatusCode::UNAUTHORIZED);
}

/// Test the combined /media_thumbnails endpoint that Android uses as its primary gallery view.
/// This returns both images and videos in a single response.
#[actix_web::test]
#[serial]
async fn test_list_all_media_thumbnails() {
    let (pool, _test_db) = setup_test_database_with_instance().await;
    let client = pool.get().await.expect("Failed to get client from pool");

    let config = common::utils::create_test_config();
    // Clean up any existing test data
    client.execute("DELETE FROM images WHERE hash = $1", &[&common::TEST_THUMBNAILS_HASH]).await.ok();
    client.execute("DELETE FROM videos WHERE hash = $1", &[&common::TEST_VIDEO_HASH]).await.ok();

    // Insert an image
    client
        .execute(
            "INSERT INTO images (hash, name, exif, created_at, type, deviceid, ext, has_thumbnail) VALUES ($1, $2, $3, $4, $5, $6, $7, $8)",
            &[
                &common::TEST_THUMBNAILS_HASH,
                &common::TEST_IMAGE_NAME,
                &None::<&str>,
                &chrono::Utc::now(),
                &"camera",
                &"test_device_id",
                &"jpg",
                &true,
            ]
        ).await
        .expect("Failed to insert test image");

    // Insert a video
    client
        .execute(
            "INSERT INTO videos (hash, name, metadata, created_at, type, deviceid, ext, has_thumbnail) VALUES ($1, $2, $3, $4, $5, $6, $7, $8)",
            &[
                &common::TEST_VIDEO_HASH,
                &common::TEST_VIDEO_NAME,
                &None::<&str>,
                &chrono::Utc::now(),
                &"camera",
                &"test_device_id",
                &"mp4",
                &true,
            ]
        ).await
        .expect("Failed to insert test video");

    // Create dummy thumbnail files
    let img_subdir = &common::TEST_THUMBNAILS_HASH[..2];
    let img_sub_dir_path = Path::new(config.get_images_dir()).join(img_subdir);
    tokio::fs::create_dir_all(&img_sub_dir_path).await.unwrap();
    let img_thumb_path = format!("{}/{}/{}.thumb.jpg", config.get_images_dir(), img_subdir, common::TEST_THUMBNAILS_HASH);
    let test_image_data = fs::read("tests/test_image.jpg").unwrap();
    fs::write(&img_thumb_path, &test_image_data).unwrap();

    let vid_subdir = &common::TEST_VIDEO_HASH[..2];
    let vid_sub_dir_path = Path::new(config.get_videos_dir()).join(vid_subdir);
    tokio::fs::create_dir_all(&vid_sub_dir_path).await.unwrap();
    let vid_thumb_path = format!("{}/{}/{}.thumb.jpg", config.get_videos_dir(), vid_subdir, common::TEST_VIDEO_HASH);
    fs::write(&vid_thumb_path, &test_image_data).unwrap();

    let main_pool = common::utils::wrap_main_pool(pool.clone());
    let geotagging_pool = common::utils::create_geotagging_pool().await;

    let app = test::init_service(
        App::new()
            .app_data(web::Data::new(main_pool.clone()))
            .app_data(web::Data::new(geotagging_pool.clone()))
            .app_data(web::Data::new(config.clone()))
            .service(list_all_media_thumbnails)
    ).await;

    let token = common::utils::create_test_jwt_token().await;

    let req = test::TestRequest::get()
        .uri("/media_thumbnails?page=1&limit=50")
        .insert_header(("Authorization", format!("Bearer {}", token)))
        .to_request();

    let response = test::call_service(&app, req).await;
    assert_eq!(response.status(), http::StatusCode::OK);

    let body: serde_json::Value = test::read_body_json(response).await;
    assert_eq!(body["total"], 2, "Should have 2 items (1 image + 1 video)");
    assert_eq!(body["page"], 1);

    let thumbnails = body["thumbnails"].as_array().unwrap();
    assert_eq!(thumbnails.len(), 2);

    // Verify each thumbnail has expected fields
    for thumb in thumbnails {
        assert!(thumb["hash"].is_string());
        assert!(thumb["created_at"].is_string());
        assert!(thumb["media_type"].is_string(), "Combined endpoint should include media_type");
    }

    // Verify both media types are present
    let media_types: Vec<&str> = thumbnails.iter()
        .map(|t| t["media_type"].as_str().unwrap())
        .collect();
    assert!(media_types.contains(&"image"), "Should contain an image");
    assert!(media_types.contains(&"video"), "Should contain a video");

    // Clean up
    fs::remove_file(&img_thumb_path).ok();
    fs::remove_file(&vid_thumb_path).ok();
    client.execute("DELETE FROM images WHERE hash = $1", &[&common::TEST_THUMBNAILS_HASH]).await.ok();
    client.execute("DELETE FROM videos WHERE hash = $1", &[&common::TEST_VIDEO_HASH]).await.ok();
}

#[actix_web::test]
#[serial]
async fn test_list_all_media_thumbnails_invalid_token() {
    let (pool, _test_db) = setup_test_database_with_instance().await;
    let config = common::utils::create_test_config();

    let main_pool = common::utils::wrap_main_pool(pool.clone());
    let geotagging_pool = common::utils::create_geotagging_pool().await;

    let app = test::init_service(
        App::new()
            .app_data(web::Data::new(main_pool))
            .app_data(web::Data::new(geotagging_pool))
            .app_data(web::Data::new(config.clone()))
            .service(list_all_media_thumbnails)
    ).await;

    let req = test::TestRequest::get()
        .uri("/media_thumbnails?page=1&limit=50")
        .insert_header(("Authorization", "Bearer invalid_token"))
        .to_request();

    let response = test::call_service(&app, req).await;
    assert_eq!(response.status(), http::StatusCode::UNAUTHORIZED);
}
