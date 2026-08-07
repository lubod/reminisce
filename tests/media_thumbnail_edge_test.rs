use actix_web::{ http, test, web, App };
use image::ImageEncoder;
use reminisce::*;
use reminisce::test_utils::setup_test_database_with_instance;
use serial_test::serial;

mod common;

/// Config pointing at an AI gRPC endpoint that refuses connections instantly.
fn config_with_dead_ai() -> reminisce::config::Config {
    let mut c = common::utils::create_test_config();
    c.ai_grpc_url = "http://127.0.0.1:1".to_string();
    c
}

const TEST_USER: &str = "550e8400-e29b-41d4-a716-446655440000";

macro_rules! authed {
    ($method:ident, $app:expr, $uri:expr, $token:expr) => {{
        let req = test::TestRequest::$method()
            .uri($uri)
            .insert_header(("Authorization", format!("Bearer {}", $token)))
            .to_request();
        test::call_service($app, req)
    }};
}

macro_rules! make_app {
    ($pool:expr, $config:expr) => {{
        let main_pool = common::utils::wrap_main_pool($pool);
        test::init_service(
            App::new()
                .app_data(web::Data::new(main_pool))
                .app_data(web::Data::new($config.clone()))
                .service(services::thumbnail::get_thumbnail)
                .service(services::thumbnail::get_face_thumbnail)
                .service(services::media::get_image)
                .service(services::media::get_video)
                .service(services::media::get_image_metadata)
                .service(services::media::enhance_image)
        ).await
    }};
}

async fn seed_image(pool: &deadpool_postgres::Pool, hash: &str) {
    let client = pool.get().await.unwrap();
    let uid = uuid::Uuid::parse_str(TEST_USER).unwrap();
    client
        .execute(
            "INSERT INTO images (user_id, deviceid, hash, name, ext, type, has_thumbnail) \
             VALUES ($1, 'dev', $2, 'pic.jpg', 'jpg', 'camera', true)",
            &[&uid, &hash],
        )
        .await
        .unwrap();
}

#[actix_web::test]
#[serial]
async fn test_thumbnail_and_media_404_paths() {
    let (pool, _db) = setup_test_database_with_instance().await;
    let config = common::utils::create_test_config();
    // DB row exists but no file on disk for the thumbnail path.
    seed_image(&pool, "no_thumb_file").await;

    let app = make_app!(pool, &config);
    let token = common::utils::create_test_jwt_token().await;

    // get_thumbnail: row exists but no thumbnail file -> 404.
    let resp = authed!(get, &app, "/thumbnail/no_thumb_file", &token).await;
    assert_eq!(resp.status(), http::StatusCode::NOT_FOUND);

    // get_thumbnail for an unknown hash -> 404.
    let resp = authed!(get, &app, "/thumbnail/does_not_exist", &token).await;
    assert_eq!(resp.status(), http::StatusCode::NOT_FOUND);

    // get_image / get_video for unknown hashes -> 404.
    let resp = authed!(get, &app, "/image/does_not_exist", &token).await;
    assert_eq!(resp.status(), http::StatusCode::NOT_FOUND);
    let resp = authed!(get, &app, "/video/does_not_exist", &token).await;
    assert_eq!(resp.status(), http::StatusCode::NOT_FOUND);

    // get_image_metadata for an unknown hash -> 404.
    let resp = authed!(get, &app, "/image/does_not_exist/metadata", &token).await;
    assert_eq!(resp.status(), http::StatusCode::NOT_FOUND);

    // get_face_thumbnail for an unknown face -> 404 (no cached face file).
    let resp = authed!(get, &app, "/face/999999/thumbnail", &token).await;
    assert_eq!(resp.status(), http::StatusCode::NOT_FOUND);

    // All endpoints require auth.
    let req = test::TestRequest::get().uri("/thumbnail/x").to_request();
    let resp = test::call_service(&app, req).await;
    assert_eq!(resp.status(), http::StatusCode::UNAUTHORIZED);
}

#[actix_web::test]
#[serial]
async fn test_enhance_image_error_paths() {
    let (pool, _db) = setup_test_database_with_instance().await;
    // Missing file: row hash has no on-disk media -> 404.
    let config = common::utils::create_test_config();
    seed_image(&pool, "enhance_missing_file").await;
    let app = make_app!(pool.clone(), &config);
    let token = common::utils::create_test_jwt_token().await;

    let resp = authed!(post, &app, "/image/enhance_missing_file/enhance", &token).await;
    assert_eq!(resp.status(), http::StatusCode::NOT_FOUND, "missing file -> 404");

    // Real file present + dead AI -> 503 (AI service unavailable).
    // Write a tiny valid JPEG whose BLAKE3 == hash, then enhance with dead AI.
    let img = image::RgbImage::from_pixel(4, 4, image::Rgb([10, 20, 30]));
    let mut buf = std::io::Cursor::new(Vec::new());
    let enc = image::codecs::jpeg::JpegEncoder::new(&mut buf);
    enc.write_image(img.as_raw(), img.width(), img.height(), image::ColorType::Rgb8).unwrap();
    let bytes = buf.into_inner();
    let hash = blake3::hash(&bytes).to_hex().to_string();
    {
        let cl = pool.get().await.unwrap();
        let uid = uuid::Uuid::parse_str(TEST_USER).unwrap();
        cl.execute(
            "INSERT INTO images (user_id, deviceid, hash, name, ext, type, has_thumbnail) VALUES ($1,'dev',$2,'pic.jpg','jpg','camera',true)",
            &[&uid, &hash],
        ).await.unwrap();
    }
    let sub = std::path::Path::new(config.get_images_dir()).join(&hash[..2]);
    std::fs::create_dir_all(&sub).unwrap();
    std::fs::write(sub.join(format!("{}.jpg", hash)), &bytes).unwrap();

    // Absolute media dir so safe_resolve_content_path's canonicalize is unambiguous.
    let abs_media = std::env::temp_dir().join(format!("reminisce_enhance_{}", std::process::id()));
    std::fs::create_dir_all(&abs_media).unwrap();
    let mut dead = config_with_dead_ai();
    dead.images_dir = Some(abs_media.to_string_lossy().to_string());
    let sub = abs_media.join(&hash[..2]);
    std::fs::create_dir_all(&sub).unwrap();
    std::fs::write(sub.join(format!("{}.jpg", hash)), &bytes).unwrap();

    let app2 = make_app!(pool.clone(), &dead);
    let resp = authed!(post, &app2, &format!("/image/{}/enhance", hash), &token).await;
    assert_eq!(resp.status(), http::StatusCode::SERVICE_UNAVAILABLE, "dead AI -> 503: {}", resp.status());
    let _ = std::fs::remove_dir_all(&abs_media);
}
