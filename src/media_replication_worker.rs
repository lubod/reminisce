//! Replicates media files to P2P storage nodes.
//!
//! Selects 5 target nodes per file using rendezvous (HRW) hashing, encrypts with
//! ChaCha20Poly1305, erasure-codes into 3/5 Reed-Solomon shards, and uploads each
//! shard via QUIC. Handles large files (>256 MB) by streaming in 256 MB segments.

use crate::config::Config;
use log::{info, warn, error};
use deadpool_postgres::Pool;
use std::net::SocketAddr;
use std::sync::Arc;
use std::path::{Path, PathBuf};
use futures::stream::{self, StreamExt};
use std::time::Duration;
use crate::utils::{get_load_average, get_cpu_count, calculate_worker_concurrency};
use crate::metrics::{
    BACKUP_PEERS_AVAILABLE, BACKUP_ATTEMPTS_TOTAL, BACKUP_SUCCESS_TOTAL,
    BACKUP_FAILURES_TOTAL, BACKUP_SIZE_BYTES, BACKUP_DURATION_SECONDS,
};
use crate::p2p_upload::{self, MIN_NODES_REQUIRED, SEGMENT_THRESHOLD};
use np2p::network::P2PService;
use np2p::storage::StorageEngine;

// Constants
// Batch size is now configurable via workers.replication_batch_size (default 50).

struct MediaToReplicate {
    hash: String,
    ext: String,
}

pub async fn media_replication_loop(
    pool: Pool,
    config: Config,
    p2p_service: Arc<P2PService>,
    shutdown_token: tokio_util::sync::CancellationToken,
) {
    info!("P2P Media Replication Worker started (3/5 EC, rendezvous hashing)");
    // Give LAN discovery time to register the Pi before the first batch.
    tokio::time::sleep(Duration::from_secs(20)).await;

    let pool = pool.clone();
    let config = config.clone();
    let p2p_service = p2p_service.clone();
    let min_dur = Duration::from_secs(config.workers.replication_min_secs);
    let max_dur = Duration::from_secs(config.workers.replication_max_secs);
    crate::utils::run_worker_loop(
        "Media Replication Worker",
        min_dur,
        max_dur,
        shutdown_token,
        move || {
            let pool = pool.clone();
            let config = config.clone();
            let p2p_service = p2p_service.clone();
            async move { replicate_all(&pool, &config, &p2p_service).await }
        }
    ).await;
}

async fn replicate_all(
    pool: &Pool,
    config: &Config,
    p2p_service: &Arc<P2PService>,
) -> Result<bool, String> {
    // Use dynamically discovered peers from the in-memory registry (LAN + coordinator)
    let nodes: Vec<(String, SocketAddr)> = p2p_service.registry.all()
        .into_iter()
        .map(|p| (p.node_id, p.addr))
        .collect();

    BACKUP_PEERS_AVAILABLE.set(nodes.len() as i64);

    if nodes.is_empty() {
        return Ok(false);
    }

    if nodes.len() < MIN_NODES_REQUIRED {
        return Ok(false);
    }

    if nodes.len() < 3 {
        warn!("Only {} P2P nodes discovered. 3+ nodes recommended for 3/5 EC redundancy.", nodes.len());
    }

    // Re-queue files that lost shard redundancy (fewer than the full data+parity complement
    // reachable) so the batches below re-replicate them to a full set. Bounded per cycle so a
    // node outage can't trigger a 50k+ file burst in one pass.
    let target_shards = (config.p2p_data_shards + config.p2p_parity_shards) as i64;
    let requeued = match requeue_under_replicated(pool, config.workers.replication_batch_size.max(1), target_shards).await {
        Ok(n) => n,
        Err(e) => {
            log::error!("Failed to re-queue under-replicated files: {}", e);
            0
        }
    };
    if requeued > 0 {
        info!("Re-queued {} under-replicated file(s) for re-replication", requeued);
    }

    let images_done = replicate_batch(pool, config, p2p_service, &nodes, "images").await
        .map_err(|e| format!("Failed to replicate image batch: {}", e))?;

    let videos_done = replicate_batch(pool, config, p2p_service, &nodes, "videos").await
        .map_err(|e| format!("Failed to replicate video batch: {}", e))?;

    Ok(images_done || videos_done || requeued > 0)
}

