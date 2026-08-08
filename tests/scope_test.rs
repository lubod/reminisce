use actix_web::{http, test, web, App};
use reminisce::test_utils::setup_test_database_with_instance;
use serial_test::serial;

mod common;

/// A `media_read`-scoped token (the 24h image_token used for <img> URLs) must
/// NEVER unlock mutation endpoints. Regression test for the privilege-escalation
/// where the image token worked as a full session token via authenticate_request.
#[actix_web::test]
#[serial]
async fn media_read_token_cannot_delete_images() {
    common::init_log();
    let (pool, _test_db) = setup_test_database_with_instance().await;
    let main_pool = common::utils::wrap_main_pool(pool.clone());
    let config = common::utils::create_test_config();

    let app = test::init_service(
        App::new()
            .app_data(web::Data::new(main_pool))
            .app_data(web::Data::new(config))
            .service(reminisce::services::media::delete_image)
    ).await;

    // A media_read-scoped token must be rejected with 403 before any handler logic.
    let media_read_token = common::utils::create_test_jwt_token_with_scope(Some("media_read")).await;
    let req = test::TestRequest::post()
        .uri("/image/whatever-hash/delete")
        .insert_header(("Authorization", format!("Bearer {}", media_read_token)))
        .to_request();
    let resp = test::call_service(&app, req).await;
    assert_eq!(resp.status(), http::StatusCode::FORBIDDEN, "media_read token must be forbidden on delete");

    // A full session token is NOT scoped: it may reach handler logic (e.g. 404 for
    // a nonexistent hash, or 200 for a real one) — in any case not a 403 scope denial.
    let full_token = common::utils::create_test_jwt_token().await;
    let req2 = test::TestRequest::post()
        .uri("/image/whatever-hash/delete")
        .insert_header(("Authorization", format!("Bearer {}", full_token)))
        .to_request();
    let resp2 = test::call_service(&app, req2).await;
    assert_ne!(resp2.status(), http::StatusCode::FORBIDDEN, "full session token should not hit the scope denial");

    // The raw-media byte-serving endpoints remain reachable with a media_read token:
    // get_image uses only config+pool and returns non-auth errors for missing media.
    let app2 = test::init_service(
        App::new()
            .app_data(web::Data::new(pool))
            .app_data(web::Data::new(common::utils::create_test_config()))
            .service(reminisce::services::media::get_image)
    ).await;
    let req3 = test::TestRequest::get()
        .uri("/image/af29ca6fd22f34f3c51c3dc5326ff277b80ad6344a3a9af35bb5548ccf8cdb16")
        .insert_header(("Authorization", format!("Bearer {}", media_read_token)))
        .to_request();
    let resp3 = test::call_service(&app2, req3).await;
    assert_ne!(resp3.status(), http::StatusCode::FORBIDDEN, "media_read token must still serve raw image bytes");
}
