use actix_web::web;
use chrono::{DateTime, Utc};
use log::{error, info, warn};
use std::sync::Arc;
use tokio::sync::Mutex;
use tokio::time::{sleep, Duration};

use crate::db::MainDbPool;

/// Shared status for the duplicate detection background worker.
#[derive(Clone)]
pub struct DuplicateWorkerStatus {
    pub running: bool,
    pub checked_images: i64,
    pub total_images: i64,
    pub total_pairs: i64,
    pub last_completed_at: Option<DateTime<Utc>>,
}

impl DuplicateWorkerStatus {
    pub fn new() -> Self {
        Self {
            running: false,
            checked_images: 0,
            total_images: 0,
            total_pairs: 0,
            last_completed_at: None,
        }
    }
}

pub type SharedDuplicateStatus = Arc<Mutex<DuplicateWorkerStatus>>;

/// Minimum similarity threshold stored in the pairs table.
/// Lower than the UI default (0.95) so the table covers all useful thresholds.
const MIN_SIMILARITY: f64 = 0.80;
const NUM_NEIGHBORS: i64 = 20;
const BATCH_SIZE: i64 = 50;
/// Sleep between batches to avoid hammering the DB.
const BATCH_SLEEP_MS: u64 = 200;
/// Sleep when idle (no unchecked images).
const IDLE_SLEEP_SECS: u64 = 300;

pub async fn start_duplicate_worker(
    pool: web::Data<MainDbPool>,
    status: SharedDuplicateStatus,
) {
    info!("Duplicate worker started");

    loop {
        let client = match pool.0.get().await {
            Ok(c) => c,
            Err(e) => {
                error!("Duplicate worker: failed to get DB client: {}", e);
                sleep(Duration::from_secs(30)).await;
                continue;
            }
        };

        // Count total and unchecked images
        let counts = client.query_one(
            "SELECT \
               COUNT(*) FILTER (WHERE embedding IS NOT NULL AND deleted_at IS NULL AND verification_status = 1) AS total, \
               COUNT(*) FILTER (WHERE duplicates_checked_at IS NULL AND embedding IS NOT NULL AND deleted_at IS NULL AND verification_status = 1) AS unchecked \
             FROM images",
            &[],
        ).await;

        let (total_images, unchecked): (i64, i64) = match counts {
            Ok(row) => (row.get(0), row.get(1)),
            Err(e) => {
                error!("Duplicate worker: count query failed: {}", e);
                sleep(Duration::from_secs(60)).await;
                continue;
            }
        };

        // Count stored pairs
        let pair_count: i64 = match client.query_one("SELECT COUNT(*) FROM image_duplicate_pairs", &[]).await {
            Ok(row) => row.get(0),
            Err(_) => 0,
        };

        {
            let mut s = status.lock().await;
            s.total_images = total_images;
            s.checked_images = total_images - unchecked;
            s.total_pairs = pair_count;
        }

        if unchecked == 0 {
            {
                let mut s = status.lock().await;
                s.running = false;
                s.last_completed_at = Some(Utc::now());
            }
            sleep(Duration::from_secs(IDLE_SLEEP_SECS)).await;
            continue;
        }

        {
            status.lock().await.running = true;
        }

        // Fetch a batch of unchecked images
        let batch = match client.query(
            "SELECT hash, deviceid, user_id, embedding \
             FROM images \
             WHERE duplicates_checked_at IS NULL \
               AND embedding IS NOT NULL \
               AND deleted_at IS NULL \
               AND verification_status = 1 \
             LIMIT $1",
            &[&BATCH_SIZE],
        ).await {
            Ok(rows) => rows,
            Err(e) => {
                error!("Duplicate worker: batch query failed: {}", e);
                sleep(Duration::from_secs(30)).await;
                continue;
            }
        };

        for row in &batch {
            let hash: String = row.get(0);
            let deviceid: String = row.get(1);
            let user_id: uuid::Uuid = row.get(2);
            let embedding: pgvector::Vector = row.get(3);

            // HNSW search: nearest neighbors for this image within the same user's library
            let neighbors = match client.query(
                "SELECT hash, (1.0 - (embedding <=> $1))::float4 AS similarity \
                 FROM images \
                 WHERE user_id = $2 \
                   AND deleted_at IS NULL \
                   AND embedding IS NOT NULL \
                   AND hash != $3 \
                 ORDER BY embedding <=> $1 \
                 LIMIT $4",
                &[&embedding, &user_id, &hash, &NUM_NEIGHBORS],
            ).await {
                Ok(rows) => rows,
                Err(e) => {
                    warn!("Duplicate worker: neighbor query failed for {}: {}", hash, e);
                    // Still mark as checked so we don't retry forever on broken rows
                    let _ = client.execute(
                        "UPDATE images SET duplicates_checked_at = NOW() WHERE hash = $1 AND deviceid = $2",
                        &[&hash, &deviceid],
                    ).await;
                    continue;
                }
            };

            // Insert qualifying pairs
            for n_row in &neighbors {
                let n_hash: String = n_row.get(0);
                let similarity: f32 = n_row.get(1);

                if (similarity as f64) < MIN_SIMILARITY {
                    continue;
                }

                // Normalise ordering: hash_a < hash_b
                let (ha, hb) = if hash < n_hash {
                    (hash.as_str(), n_hash.as_str())
                } else {
                    (n_hash.as_str(), hash.as_str())
                };

                if let Err(e) = client.execute(
                    "INSERT INTO image_duplicate_pairs (hash_a, hash_b, similarity, user_id) \
                     VALUES ($1, $2, $3, $4) \
                     ON CONFLICT (hash_a, hash_b, user_id) DO UPDATE \
                       SET similarity = GREATEST(excluded.similarity, image_duplicate_pairs.similarity), \
                           computed_at = NOW()",
                    &[&ha, &hb, &similarity, &user_id],
                ).await {
                    warn!("Duplicate worker: insert pair failed ({}, {}): {}", ha, hb, e);
                }
            }

            // Mark image as checked
            if let Err(e) = client.execute(
                "UPDATE images SET duplicates_checked_at = NOW() WHERE hash = $1 AND deviceid = $2",
                &[&hash, &deviceid],
            ).await {
                warn!("Duplicate worker: mark checked failed for {}: {}", hash, e);
            }
        }

        sleep(Duration::from_millis(BATCH_SLEEP_MS)).await;
    }
}