/// Resets `p2p_synced_at` for synced media files whose currently-reachable shard count is below
/// `target_shards`, so the next replication batch re-shards them from their local originals.
/// Only shards on recently-active nodes count, mirroring the status/verify queries. Capped to
/// `limit` files per table per cycle to keep self-healing bounded.
async fn requeue_under_replicated(
    pool: &Pool,
    limit: i64,
    target_shards: i64,
) -> Result<u64, String> {
    let client = pool.get().await.map_err(|e| e.to_string())?;

    let mut total: u64 = 0;
    for table in ["images", "videos"] {
        crate::utils::validate_table_name(table)?;
        let query = format!(
            "UPDATE {} SET p2p_synced_at = NULL
             WHERE deleted_at IS NULL AND p2p_synced_at IS NOT NULL
               AND hash IN (
                 SELECT i.hash FROM {} i
                 LEFT JOIN (
                   SELECT s.file_hash, COUNT(s.id) FILTER (WHERE n.node_id IS NOT NULL) AS sc
                   FROM p2p_shards s
                   LEFT JOIN p2p_nodes n ON n.node_id = s.node_id
                     AND n.is_active = TRUE AND n.last_seen > NOW() - INTERVAL '10 minutes'
                   GROUP BY s.file_hash
                 ) t ON t.file_hash = i.hash
                 WHERE COALESCE(t.sc, 0) < $1
                 ORDER BY i.created_at ASC
                 LIMIT $2
               )",
            table, table
        );
        let n = client.execute(&query, &[&target_shards, &limit])
            .await.map_err(|e| e.to_string())?;
        total += n;
    }
    Ok(total)
}

async fn replicate_batch(
    pool: &Pool,
    config: &Config,
    p2p_service: &Arc<P2PService>,
    nodes: &[(String, SocketAddr)],
    table: &str,
) -> Result<bool, Box<dyn std::error::Error + Send + Sync>> {
    let load_average = get_load_average().await;
    let cpu_count = get_cpu_count();
    let limits = calculate_worker_concurrency(load_average, 0, cpu_count);

    if limits.is_overloaded() {
        return Ok(false);
    }

    let query = format!(
        "SELECT hash, name, ext
         FROM {}
         WHERE p2p_synced_at IS NULL
         ORDER BY created_at ASC
         LIMIT $1",
        table
    );

    let client = pool.get().await?;
    let batch_size = config.workers.replication_batch_size.max(1);
    let rows = client.query(&query, &[&batch_size]).await?;

    let files: Vec<MediaToReplicate> = rows.iter().map(|row| {
        MediaToReplicate {
            hash: row.get(0),
            ext: row.get(2),
        }
    }).collect();

    if files.is_empty() {
        return Ok(false);
    }

    info!("Found {} {} to shard and replicate", files.len(), table);

    let base_dir = if table == "images" { config.get_images_dir() } else { config.get_videos_dir() };

    let api_secret = config.get_api_key().map_err(|e| {
        log::error!("Replication worker: failed to retrieve API key: {}", e);
        std::io::Error::other(e)
    })?.to_string();

    let successes = std::sync::atomic::AtomicUsize::new(0);

    // Videos load entire files into memory for encryption+erasure coding; process one at a time
    // so that the 256 MB segment budget is not multiplied by concurrency.
    let concurrency = if table == "videos" { 1 } else { limits.verification };

    stream::iter(files)
        .for_each_concurrent(concurrency, |file| {
            let pool_clone = pool.clone();
            let p2p_service_clone = p2p_service.clone();
            let base_dir_owned = base_dir.to_string();
            let table_owned = table.to_string();
            let nodes_owned = nodes.to_vec();
            let success_counter = &successes;

            let api_secret_clone = api_secret.clone();
            let config_clone = config.clone();

            async move {
                BACKUP_ATTEMPTS_TOTAL.inc();
                match replicate_single_file(
                    &pool_clone,
                    &p2p_service_clone,
                    &base_dir_owned,
                    &table_owned,
                    &nodes_owned,
                    &file,
                    &api_secret_clone,
                    &config_clone,
                ).await {
                    Ok(_) => {
                        success_counter.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                        BACKUP_SUCCESS_TOTAL.inc();
                    }
                    Err(e) => {
                        error!("Failed to replicate {}: {}", file.hash, e);
                        BACKUP_FAILURES_TOTAL.inc();
                    }
                }
            }
        })
        .await;

    Ok(successes.load(std::sync::atomic::Ordering::Relaxed) > 0)
}

