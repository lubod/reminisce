use reminisce::services::thumbnail::ThumbnailsResponse;
use reminisce::services::stats::StatsResponse;
// use reminisce::run_server;
use std::path::Path;
use tokio::fs;
use actix_web::{test, App, web};

mod common;
use common::utils;
use common::multipart_builder;

#[tokio::test]
async fn test_soft_delete_workflow() {
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
            .service(reminisce::services::upload::upload_image)
            .service(reminisce::services::thumbnail::list_all_media_thumbnails)
            .service(reminisce::services::stats::get_stats)
            .service(reminisce::services::media::delete_image)
    ).await;

    let token = utils::create_test_jwt_token().await;

    // 1. Upload an image
    let image_path = Path::new("tests/test_image.jpg");
    let image_data = fs::read(image_path).await.unwrap();
    let (payload, content_type) = multipart_builder::create_multipart_payload_without_thumbnail(
        common::TEST_IMAGE_HASH,
        common::TEST_IMAGE_NAME,
        &image_data
    );
    
    let req = test::TestRequest::post()
        .uri("/upload/image")
        .insert_header(("Authorization", format!("Bearer {}", token)))
        .insert_header(("Content-Type", content_type))
        .set_payload(payload)
        .to_request();

    let resp = test::call_service(&app, req).await;
    assert_eq!(resp.status(), actix_web::http::StatusCode::CREATED);

    // Manually set has_thumbnail = true to simulate verification worker for thumbnail listing
    let client = pool.get().await.unwrap();
    client.execute("UPDATE images SET has_thumbnail = true WHERE hash = $1", &[&common::TEST_IMAGE_HASH]).await.unwrap();

    // 2. Verify it exists in thumbnails
    let req = test::TestRequest::get()
        .uri("/media_thumbnails")
        .insert_header(("Authorization", format!("Bearer {}", token)))
        .to_request();
    
    let resp = test::call_service(&app, req).await;
    assert_eq!(resp.status(), actix_web::http::StatusCode::OK);
    let body: ThumbnailsResponse = test::read_body_json(resp).await;
    assert_eq!(body.total, 1);
    assert_eq!(body.thumbnails[0].hash, common::TEST_IMAGE_HASH);

    // 3. Verify stats
    let req = test::TestRequest::get()
        .uri("/stats")
        .insert_header(("Authorization", format!("Bearer {}", token)))
        .to_request();
    let resp = test::call_service(&app, req).await;
    let stats: StatsResponse = test::read_body_json(resp).await;
    assert_eq!(stats.total_images, 1);

    // 4. Soft delete the image
    let req = test::TestRequest::post()
        .uri(&format!("/image/{}/delete", common::TEST_IMAGE_HASH))
        .insert_header(("Authorization", format!("Bearer {}", token)))
        .to_request();
    let resp = test::call_service(&app, req).await;
    assert_eq!(resp.status(), actix_web::http::StatusCode::OK);

    // 5. Verify it's gone from thumbnails
    let req = test::TestRequest::get()
        .uri("/media_thumbnails")
        .insert_header(("Authorization", format!("Bearer {}", token)))
        .to_request();
    let resp = test::call_service(&app, req).await;
    let body: ThumbnailsResponse = test::read_body_json(resp).await;
    assert_eq!(body.total, 0);

    // 6. Verify stats decreased
    let req = test::TestRequest::get()
        .uri("/stats")
        .insert_header(("Authorization", format!("Bearer {}", token)))
        .to_request();
    let resp = test::call_service(&app, req).await;
    let stats: StatsResponse = test::read_body_json(resp).await;
    assert_eq!(stats.total_images, 0);

    // 7. Verify re-upload restores it
    let (payload, content_type) = multipart_builder::create_multipart_payload_without_thumbnail(
        common::TEST_IMAGE_HASH,
        common::TEST_IMAGE_NAME,
        &image_data
    );
    let req = test::TestRequest::post()
        .uri("/upload/image")
        .insert_header(("Authorization", format!("Bearer {}", token)))
        .insert_header(("Content-Type", content_type))
        .set_payload(payload)
        .to_request();

    let resp = test::call_service(&app, req).await;
    assert_eq!(resp.status(), actix_web::http::StatusCode::CREATED);

    // Manually set has_thumbnail = true to simulate verification worker for thumbnail listing
    let client = pool.get().await.unwrap();
    client.execute("UPDATE images SET has_thumbnail = true WHERE hash = $1", &[&common::TEST_IMAGE_HASH]).await.unwrap();

    // 8. Verify it exists again
    let req = test::TestRequest::get()
        .uri("/media_thumbnails")
        .insert_header(("Authorization", format!("Bearer {}", token)))
        .to_request();
    let resp = test::call_service(&app, req).await;
    let body: ThumbnailsResponse = test::read_body_json(resp).await;
    assert_eq!(body.total, 1);
    
    // Cleanup
    let _ = fs::remove_dir_all("uploaded_images_test").await;
}