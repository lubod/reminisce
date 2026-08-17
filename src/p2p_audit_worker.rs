//! Audits P2P shard consistency and repairs missing shards.
//!
//! Runs every 7 days. Cleans up orphaned shard rows for soft-deleted files, finds
//! files with fewer than DATA_SHARDS shards, and re-uploads the missing shards.
//! Large files are repaired by streaming the local file segment-by-segment.

use crate::config::Config;
use crate::p2p_upload::{retrieve_shard, rendezvous_select, SHARD_COUNT, MIN_NODES_REQUIRED};
use crate::shard_rebalance_worker::{find_file_info, upload_shard_to_node, lookup_node_addr};
use log::{info, warn, error};
use crate::metrics::{
    P2P_SHARDS_AUDITED_TOTAL, P2P_SHARDS_REPAIRED_TOTAL,
    P2P_SHARDS_REPAIR_FAILED_TOTAL, P2P_ORPHANED_SHARDS_CLEANED_TOTAL,
};
use deadpool_postgres::Pool;
use np2p::network::P2PService;
use np2p::storage::StorageEngine;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;
use tokio::io::AsyncReadExt;

const SEGMENT_THRESHOLD: usize = 256 * 1024 * 1024;

pub async fn start_audit_worker(
    pool: Pool,
    config: Config,
    p2p_service: Arc<P2PService>,
    shutdown_token: tokio_util::sync::CancellationToken,
) {
    info!("P2P Audit & Repair Worker started");

    let pool = pool.clone();
    let config = config.clone();
    let p2p_service = p2p_service.clone();
    let min_dur = Duration::from_secs(config.workers.audit_min_secs);
    let max_dur = Duration::from_secs(config.workers.audit_max_secs);
    crate::utils::run_worker_loop(
        "P2P Audit Worker",
        min_dur,
        max_dur,
        shutdown_token,
        move || {
            let pool = pool.clone();
            let config = config.clone();
            let p2p_service = p2p_service.clone();
            async move { perform_audit(&pool, &config, &p2p_service).await }
        }
    ).await;
}

async fn perform_audit(
    pool: &Pool,
    config: &Config,
    p2p_service: &Arc<P2PService>,
) -> Result<bool, String> {
        crate::shard_rebalance_worker::sync_db_nodes_to_registry(pool, p2p_service).await;
    let client = pool.get().await.map_err(|e| e.to_string())?;
    let rows = client.query(
        "SELECT id, file_hash, shard_index, node_id, shard_hash
         FROM p2p_shards
         WHERE last_checked_at IS NULL OR last_checked_at < NOW() - INTERVAL '7 days'
         LIMIT 50",
        &[]
    ).await.map_err(|e| e.to_string())?;

    if rows.is_empty() {
        // If we have no shards to audit, check for consistency issues
        // (files marked as synced but missing from shard table)
        return check_consistency(pool, config, p2p_service).await;
    }

    info!("Auditing {} distributed shards", rows.len());

    for row in rows {
        let shard_db_id: i64 = row.get(0);
        let file_hash: String = row.get(1);
        let shard_index: i32 = row.get(2);
        let node_id: String = row.get(3);
        let expected_shard_hash: String = row.get(4);

        let addr = match lookup_node_addr(pool, p2p_service, &node_id).await {
            Some(a) => a,
            None => {
                warn!("Cannot audit shard: unknown addr for node {}", node_id);
                continue;
            }
        };

        // retrieve_shard opens (and closes) its own QUIC connection, so we do not
        // open a second "probe" connection here — that would double the handshake
        // load on low-power peers during a large audit batch.
        let mut success = false;
        match retrieve_shard(p2p_service, addr, &expected_shard_hash).await {
            Ok(data) => {
                let actual_hash = blake3::hash(&data).to_hex().to_string();
                if actual_hash == expected_shard_hash {
                    success = true;
                } else {
                    warn!("Shard {} index {} on node {} is CORRUPTED!", file_hash, shard_index, node_id);
                }
            }
            Err(e) => {
                warn!("Shard {} index {} on node {} is MISSING: {}", file_hash, shard_index, node_id, e);
            }
        }

        P2P_SHARDS_AUDITED_TOTAL.inc();
        if success {
            let _ = client.execute(
                "UPDATE p2p_shards SET last_checked_at = NOW() WHERE id = $1",
                &[&shard_db_id]
            ).await;
        } else {
            info!("Triggering repair for file {} (shard {} lost)", file_hash, shard_index);
            match repair_file(pool, config, p2p_service, &file_hash, shard_index as usize).await {
                Ok(_) => P2P_SHARDS_REPAIRED_TOTAL.inc(),
                Err(e) => {
                    error!("Repair failed for {}: {}", file_hash, e);
                    P2P_SHARDS_REPAIR_FAILED_TOTAL.inc();
                }
            }
        }
    }

    Ok(true)
}

