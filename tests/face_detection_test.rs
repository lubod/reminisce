use deadpool_postgres::Pool;
use pgvector::Vector;
use reminisce::services::face_detection::{ cluster_faces_for_user, store_faces };
use reminisce::test_utils::setup_test_database_with_instance;
use serial_test::serial;

mod common;

const TEST_USER: &str = "550e8400-e29b-41d4-a716-446655440000";

fn user_uuid() -> uuid::Uuid {
    uuid::Uuid::parse_str(TEST_USER).unwrap()
}

/// Monotonic ramp embedding (all similar constant-scaled ramps point the same
/// direction -> cosine ~1.0); the reversed ramp points the opposite way.
fn ramp(sign: f32) -> Vector {
    Vector::from(
        (0..512)
            .map(|i| sign * (i as f32 + 1.0) / 512.0)
            .collect::<Vec<f32>>(),
    )
}

fn rev_ramp() -> Vector {
    Vector::from(
        (0..512)
            .map(|i| 1.0 - (i as f32 + 1.0) / 512.0)
            .collect::<Vec<f32>>(),
    )
}

fn emb(fill: f32) -> Vector {
    ramp(fill)
}

async fn seed_image(pool: &Pool, hash: &str) {
    let client = pool.get().await.unwrap();
    client
        .execute(
            "INSERT INTO images (user_id, deviceid, hash, name, ext, type, has_thumbnail) \
             VALUES ($1, 'dev', $2, 'face.jpg', 'jpg', 'camera', true)",
            &[&user_uuid(), &hash],
        )
        .await
        .unwrap();
}

#[actix_web::test]
#[serial]
async fn test_store_faces_inserts_and_skips_malformed() {
    let (pool, _db) = setup_test_database_with_instance().await;
    seed_image(&pool, "img_faces").await;

    let client = pool.get().await.unwrap();
    let faces = vec![
        (vec![0, 0, 100, 100], emb(0.1), 0.99),
        (vec![10, 20, 50, 60], emb(0.2), 0.95),
        // malformed bbox (len != 4) -> skipped
        (vec![1, 2], emb(0.3), 0.9),
    ];
    let stored = store_faces("img_faces", &user_uuid(), faces, &client).await.expect("store ok");
    assert_eq!(stored, 2, "two valid faces stored");

    let count: i64 = client
        .query_one("SELECT COUNT(*) FROM faces WHERE image_hash = 'img_faces'", &[])
        .await
        .unwrap()
        .get(0);
    assert_eq!(count, 2);
}

#[actix_web::test]
#[serial]
async fn test_cluster_faces_assigns_and_creates_persons() {
    let (pool, _db) = setup_test_database_with_instance().await;
    seed_image(&pool, "img_f1").await;
    seed_image(&pool, "img_f2").await;

    let client = pool.get().await.unwrap();

    // Face A (0.1) and Face A' (0.11) are similar -> same person.
    // Face B (0.9) is far -> its own person.
    store_faces("img_f1", &user_uuid(), vec![(vec![0,0,10,10], ramp(1.0), 0.9)], &client).await.unwrap();
    store_faces("img_f1", &user_uuid(), vec![(vec![0,0,10,10], rev_ramp(), 0.9)], &client).await.unwrap();
    store_faces("img_f2", &user_uuid(), vec![(vec![0,0,10,10], ramp(2.0), 0.9)], &client).await.unwrap();

    // All three are unclustered; cluster them: A and A' should join, B separate.
    let clustered = cluster_faces_for_user(&user_uuid(), &client).await.expect("cluster ok");
    assert_eq!(clustered, 3, "all faces assigned to a person");

    let persons: i64 = client
        .query_one("SELECT COUNT(*) FROM persons WHERE user_id = $1", &[&user_uuid()])
        .await
        .unwrap()
        .get(0);
    assert_eq!(persons, 2, "two distinct persons (A/A' merged, B alone)");

    // Every face has a person assigned.
    let unassigned: i64 = client
        .query_one("SELECT COUNT(*) FROM faces WHERE user_id = $1 AND person_id IS NULL", &[&user_uuid()])
        .await
        .unwrap()
        .get(0);
    assert_eq!(unassigned, 0);

    // Running again (nothing unclustered) -> 0.
    let again = cluster_faces_for_user(&user_uuid(), &client).await.expect("cluster ok");
    assert_eq!(again, 0);
}

#[actix_web::test]
#[serial]
async fn test_cluster_faces_joins_existing_person() {
    let (pool, _db) = setup_test_database_with_instance().await;
    seed_image(&pool, "img_a").await;

    let client = pool.get().await.unwrap();
    // Seed an already-clustered face plus a new similar one.
    store_faces("img_a", &user_uuid(), vec![(vec![0,0,10,10], ramp(1.0), 0.9)], &client).await.unwrap();
    // Manually cluster the first face into a person to simulate prior work.
    client
        .execute(
            "INSERT INTO persons (user_id, name, face_count) VALUES ($1, 'P', 1) RETURNING id",
            &[&user_uuid()],
        )
        .await
        .unwrap();
    let person_id: i64 = client
        .query_one("SELECT id FROM persons WHERE user_id = $1 LIMIT 1", &[&user_uuid()])
        .await
        .unwrap()
        .get(0);
    let face_id: i64 = client
        .query_one("SELECT id FROM faces WHERE image_hash = 'img_a' LIMIT 1", &[])
        .await
        .unwrap()
        .get(0);
    client
        .execute("UPDATE faces SET person_id = $1 WHERE id = $2", &[&person_id, &face_id])
        .await
        .unwrap();

    // New similar face (0.11) gets added to the SAME person.
    store_faces("img_a", &user_uuid(), vec![(vec![0,0,10,10], ramp(2.0), 0.9)], &client).await.unwrap();
    let clustered = cluster_faces_for_user(&user_uuid(), &client).await.expect("cluster ok");
    assert_eq!(clustered, 1);

    let assigned_person: i64 = client
        .query_one("SELECT person_id FROM faces WHERE embedding = $1 LIMIT 1", &[&ramp(2.0)])
        .await
        .unwrap()
        .get(0);
    assert_eq!(assigned_person, person_id, "similar face joined the existing person");
}