#[allow(clippy::too_many_arguments)]
async fn replicate_single_file(
    pool: &Pool,
    p2p_service: &Arc<P2PService>,
    base_dir: &str,
    table: &str,
    nodes: &[(String, SocketAddr)],
    file: &MediaToReplicate,
    api_secret: &str,
    config: &Config,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let file_path = PathBuf::from(base_dir)
        .join(&file.hash[0..2])
        .join(format!("{}.{}", file.hash, file.ext));

    if !file_path.exists() {
        warn!("File {} not found on disk — skipping replication (will retry next cycle)", file.hash);
        return Ok(());
    }

    let metadata = tokio::fs::metadata(&file_path).await?;
    let file_size = metadata.len() as usize;

    let data_shards = config.p2p_data_shards;
    let parity_shards = config.p2p_parity_shards;
    let total_shards = data_shards + parity_shards;

    let start = std::time::Instant::now();

    // Route large files through the segmented streaming path to cap peak RAM at ~940 MB.
    if file_size > SEGMENT_THRESHOLD {
        let res = replicate_large_file(pool, p2p_service, base_dir, table, nodes, file, &file_path, api_secret, data_shards, parity_shards).await;
        if res.is_ok() {
            BACKUP_SIZE_BYTES.observe(file_size as f64);
            BACKUP_DURATION_SECONDS.observe(start.elapsed().as_secs_f64());
        }
        return res;
    }

    // 1. Encrypt and Shard
    let file_data = tokio::fs::read(&file_path).await?;
    let mut encryption_key = [0u8; 32];
    rand::fill(&mut encryption_key);

    // nonce_context = key: key is randomly generated once per file, ensuring unique nonce per file.
    let (shards, enc_size) = StorageEngine::process_for_backup(&file_data, &encryption_key, &encryption_key, data_shards, parity_shards)?;

    // 2. Upload shards in parallel to rendezvous-selected nodes.
    info!("Sharding {} into {} pieces (rendezvous)", file.hash, shards.len());
    let final_results = p2p_upload::upload_inmemory(p2p_service, nodes, &file.hash, shards).await?;

    if final_results.len() < data_shards {
        return Err(format!("Only {}/{} shards stored. Minimum {} required (for reconstruction).", final_results.len(), total_shards, data_shards).into());
    }

    // 4. Update Database
    let mut client = pool.get().await?;

    // Upsert nodes first in a separate statement (outside the per-file transaction)
    // so concurrent file transactions don't race on the same p2p_nodes rows.
    for s in final_results.iter() {
        client.execute(
            "INSERT INTO p2p_nodes (node_id, public_addr, is_active)
             VALUES ($1, $2, TRUE)
             ON CONFLICT (node_id) DO UPDATE SET public_addr = $2, is_active = TRUE, last_seen = NOW()",
            &[&s.node_id, &s.addr],
        ).await?;
    }

    let trans = client.transaction().await?;

    for s in final_results.iter() {
        trans.execute(
            "INSERT INTO p2p_shards (file_hash, shard_index, node_id, shard_hash)
             VALUES ($1, $2, $3, $4)
             ON CONFLICT (file_hash, shard_index) DO UPDATE SET node_id = $3, shard_hash = $4",
            &[&file.hash, &(s.idx as i32), &s.node_id, &s.shard_hash_hex]
        ).await?;
    }

    // Mark as synced and store the encryption key + encrypted size for future re-sharding
    let enc_size_i32 = enc_size as i32;
    let data_shards_i32 = data_shards as i32;
    let parity_shards_i32 = parity_shards as i32;
    let encrypted_key = crate::utils::encrypt_key(&encryption_key, api_secret)?;
    // Compute a manifest hash: BLAKE3 over all stored shard hashes concatenated.
    let mut manifest_hasher = blake3::Hasher::new();
    for s in final_results.iter() { manifest_hasher.update(s.shard_hash_hex.as_bytes()); }
    let manifest_hash = manifest_hasher.finalize().to_hex().to_string();

    crate::utils::validate_table_name(table)?;
    let update_query = format!(
        "UPDATE {} SET p2p_synced_at = NOW(), p2p_shard_hash = $1, p2p_encryption_key = $2, p2p_encrypted_size = $3, p2p_data_shards = $4, p2p_parity_shards = $5 WHERE hash = $6",
        table
    );
    trans.execute(&update_query, &[&manifest_hash, &encrypted_key, &enc_size_i32, &data_shards_i32, &parity_shards_i32, &file.hash]).await?;

    trans.commit().await?;

    // Append to escrow file for key recovery
    append_key_to_escrow(base_dir, &file.hash, &encrypted_key);

    info!("Replicated {}: {} shards stored (rendezvous)", file.hash, final_results.len());
    BACKUP_SIZE_BYTES.observe(file_size as f64);
    BACKUP_DURATION_SECONDS.observe(start.elapsed().as_secs_f64());
    Ok(())
}

