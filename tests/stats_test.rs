use actix_web::{http, test, web, App};
use reminisce::*;
use reminisce::test_utils::setup_test_database_with_instance;
use serial_test::serial;
use reminisce::services::stats::StatsResponse;

mod common;

#[actix_web::test]
#[serial]
async fn test_get_stats() {
    common::init_log();
    let (pool, _test_db) = setup_test_database_with_instance().await;
    let client = pool.get().await.unwrap();

    // Clean up before test
    client.execute("TRUNCATE TABLE users, images, videos, starred_images, starred_videos, persons, faces, ai_settings RESTART IDENTITY CASCADE", &[]).await.unwrap();

    // Insert test data
    // Users
    client.execute("INSERT INTO users (username, email, password_hash, role) VALUES ('admin', 'admin@test.com', 'hash', 'admin')", &[]).await.unwrap();
    client.execute("INSERT INTO users (username, email, password_hash, role) VALUES ('user', 'user@test.com', 'hash', 'user')", &[]).await.unwrap();

    // Images
    let user_id: uuid::Uuid = client.query_one("SELECT id FROM users WHERE username = 'user'", &[]).await.unwrap().get(0);
    client.execute("INSERT INTO images (hash, name, ext, deviceid, description, verification_status) VALUES ($1, $2, $3, $4, $5, $6)",
                   &[&"hash1", &"img1.jpg", &"jpg", &"dev1", &"a description", &1i32]).await.unwrap();
    client.execute("INSERT INTO images (hash, name, ext, deviceid, embedding_generated_at) VALUES ($1, $2, $3, $4, NOW())",
                   &[&"hash2", &"img2.jpg", &"jpg", &"dev1"]).await.unwrap();
    client.execute("INSERT INTO images (hash, name, ext, deviceid) VALUES ($1, $2, $3, $4)",
                   &[&"hash3", &"img3.jpg", &"jpg", &"dev2"]).await.unwrap();
    client.execute("INSERT INTO starred_images (user_id, hash) VALUES ($1, $2)", &[&user_id, &"hash1"]).await.unwrap();

    // Videos
    client.execute("INSERT INTO videos (hash, name, ext, deviceid, verification_status) VALUES ($1, $2, $3, $4, $5)",
                   &[&"vhash1", &"vid1.mp4", &"mp4", &"dev1", &1i32]).await.unwrap();
    client.execute("INSERT INTO videos (hash, name, ext, deviceid) VALUES ($1, $2, $3, $4)",
                   &[&"vhash2", &"vid2.mp4", &"mp4", &"dev2"]).await.unwrap();
    client.execute("INSERT INTO starred_videos (user_id, hash) VALUES ($1, $2)", &[&user_id, &"vhash1"]).await.unwrap();


    let config = common::utils::create_test_config();
    let main_pool = common::utils::wrap_main_pool(pool.clone());
    let geotagging_pool = common::utils::create_mock_geotagging_pool(pool.clone());

    let app = test::init_service(
        App::new()
            .app_data(web::Data::new(main_pool.clone()))
            .app_data(web::Data::new(geotagging_pool.clone()))
            .app_data(web::Data::new(config.clone()))
            .service(services::stats::get_stats)
    ).await;

    // Test as admin
    let admin_token = common::utils::create_test_jwt_token().await;
    let req = test::TestRequest::get()
        .uri("/stats")
        .insert_header(("Authorization", format!("Bearer {}", admin_token)))
        .to_request();
    let resp = test::call_service(&app, req).await;
    assert_eq!(resp.status(), http::StatusCode::OK);

    let stats: StatsResponse = test::read_body_json(resp).await;
    assert_eq!(stats.total_images, 3);
    assert_eq!(stats.total_videos, 2);
    assert_eq!(stats.total_users, 2);
    assert_eq!(stats.images_with_description, 1);
    assert_eq!(stats.starred_images, 1);
    assert_eq!(stats.starred_videos, 1);
    assert_eq!(stats.images_with_embedding, 1);
    assert_eq!(stats.verified_images, 1);
    assert_eq!(stats.verified_videos, 1);
    // Since default is FALSE and we didn't specify TRUE for any insert above, expect 0.
    // Wait, let's update one to true to verify the count.
    client.execute("UPDATE images SET has_thumbnail = true WHERE hash = 'hash1'", &[]).await.unwrap();
    client.execute("UPDATE videos SET has_thumbnail = true WHERE hash = 'vhash1'", &[]).await.unwrap();

    let req2 = test::TestRequest::get()
        .uri("/stats")
        .insert_header(("Authorization", format!("Bearer {}", admin_token)))
        .to_request();
    let resp2 = test::call_service(&app, req2).await;
    assert_eq!(resp2.status(), http::StatusCode::OK);
    let stats2: StatsResponse = test::read_body_json(resp2).await;
    assert_eq!(stats2.thumbnail_count, 2); // 1 image + 1 video

    // Test as non-admin
    let user_token = {
        use jsonwebtoken::{encode, Algorithm, EncodingKey, Header};
        let claims = Claims {
            user_id: user_id.to_string(),
            username: "test-user".to_string(),
            email: "test@example.com".to_string(),
            role: "user".to_string(),
            exp: (chrono::Utc::now() + chrono::Duration::days(1)).timestamp() as usize,
        };
        encode(
            &Header::new(Algorithm::HS512),
            &claims,
            &EncodingKey::from_secret("test_secret".as_ref()),
        ).unwrap()
    };
    let req = test::TestRequest::get()
        .uri("/stats")
        .insert_header(("Authorization", format!("Bearer {}", user_token)))
        .to_request();
    let resp = test::call_service(&app, req).await;
    assert_eq!(resp.status(), http::StatusCode::FORBIDDEN);
}