/// Delete p2p_shards rows whose file_hash has no matching non-deleted image/video,
/// optionally notifying remote storage nodes to delete the physical shard files.
/// Returns the count of purged rows.
pub async fn cleanup_orphaned_shards_with_service(
    pool: &Pool,
    p2p_service: Option<&Arc<P2PService>>,
) -> Result<u64, String> {
    let client = pool.get().await.map_err(|e| e.to_string())?;
    let rows = client.query(
        "SELECT node_id, shard_hash
         FROM p2p_shards
         WHERE file_hash NOT IN (
             SELECT hash FROM images WHERE deleted_at IS NULL
             UNION ALL
             SELECT hash FROM videos WHERE deleted_at IS NULL
         )
         LIMIT 1000",
        &[],
    ).await.map_err(|e| e.to_string())?;

    if let Some(p2p) = p2p_service {
        for row in &rows {
            let node_id: String = row.get(0);
            let shard_hash: String = row.get(1);
            if let Some(addr) = crate::shard_rebalance_worker::lookup_node_addr(pool, p2p, &node_id).await {
                let _ = crate::p2p_upload::delete_shard_remote(p2p, addr, &node_id, &shard_hash).await;
            }
        }
    }

    client.execute(
        "DELETE FROM p2p_shards
         WHERE file_hash NOT IN (
             SELECT hash FROM images WHERE deleted_at IS NULL
             UNION ALL
             SELECT hash FROM videos WHERE deleted_at IS NULL
         )",
        &[],
    ).await.map_err(|e| e.to_string())
}

/// Delete p2p_shards rows whose file_hash has no matching non-deleted image/video.
/// Returns the count of purged rows.
pub async fn cleanup_orphaned_shards(pool: &Pool) -> Result<u64, String> {
    cleanup_orphaned_shards_with_service(pool, None).await
}