/// Replicates a file larger than SEGMENT_THRESHOLD by processing it in 256 MB segments.
/// Opens one persistent QUIC stream per shard, then streams sub-shard chunks across all
/// segments before finalising with a BLAKE3 hash. Peak RAM ≈ 940 MB regardless of file size.
///
/// The per-file encryption key is stored wrapped with the master secret (same as the
/// single-segment path) and appended to the on-disk escrow, so large-file keys survive
/// a database loss and are never stored in plaintext in the DB.
#[allow(clippy::too_many_arguments)]
async fn replicate_large_file(
    pool: &Pool,
    p2p_service: &Arc<P2PService>,
    base_dir: &str,
    table: &str,
    nodes: &[(String, SocketAddr)],
    file: &MediaToReplicate,
    file_path: &Path,
    api_secret: &str,
    data_shards: usize,
    parity_shards: usize,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let mut encryption_key = [0u8; 32];
    rand::fill(&mut encryption_key);

    info!("Replicating large file {} using segmented streaming", file.hash);
    let (shard_results, segment_enc_sizes) = p2p_upload::upload_segmented(
        p2p_service, nodes, &file.hash, file_path, &encryption_key, data_shards, parity_shards,
    ).await?;

    if shard_results.len() < data_shards {
        return Err(format!(
            "Only {}/{} shards stored for large file {}. Minimum {} required.",
            shard_results.len(), data_shards + parity_shards, file.hash, data_shards
        ).into());
    }

    let encrypted_key = crate::utils::encrypt_key(&encryption_key, api_secret)?;

    // Update database
    let mut client = pool.get().await?;
    for r in &shard_results {
        client.execute(
            "INSERT INTO p2p_nodes (node_id, public_addr, is_active) VALUES ($1, $2, TRUE)
             ON CONFLICT (node_id) DO UPDATE SET public_addr = $2, is_active = TRUE, last_seen = NOW()",
            &[&r.node_id, &r.addr],
        ).await?;
    }

    let trans = client.transaction().await?;
    for r in &shard_results {
        trans.execute(
            "INSERT INTO p2p_shards (file_hash, shard_index, node_id, shard_hash) VALUES ($1, $2, $3, $4)
             ON CONFLICT (file_hash, shard_index) DO UPDATE SET node_id = $3, shard_hash = $4",
            &[&file.hash, &(r.idx as i32), &r.node_id, &r.shard_hash_hex],
        ).await?;
    }

    let segment_count = segment_enc_sizes.len() as i32;
    let update_query = format!(
        "UPDATE {} SET p2p_synced_at = NOW(), p2p_encryption_key = $2, \
         p2p_encrypted_size = 0, p2p_segment_count = $3, p2p_segment_enc_sizes = $4, \
         p2p_data_shards = $5, p2p_parity_shards = $6 WHERE hash = $1",
        table
    );
    trans.execute(&update_query, &[&file.hash, &encrypted_key, &segment_count, &segment_enc_sizes, &(data_shards as i32), &(parity_shards as i32)]).await?;
    trans.commit().await?;

    // Append to escrow file for key recovery (same as the single-segment path).
    append_key_to_escrow(base_dir, &file.hash, &encrypted_key);

    info!("Replicated {} ({} segments, {} shards stored)", file.hash, segment_count, shard_results.len());
    Ok(())
}

/// Best-effort append of `hash,encrypted_key` to the `p2p_keys.escrow` file that
/// lives next to the media data directory (mode 0600), so backup keys survive a
/// database loss.
fn append_key_to_escrow(base_dir: &str, hash: &str, encrypted_key: &[u8]) {
    use std::io::Write;
    let abs_base = std::fs::canonicalize(base_dir).unwrap_or_else(|_| PathBuf::from(base_dir));
    let escrow_path = abs_base.parent().unwrap_or(&abs_base).join("p2p_keys.escrow");

    let mut options = std::fs::OpenOptions::new();
    options.create(true).append(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }

    if let Ok(mut escrow_file) = options.open(&escrow_path) {
        let line = format!("{},{}\n", hash, hex::encode(encrypted_key));
        let _ = escrow_file.write_all(line.as_bytes());
    }
}
