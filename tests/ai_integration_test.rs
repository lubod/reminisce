use actix_web::{http, test, web, App};
use reminisce::services::person::{get_persons, get_person, get_person_images, update_person_name};
use reminisce::test_utils::setup_test_database_with_instance;
use serial_test::serial;
use uuid::Uuid;
use chrono::Utc;
use pgvector::Vector;

mod common;

#[actix_web::test]
#[serial]
async fn test_person_management_workflow() {
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
            .service(get_persons)
            .service(get_person)
            .service(get_person_images)
            .service(update_person_name)
    ).await;

    let token = common::utils::create_test_jwt_token().await;
    // create_test_jwt_token usually embeds a specific user_id. 
    // We need that ID to link data correctly. 
    // Assuming from previous tests it's likely a fixed or random one, but for integration 
    // testing without inspecting `common::utils` deeply, I'll insert a user 
    // that matches what the token claims if possible, OR I will just insert 
    // the user I want and rely on `create_test_jwt_token` using a user 
    // that exists or I create a token for MY user.
    // 
    // Actually, `common::utils::create_test_jwt_token` generates a token.
    // Let's assume the user ID in the token is "00000000-0000-0000-0000-000000000001" based on common test patterns,
    // or I should just create a user and generate a token for THAT user.
    // Since I can't easily generate a token without the private key (which is inside `config`),
    // I will look at `auth_utils::create_jwt_token`.
    //
    // Easier path: `common::utils::create_test_jwt_token` likely uses a hardcoded secret from `create_test_config`.
    // And it probably uses a hardcoded user_id.
    //
    // Let's just try to insert the user with the ID that `common` uses. 
    // Typically in these tests `user_id` is just a claim.
    
    // Let's parse the user_id from the token or just use the one I found in `init.sql` as a hint?
    // `init.sql` has '550e8400-e29b-41d4-a716-446655440000'.
    
    // Let's rely on `common::utils` likely using a consistent ID or me being able to insert *any* user
    // and just needing the token to match *a* user.
    
    // Actually, `ingest` logic uses `claims.user_id`.
    // The `person` endpoints filter by `claims.user_id`.
    // So the data I insert MANUALLY must match the `user_id` in the token.
    // I will decode the token to find the user_id? No, that requires decoding.
    
    // Let's just look at `tests/common.rs` or `src/test_utils.rs` if available? 
    // I can't see them now easily without reading files again.
    // I'll take a gamble: I'll use the `create_test_jwt_token` and assume the user_id is the one found in `auth_utils` default or similar.
    // Wait, `tests/common.rs` calls `utils::create_test_jwt_token`.
    // Let's assume the user ID is the one from `init.sql` if `create_test_jwt_token` uses it?
    // No, `create_test_jwt_token` probably generates a new ID or uses a fixed one.
    
    // BETTER PLAN: Update `common::utils` to expose the `TEST_USER_ID` or similar?
    // Or just guess `uuid::Uuid::nil()` or similar?
    
    // Let's look at `tests/common.rs` again briefly? No, I'll just try to "Select" the user from the DB *after* making a request?
    // No, `create_test_jwt_token` doesn't insert into DB.
    
    // Let's just use a known UUID and create a token for it *if* I can. 
    // I can't create a token easily.
    
    // Workaround: Use the `register_user` endpoint to create a user and get a token!
    // But `register_user` is in `auth`. I didn't add it to `App`.
    
    // Ok, I'll blindly assume the ID is `00000000-0000-0000-0000-000000000001` or I'll read `tests/common.rs` content again to be sure.
    // I'll read `tests/common.rs` first.
    
    let client = pool.get().await.expect("Failed to get client");

    // Use the same user_id as in common::utils::create_test_jwt_token
    let user_id_str = "550e8400-e29b-41d4-a716-446655440000";
    let user_uuid = Uuid::parse_str(user_id_str).unwrap();

    // 1. Ensure User Exists
    client.execute(
        "INSERT INTO users (id, username, email, password_hash, role, created_at) VALUES ($1, $2, $3, $4, $5, $6) ON CONFLICT (id) DO NOTHING",
        &[&user_uuid, &"ai_test_user", &"ai_test@example.com", &"hash", &"admin", &Utc::now()]
    ).await.unwrap();

    // 2. Insert Image
    let image_hash = "ai_test_image_hash_1";
    let image_name = "test_face.jpg";
    
    // Clean up potentially existing image from failed runs
    client.execute("DELETE FROM images WHERE hash = $1", &[&image_hash]).await.ok();
    
    client.execute(
        "INSERT INTO images (hash, name, created_at, type, deviceid, ext, has_thumbnail, verification_status, user_id)
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9)",
        &[&image_hash, &image_name, &Utc::now(), &"image", &"test_device_id", &"jpg", &false, &1, &user_uuid]
    ).await.unwrap();

    // 3. Insert Person
    // Manually inserting person to simulate what the AI clustering would produce
    // Using simple INSERT since 'id' is BIGSERIAL, we use DEFAULT or RETURNING
    let row = client.query_one(
        "INSERT INTO persons (name, user_id, created_at, updated_at) VALUES ($1, $2, $3, $4) RETURNING id",
        &[&"Unknown Person", &user_uuid, &Utc::now(), &Utc::now()]
    ).await.unwrap();
    let person_id: i64 = row.get(0);

    // 4. Insert Face
    // Embedding is vector(512) for InsightFace. Use pgvector::Vector.
    let dummy_embedding = Vector::from(vec![0.1f32; 512]);

    client.execute(
        "INSERT INTO faces (image_hash, image_deviceid, user_id, bbox_x, bbox_y, bbox_width, bbox_height, embedding, confidence, person_id, detected_at)
         VALUES ($1, $2, $3, 0, 0, 100, 100, $4, 0.99, $5, $6)",
         &[&image_hash, &"test_device_id", &user_uuid, &dummy_embedding, &person_id, &Utc::now()]
    ).await.unwrap();

    // Update person face count (usually done by worker, do it manually here)
    client.execute(
        "UPDATE persons SET face_count = 1 WHERE id = $1",
        &[&person_id]
    ).await.unwrap();

    // --- EXECUTE TESTS ---

    // TEST 1: List Persons
    let req = test::TestRequest::get()
        .uri("/persons")
        .insert_header(("Authorization", format!("Bearer {}", token)))
        .to_request();
    let resp = test::call_service(&app, req).await;
    assert_eq!(resp.status(), http::StatusCode::OK);
    let result: serde_json::Value = test::read_body_json(resp).await;
    
    // Response is { "persons": [...], "total": ... }
    let persons = result["persons"].as_array().expect("Response missing 'persons' array");
    
    // We might have other persons from other tests if DB wasn't clean, but we should find ours
    let my_person = persons.iter().find(|p| p["id"].as_i64() == Some(person_id)).expect("Created person not found in list");
    assert_eq!(my_person["name"], "Unknown Person");
    assert_eq!(my_person["face_count"], 1);

    // TEST 2: Get Person Details
    let req = test::TestRequest::get()
        .uri(&format!("/persons/{}", person_id))
        .insert_header(("Authorization", format!("Bearer {}", token)))
        .to_request();
    let resp = test::call_service(&app, req).await;
    assert_eq!(resp.status(), http::StatusCode::OK);
    let response_json: serde_json::Value = test::read_body_json(resp).await;
    
    // Response is { "person": {...} }
    let person = &response_json["person"];
    assert_eq!(person["id"].as_i64(), Some(person_id));

    // TEST 3: Get Person Images
    let req = test::TestRequest::get()
        .uri(&format!("/persons/{}/images", person_id))
        .insert_header(("Authorization", format!("Bearer {}", token)))
        .to_request();
    let resp = test::call_service(&app, req).await;
    assert_eq!(resp.status(), http::StatusCode::OK);
    let images_response: serde_json::Value = test::read_body_json(resp).await;
    
    // Response is { "images": [...], "total": ... }
    let image_list = images_response["images"].as_array().expect("Response missing 'images' array");
    assert!(!image_list.is_empty());
    assert_eq!(image_list[0]["hash"], image_hash);

    // TEST 4: Rename Person
    let req = test::TestRequest::put() // It's PUT, not PATCH according to service definition
        .uri(&format!("/persons/{}/name", person_id))
        .insert_header(("Authorization", format!("Bearer {}", token)))
        .set_json(serde_json::json!({ "name": "John Doe" }))
        .to_request();
    let resp = test::call_service(&app, req).await;
    assert_eq!(resp.status(), http::StatusCode::OK);

    // Verify Rename
     let req = test::TestRequest::get()
        .uri(&format!("/persons/{}", person_id))
        .insert_header(("Authorization", format!("Bearer {}", token)))
        .to_request();
    let resp = test::call_service(&app, req).await;
    let response_json: serde_json::Value = test::read_body_json(resp).await;
    assert_eq!(response_json["person"]["name"], "John Doe");

    // Cleanup
    client.execute("DELETE FROM persons WHERE id = $1", &[&person_id]).await.ok();
    client.execute("DELETE FROM images WHERE hash = $1", &[&image_hash]).await.ok();
}