/// Proactively audits each registered storage node, lists stored shard hashes,
/// compares them against Postgres (p2p_shards + db_backup_shards), and deletes
/// any unreferenced shards.
pub async fn sweep_storage_node_orphans(
    pool: &Pool,
    p2p_service: &Arc<P2PService>,
) -> Result<usize, String> {
    let client = pool.get().await.map_err(|e| e.to_string())?;
    let node_rows = client.query(
        "SELECT node_id, public_addr FROM p2p_nodes WHERE is_active = true",
        &[],
    ).await.map_err(|e| e.to_string())?;

    let mut total_pruned = 0;

    for node_row in node_rows {
        let node_id: String = node_row.get(0);
        let addr = match crate::shard_rebalance_worker::lookup_node_addr(pool, p2p_service, &node_id).await {
            Some(a) => a,
            None => continue,
        };

        info!("Auditing storage node {} ({}) for unreferenced shards", node_id, addr);

        for i in 0u8..=255 {
            let prefix = format!("{:02x}", i);
            let (shards, available_space) = match crate::p2p_upload::list_remote_shards(p2p_service, addr, Some(prefix.clone())).await {
                Ok(res) => res,
                Err(e) => {
                    warn!("Failed to list shards (prefix {}) from node {}: {}", prefix, node_id, e);
                    continue;
                }
            };

            crate::metrics::record_peer_write(&node_id, true, Some(available_space));

            if shards.is_empty() {
                continue;
            }

            let hex_shards: Vec<String> = shards.into_iter().map(hex::encode).collect();

            let valid_rows = client.query(
                "SELECT shard_hash FROM p2p_shards WHERE shard_hash = ANY($1)
                 UNION
                 SELECT shard_hash FROM db_backup_shards WHERE shard_hash = ANY($1)",
                &[&hex_shards],
            ).await.map_err(|e| e.to_string())?;

            let valid_set: std::collections::HashSet<String> = valid_rows.into_iter().map(|r| r.get(0)).collect();

            for shard_hash_hex in hex_shards {
                if !valid_set.contains(&shard_hash_hex) {
                    match crate::p2p_upload::delete_shard_remote(p2p_service, addr, &node_id, &shard_hash_hex).await {
                        Ok(true) => {
                            total_pruned += 1;
                            P2P_ORPHANED_SHARDS_CLEANED_TOTAL.inc();
                            info!("Pruned orphan shard {} from node {}", &shard_hash_hex[..16.min(shard_hash_hex.len())], node_id);
                        }
                        Ok(false) => {
                            warn!("Node {} refused to delete orphan shard {}", node_id, &shard_hash_hex[..16.min(shard_hash_hex.len())]);
                        }
                        Err(e) => {
                            warn!("Failed to delete orphan shard {} from node {}: {}", &shard_hash_hex[..16.min(shard_hash_hex.len())], node_id, e);
                        }
                    }
                }
            }
        }
    }

    Ok(total_pruned)
}

/// Return hashes of files marked as p2p_synced but with fewer than 3 shards in p2p_shards.
pub async fn find_undersharded_files(pool: &Pool, limit: i64) -> Result<Vec<String>, String> {
    let client = pool.get().await.map_err(|e| e.to_string())?;
    let rows = client.query(
        "WITH synced_files AS (
             SELECT hash FROM images WHERE p2p_synced_at IS NOT NULL
             UNION ALL
             SELECT hash FROM videos WHERE p2p_synced_at IS NOT NULL
         ),
         shard_counts AS (
             SELECT file_hash, count(*) as count FROM p2p_shards GROUP BY file_hash
         )
         SELECT s.hash
         FROM synced_files s
         LEFT JOIN shard_counts c ON s.hash = c.file_hash
         WHERE c.count IS NULL OR c.count < COALESCE(
             (SELECT p2p_data_shards FROM images WHERE hash = s.hash),
             (SELECT p2p_data_shards FROM videos WHERE hash = s.hash),
             3
         )
         LIMIT $1",
        &[&limit],
    ).await.map_err(|e| e.to_string())?;
    Ok(rows.iter().map(|r| r.get(0)).collect())
}

