use deadpool_postgres::Pool;
use pgvector::Vector;
use reminisce::db::MainDbPool;
use reminisce::duplicate_worker::{ process_batch, DuplicateWorkerStatus, SharedDuplicateStatus };
use reminisce::test_utils::setup_test_database_with_instance;
use serial_test::serial;
use std::sync::Arc;
use tokio::sync::Mutex;

mod common;

const TEST_USER: &str = "550e8400-e29b-41d4-a716-446655440000";

fn user_uuid() -> uuid::Uuid {
    uuid::Uuid::parse_str(TEST_USER).unwrap()
}

async fn seed_image(pool: &Pool, hash: &str, emb: &Vector) {
    let client = pool.get().await.unwrap();
    client
        .execute(
            "INSERT INTO images (user_id, deviceid, hash, name, ext, type, has_thumbnail, verification_status, embedding) \
             VALUES ($1, 'dev', $2, $3, 'jpg', 'camera', true, 1, $4)",
            &[&user_uuid(), &hash, &format!("{}.jpg", hash), emb],
        )
        .await
        .expect("seed image");
}

fn main_pool(pool: Pool) -> MainDbPool {
    MainDbPool(pool)
}

fn fresh_status() -> SharedDuplicateStatus {
    Arc::new(Mutex::new(DuplicateWorkerStatus::new()))
}

#[actix_web::test]
#[serial]
async fn process_batch_creates_pairs_for_similar_embeddings() {
    let (pool, _db) = setup_test_database_with_instance().await;

    // img_a and img_b share an embedding -> similarity 1.0 -> pair inserted.
    // img_c is orthogonal -> no pair with the others (below MIN_SIMILARITY).
    let emb = Vector::from(vec![1.0f32; 1152]);
    // img_c is 180 degrees from emb => similarity ~ -1 (below MIN_SIMILARITY).
    let other: Vec<f32> = (0..1152).map(|i| if i < 576 { 1.0 } else { -1.0 }).collect();
    let other = Vector::from(other);
    seed_image(&pool, "img_a", &emb).await;
    seed_image(&pool, "img_b", &emb).await;
    seed_image(&pool, "img_c", &other).await;

    let status = fresh_status();
    let ran = process_batch(&main_pool(pool.clone()), &status).await.expect("process_batch ok");
    assert!(ran, "batch had work");

    let client = pool.get().await.unwrap();

    // DIAGNOSTIC: dump worker status + neighbor similarities on the same data.
    {
        let s = status.lock().await;
        eprintln!("DIAG ran={} total_images={} total_pairs={}", ran, s.total_images, s.total_pairs);
        drop(s);
    }
    {
        let emb = Vector::from(vec![1.0f32; 1152]);
        let params: &[&(dyn tokio_postgres::types::ToSql + Sync)] = &[&emb, &user_uuid()];
        let neighbors = client.query(
            "SELECT hash, (1.0 - (embedding <=> $1))::float4 AS sim FROM images WHERE user_id=$2 AND deleted_at IS NULL AND embedding IS NOT NULL AND hash!='img_a' ORDER BY embedding <=> $1 LIMIT 20",
            params,
        ).await.unwrap();
        for r in &neighbors { eprintln!("DIAG img_a -> {} sim={}", r.get::<_,String>(0), r.get::<_,f32>(1)); }
    }
    // DIAGNOSTIC
    {
        let emb = Vector::from(vec![1.0f32; 1152]);
        let rows = client.query(
            "SELECT hash, (1.0 - (embedding <=> $1))::float4 AS sim FROM images WHERE user_id=$2 AND hash!='x' ORDER BY embedding <=> $1 LIMIT 5",
            &[&emb, &user_uuid()],
        ).await.unwrap();
        for r in &rows { eprintln!("DIAG {} sim={}", r.get::<_,String>(0), r.get::<_,f32>(1)); }
    }
    let pairs: i64 = client
        .query_one("SELECT COUNT(*) FROM image_duplicate_pairs", &[])
        .await
        .unwrap()
        .get(0);
    assert_eq!(pairs, 1, "exactly one qualifying pair (img_a/img_b)");

    let row = client
        .query_one("SELECT hash_a, hash_b, similarity FROM image_duplicate_pairs LIMIT 1", &[])
        .await
        .unwrap();
    let ha: String = row.get(0);
    let hb: String = row.get(1);
    let sim: f32 = row.get(2);
    assert!(ha < hb, "pairs stored with hash_a < hash_b");
    assert!(sim > 0.99, "identical embeddings => near-1 similarity, got {}", sim);
    assert!(ha == "img_a" || ha == "img_b", "pair involves the similar images");

    let checked: i64 = client
        .query_one(
            "SELECT COUNT(*) FROM images WHERE duplicates_checked_at IS NOT NULL",
            &[],
        )
        .await
        .unwrap()
        .get(0);
    assert_eq!(checked, 3, "every image in the batch is marked checked");

    let s = status.lock().await;
    assert_eq!(s.total_images, 3);
    // total_pairs is the snapshot taken when process_batch STARTED (before the
    // batch inserted pairs), so it is 0 here even though the pair now exists.
    assert_eq!(s.total_pairs, 0);
}

#[actix_web::test]
#[serial]
async fn process_batch_reports_no_work_when_all_checked() {
    let (pool, _db) = setup_test_database_with_instance().await;
    let emb = Vector::from(vec![1.0f32; 1152]);
    seed_image(&pool, "img_a", &emb).await;

    let client = pool.get().await.unwrap();
    client
        .execute("UPDATE images SET duplicates_checked_at = NOW() WHERE hash = 'img_a'", &[])
        .await
        .unwrap();

    let status = fresh_status();
    let ran = process_batch(&main_pool(pool.clone()), &status).await.expect("process_batch ok");
    assert!(!ran, "no unchecked images -> no work");
    let s = status.lock().await;
    assert!(!s.running, "worker marks itself idle");
}

#[actix_web::test]
#[serial]
async fn process_batch_excludes_deleted_and_unverified_images() {
    let (pool, _db) = setup_test_database_with_instance().await;
    let emb = Vector::from(vec![1.0f32; 1152]);

    // Should be considered but similar pair (will be the only pair).
    seed_image(&pool, "keep_a", &emb).await;
    seed_image(&pool, "keep_b", &emb).await;
    // Same embedding as keep_a/b but deleted -> must NOT generate extra pairs.
    {
        let client = pool.get().await.unwrap();
        client
            .execute(
                "INSERT INTO images (user_id, deviceid, hash, name, ext, type, has_thumbnail, verification_status, embedding, deleted_at) \
                 VALUES ($1, 'dev', 'gone_a', 'gone.jpg', 'jpg', 'camera', true, 1, $2, NOW())",
                &[&user_uuid(), &emb],
            )
            .await
            .unwrap();
        // Unverified image (verification_status = 0) -> excluded from counts.
        client
            .execute(
                "INSERT INTO images (user_id, deviceid, hash, name, ext, type, has_thumbnail, verification_status, embedding) \
                 VALUES ($1, 'dev', 'unver', 'u.jpg', 'jpg', 'camera', true, 0, $2)",
                &[&user_uuid(), &emb],
            )
            .await
            .unwrap();
    }

    let status = fresh_status();
    let _ = process_batch(&main_pool(pool.clone()), &status).await.expect("process_batch ok");

    let s = status.lock().await;
    assert_eq!(s.total_images, 2, "deleted + unverified images are excluded from totals");
}
