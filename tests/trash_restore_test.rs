use actix_web::{ http, test, web, App };
use reminisce::*;
use reminisce::test_utils::setup_test_database_with_instance;
use serial_test::serial;

mod common;

const TEST_USER: &str = "550e8400-e29b-41d4-a716-446655440000";

fn user_uuid() -> uuid::Uuid {
    uuid::Uuid::parse_str(TEST_USER).unwrap()
}

macro_rules! authed {
    ($method:ident, $app:expr, $uri:expr, $token:expr) => {{
        let req = test::TestRequest::$method()
            .uri($uri)
            .insert_header(("Authorization", format!("Bearer {}", $token)))
            .to_request();
        test::call_service($app, req)
    }};
}

async fn seed_image(pool: &deadpool_postgres::Pool, hash: &str) {
    let client = pool.get().await.unwrap();
    client
        .execute(
            "INSERT INTO images (user_id, deviceid, hash, name, ext, has_thumbnail) VALUES ($1, $2, $3, $4, $5, true)",
            &[&user_uuid(), &"dev", &hash, &format!("{}.jpg", hash), &"jpg"],
        )
        .await
        .expect("seed image failed");
}

async fn seed_video(pool: &deadpool_postgres::Pool, hash: &str) {
    let client = pool.get().await.unwrap();
    client
        .execute(
            "INSERT INTO videos (user_id, deviceid, hash, name, ext, has_thumbnail) VALUES ($1, $2, $3, $4, $5, true)",
            &[&user_uuid(), &"dev", &hash, &format!("{}.mp4", hash), &"mp4"],
        )
        .await
        .expect("seed video failed");
}

#[actix_web::test]
#[serial]
async fn test_trash_delete_list_restore_cycle() {
    let (pool, _test_db) = setup_test_database_with_instance().await;
    seed_image(&pool, "img_trash").await;
    seed_video(&pool, "vid_trash").await;

    let config = common::utils::create_test_config();
    let main_pool = common::utils::wrap_main_pool(pool.clone());
    let app = test::init_service(
        App::new()
            .app_data(web::Data::new(main_pool))
            .app_data(web::Data::new(config.clone()))
            .service(services::media::get_trash)
            .service(services::media::delete_image)
            .service(services::media::delete_video)
            .service(services::media::restore_image)
            .service(services::media::restore_video)
    ).await;
    let token = common::utils::create_test_jwt_token().await;

    // Nothing deleted yet.
    let resp = authed!(get, &app, "/trash", &token).await;
    assert_eq!(resp.status(), http::StatusCode::OK);
    let items: serde_json::Value = test::read_body_json(resp).await;
    assert_eq!(items.as_array().unwrap().len(), 0, "trash starts empty: {:?}", items);

    // Soft-delete both.
    let r = authed!(post, &app, "/image/img_trash/delete", &token).await;
    assert_eq!(r.status(), http::StatusCode::OK);
    let r = authed!(post, &app, "/video/vid_trash/delete", &token).await;
    assert_eq!(r.status(), http::StatusCode::OK);

    // Both now appear in trash.
    let resp = authed!(get, &app, "/trash", &token).await;
    let items: serde_json::Value = test::read_body_json(resp).await;
    let hashes: Vec<&str> = items.as_array().unwrap().iter().map(|i| i["hash"].as_str().unwrap()).collect();
    assert!(hashes.contains(&"img_trash") && hashes.contains(&"vid_trash"), "trash has both: {:?}", hashes);

    // Restore both.
    let r = authed!(post, &app, "/image/img_trash/restore", &token).await;
    assert_eq!(r.status(), http::StatusCode::OK);
    let r = authed!(post, &app, "/video/vid_trash/restore", &token).await;
    assert_eq!(r.status(), http::StatusCode::OK);

    let resp = authed!(get, &app, "/trash", &token).await;
    let items: serde_json::Value = test::read_body_json(resp).await;
    assert_eq!(items.as_array().unwrap().len(), 0, "trash empty after restore: {:?}", items);

    // Restoring something not deleted -> 404.
    let r = authed!(post, &app, "/image/img_trash/restore", &token).await;
    assert_eq!(r.status(), http::StatusCode::NOT_FOUND);
    let r = authed!(post, &app, "/image/never-existed/restore", &token).await;
    assert_eq!(r.status(), http::StatusCode::NOT_FOUND);
}

#[actix_web::test]
#[serial]
async fn test_trash_requires_auth() {
    let (pool, _test_db) = setup_test_database_with_instance().await;
    let config = common::utils::create_test_config();
    let main_pool = common::utils::wrap_main_pool(pool.clone());
    let app = test::init_service(
        App::new()
            .app_data(web::Data::new(main_pool))
            .app_data(web::Data::new(config.clone()))
            .service(services::media::get_trash)
    ).await;
    let req = test::TestRequest::get().uri("/trash").to_request();
    let resp = test::call_service(&app, req).await;
    assert_eq!(resp.status(), http::StatusCode::UNAUTHORIZED);
}

#[actix_web::test]
#[serial]
async fn test_random_image_filters() {
    let (pool, _test_db) = setup_test_database_with_instance().await;
    seed_image(&pool, "rnd_a").await;
    seed_image(&pool, "rnd_b").await;
    // Only rnd_b is starred.
    let client = pool.get().await.unwrap();
    client
        .execute("INSERT INTO starred_images (user_id, hash) VALUES ($1, 'rnd_b')", &[&user_uuid()])
        .await
        .unwrap();

    let config = common::utils::create_test_config();
    let main_pool = common::utils::wrap_main_pool(pool.clone());
    let app = test::init_service(
        App::new()
            .app_data(web::Data::new(main_pool))
            .app_data(web::Data::new(config.clone()))
            .service(services::media::get_random_image)
    ).await;
    let token = common::utils::create_test_jwt_token().await;

    // Random (unfiltered) returns one of the two.
    let resp = authed!(get, &app, "/image/random", &token).await;
    assert_eq!(resp.status(), http::StatusCode::OK);
    let body: serde_json::Value = test::read_body_json(resp).await;
    assert!(["rnd_a", "rnd_b"].contains(&body["hash"].as_str().unwrap()), "random image: {:?}", body);

    // starred_only returns the starred one (only candidate).
    let resp = authed!(get, &app, "/image/random?starred_only=true", &token).await;
    assert_eq!(resp.status(), http::StatusCode::OK);
    let body: serde_json::Value = test::read_body_json(resp).await;
    assert_eq!(body["hash"], "rnd_b");

    // Filtering that matches nothing -> 404.
    let resp = authed!(get, &app, "/image/random?starred_only=true&label_ids=999999", &token).await;
    assert_eq!(resp.status(), http::StatusCode::NOT_FOUND);
}