async fn check_consistency(
    pool: &Pool,
    config: &Config,
    p2p_service: &Arc<P2PService>,
) -> Result<bool, String> {
    crate::shard_rebalance_worker::sync_db_nodes_to_registry(pool, p2p_service).await;
    let deleted = cleanup_orphaned_shards_with_service(pool, Some(p2p_service)).await?;
    if deleted > 0 {
        info!("Consistency check: purged {} orphaned shard records for deleted files", deleted);
        P2P_ORPHANED_SHARDS_CLEANED_TOTAL.inc_by(deleted);
    }

    let pruned = sweep_storage_node_orphans(pool, p2p_service).await.unwrap_or(0);
    if pruned > 0 {
        info!("Consistency check: pruned {} unreferenced shard files across storage nodes", pruned);
    }

    let file_hashes = find_undersharded_files(pool, 10).await?;

    if file_hashes.is_empty() {
        return Ok(false);
    }

    let client = pool.get().await.map_err(|e| e.to_string())?;
    info!("Consistency check: Found {} files with missing/incomplete shards", file_hashes.len());

    for file_hash in file_hashes {
        let existing_count: i64 = client.query_one(
            "SELECT count(*) FROM p2p_shards WHERE file_hash = $1",
            &[&file_hash],
        ).await.map_err(|e| e.to_string())?.get(0);

        if existing_count == 0 {
            info!("Resetting p2p_synced_at for un-sharded file {} to trigger fresh replication", file_hash);
            let _ = client.execute("UPDATE images SET p2p_synced_at = NULL WHERE hash = $1", &[&file_hash]).await;
            let _ = client.execute("UPDATE videos SET p2p_synced_at = NULL WHERE hash = $1", &[&file_hash]).await;
            continue;
        }

        info!("Consistency check: Fixing missing shards for file {}", file_hash);
        for i in 0..SHARD_COUNT {
            match repair_file(pool, config, p2p_service, &file_hash, i).await {
                Ok(_) => P2P_SHARDS_REPAIRED_TOTAL.inc(),
                Err(e) => {
                    error!("Consistency check: Failed to fix shard {} for {}: {}", i, file_hash, e);
                    P2P_SHARDS_REPAIR_FAILED_TOTAL.inc();
                }
            }
        }
    }

    Ok(true)
}

async fn repair_file(
    pool: &Pool,
    config: &Config,
    p2p_service: &Arc<P2PService>,
    file_hash: &str,
    failed_shard_index: usize,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let client = pool.get().await?;

    // Determine the correct target node for this shard using live registry peers or DB fallback
    let mut active_nodes: Vec<(String, std::net::SocketAddr)> = p2p_service.registry.all()
        .into_iter().map(|p| (p.node_id, p.addr)).collect();

    if active_nodes.is_empty() {
        let node_rows = client.query(
            "SELECT node_id FROM p2p_nodes WHERE is_active = true",
            &[],
        ).await?;
        for row in node_rows {
            let nid: String = row.get(0);
            if let Some(addr) = crate::shard_rebalance_worker::lookup_node_addr(pool, p2p_service, &nid).await {
                active_nodes.push((nid, addr));
            }
        }
    }

    if active_nodes.len() < MIN_NODES_REQUIRED {
        return Err("Not enough active nodes for repair".into());
    }

    let ideal_nodes = rendezvous_select(file_hash, &active_nodes, SHARD_COUNT.min(active_nodes.len()));
    let (target_node_id, target_node_addr) = &ideal_nodes[failed_shard_index % ideal_nodes.len()];

    // Check if this is a segmented large file and get segment metadata
    let seg_row = client.query_opt(
        "SELECT ext, p2p_encryption_key, p2p_segment_count, p2p_segment_enc_sizes, p2p_data_shards, p2p_parity_shards \
         FROM images WHERE hash = $1 AND p2p_segment_count > 1 \
         UNION ALL \
         SELECT ext, p2p_encryption_key, p2p_segment_count, p2p_segment_enc_sizes, p2p_data_shards, p2p_parity_shards \
         FROM videos WHERE hash = $1 AND p2p_segment_count > 1 \
         LIMIT 1",
        &[&file_hash]
    ).await?;

    if let Some(seg_info) = seg_row {
        let ext: String = seg_info.get(0);
        let key_enc: Option<Vec<u8>> = seg_info.get(1);
        let data_shards = seg_info.get::<_, Option<i32>>(4).unwrap_or(3) as usize;
        let parity_shards = seg_info.get::<_, Option<i32>>(5).unwrap_or(2) as usize;
        let total_shards = data_shards + parity_shards;

        if let Some(key_enc) = key_enc {
            let api_secret = config.get_api_key().map_err(|e| format!("Failed to retrieve API key: {}", e))?;
            let key = crate::utils::decrypt_key(&key_enc, api_secret)?;

            info!("Repairing shard {} of segmented large file {} by streaming re-shard", failed_shard_index, file_hash);

            if failed_shard_index >= total_shards {
                return Err(format!("Shard index {} out of range (total {})", failed_shard_index, total_shards).into());
            }

            let images_path = PathBuf::from(config.get_images_dir())
                .join(&file_hash[0..2])
                .join(format!("{}.{}", file_hash, ext));
            let videos_path = PathBuf::from(config.get_videos_dir())
                .join(&file_hash[0..2])
                .join(format!("{}.{}", file_hash, ext));

            let file_path = if images_path.exists() { images_path }
                else if videos_path.exists() { videos_path }
                else { return Err(format!("Local file not found for segmented hash {}", file_hash).into()); };

            // Stream through the file in SEGMENT_THRESHOLD chunks, collecting the
            // sub-shard for failed_shard_index from each segment, then concatenate.
            // The same chunking (SEGMENT_THRESHOLD) and per-segment nonce derivation
            // as upload_segmented keeps sub-shard offsets aligned with the survivors.
            let mut file_handle = tokio::fs::File::open(&file_path).await?;
            let mut buf = vec![0u8; SEGMENT_THRESHOLD];
            let mut full_shard_data: Vec<u8> = Vec::new();
            let mut hasher = blake3::Hasher::new();
            let mut seg_idx: u32 = 0;

            loop {
                let mut total = 0usize;
                while total < buf.len() {
                    match file_handle.read(&mut buf[total..]).await? {
                        0 => break,
                        n => total += n,
                    }
                }
                if total == 0 { break; }
                hasher.update(&buf[..total]);

                let nonce_ctx: Vec<u8> = key.iter().chain(seg_idx.to_le_bytes().iter()).cloned().collect();
                let (sub_shards, _enc_size) = StorageEngine::process_for_backup(&buf[..total], &key, &nonce_ctx, data_shards, parity_shards)?;

                if failed_shard_index < sub_shards.len() {
                    full_shard_data.extend_from_slice(&sub_shards[failed_shard_index]);
                } else {
                    return Err(format!("Shard index {} out of range for segment {}", failed_shard_index, seg_idx).into());
                }
                seg_idx += 1;
            }

            if full_shard_data.is_empty() {
                return Err(format!("Segmented repair produced empty shard for {}", file_hash).into());
            }

            // A repaired shard is only valid if the local file is byte-identical to the
            // one that was originally sharded (identified by its BLAKE3 content hash).
            // If the file changed, re-sharding would produce a shard incompatible with
            // the surviving ones — refuse the repair instead of writing a corrupt shard.
            let local_hash = hasher.finalize().to_hex().to_string();
            if local_hash != file_hash {
                return Err(format!(
                    "Local file hash mismatch ({local_hash} != {file_hash}) — refusing to repair from a modified file"
                ).into());
            }

            let new_shard_hash = upload_shard_to_node(p2p_service, *target_node_addr, &full_shard_data).await?;
            client.execute(
                "INSERT INTO p2p_shards (file_hash, shard_index, node_id, shard_hash, last_checked_at)
                 VALUES ($1, $2, $3, $4, NOW())
                 ON CONFLICT (file_hash, shard_index) DO UPDATE SET node_id = $3, shard_hash = $4, last_checked_at = NOW()",
                &[&file_hash, &(failed_shard_index as i32), target_node_id, &new_shard_hash],
            ).await?;
            info!("Repaired shard {} of segmented file {} on node {}", failed_shard_index, file_hash, target_node_id);
            return Ok(());
        } else {
            return Err(format!("No encryption key stored for segmented file {} — cannot repair", file_hash).into());
        }
    }

    // Single-segment file path
    // Try re-sharding from local file if encryption key is stored
    let api_key = config.get_api_key().map_err(|e| format!("Failed to retrieve API key: {}", e))?;
    let file_info = find_file_info(&client, file_hash, api_key).await?;

    match file_info {
        Some((ext, Some(key), _enc_size, data_shards, parity_shards)) => {
            info!("Repairing shard {} of {} by re-sharding from local file", failed_shard_index, file_hash);

            let images_path = PathBuf::from(config.get_images_dir())
                .join(&file_hash[0..2])
                .join(format!("{}.{}", file_hash, ext));
            let videos_path = PathBuf::from(config.get_videos_dir())
                .join(&file_hash[0..2])
                .join(format!("{}.{}", file_hash, ext));

            let file_data = if images_path.exists() {
                tokio::fs::read(&images_path).await?
            } else if videos_path.exists() {
                tokio::fs::read(&videos_path).await?
            } else {
                return Err(format!("Local file not found for hash {}", file_hash).into());
            };

            // Only re-shard when the local file still matches the content hash the
            // surviving shards were created from (see segmented path above).
            let actual_hash = blake3::hash(&file_data).to_hex().to_string();
            if actual_hash != file_hash {
                return Err(format!(
                    "Local file hash mismatch ({} != {}) — refusing to repair from a modified file",
                    &actual_hash[..16], &file_hash[..16]
                ).into());
            }

            let (shards, _) = StorageEngine::process_for_backup(&file_data, &key, &key, data_shards, parity_shards)?;

            if failed_shard_index >= shards.len() {
                return Err(format!("Shard index {} out of range", failed_shard_index).into());
            }

            let shard_data = &shards[failed_shard_index];
            let new_shard_hash = upload_shard_to_node(p2p_service, *target_node_addr, shard_data).await?;

            client.execute(
                "INSERT INTO p2p_shards (file_hash, shard_index, node_id, shard_hash, last_checked_at)
                 VALUES ($1, $2, $3, $4, NOW())
                 ON CONFLICT (file_hash, shard_index) DO UPDATE SET node_id = $3, shard_hash = $4, last_checked_at = NOW()",
                &[&file_hash, &(failed_shard_index as i32), target_node_id, &new_shard_hash],
            ).await?;

            info!("Repaired shard {} of {} on node {}", failed_shard_index, file_hash, target_node_id);
            Ok(())
        }
        _ => {
            // Fallback: try to find the shard on other active nodes
            info!("No encryption key for {} - trying to find shard on other nodes", file_hash);

            let shard_rows = client.query(
                "SELECT shard_index, node_id, shard_hash FROM p2p_shards WHERE file_hash = $1 AND shard_index = $2",
                &[&file_hash, &(failed_shard_index as i32)],
            ).await?;

            if shard_rows.is_empty() {
                return Err("No shard record found in DB".into());
            }

            let expected_shard_hash: String = shard_rows[0].get(2);

            // Try each active node to find the shard (it may have been stored on a different node before)
            for (node_id, node_addr) in &active_nodes {
                if node_id == target_node_id {
                    continue; // Skip the target, we're trying to send it there
                }

                if let Ok(data) = retrieve_shard(p2p_service, *node_addr, &expected_shard_hash).await {
                    let actual_hash = blake3::hash(&data).to_hex().to_string();
                    if actual_hash == expected_shard_hash {
                        let new_hash = upload_shard_to_node(p2p_service, *target_node_addr, &data).await?;
                        client.execute(
                            "UPDATE p2p_shards SET node_id = $1, shard_hash = $2, last_checked_at = NOW() WHERE file_hash = $3 AND shard_index = $4",
                            &[target_node_id, &new_hash, &file_hash, &(failed_shard_index as i32)],
                        ).await?;

                        info!("Repaired shard {} of {} via fallback from node {}", failed_shard_index, file_hash, node_id);
                        return Ok(());
                    }
                }
            }

            Err(format!("Unrecoverable: shard {} of {} not found on any node and no encryption key stored", failed_shard_index, file_hash).into())
        }
    }
}
