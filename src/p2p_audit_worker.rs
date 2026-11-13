//! Audits P2P shard consistency and repairs missing shards.
//!
//! Runs every 7 days. Cleans up orphaned shard rows for soft-deleted files, finds
//! files with fewer than DATA_SHARDS shards, and re-uploads the missing shards.
//! Large files are repaired by streaming the local file segment-by-segment.

use crate::config::Config;
use crate::p2p_upload::{rendezvous_select, SHARD_COUNT, MIN_NODES_REQUIRED};
use crate::shard_rebalance_worker::{find_file_info, lookup_node_addr};
use log::{info, warn, error};
use crate::metrics::{
    P2P_SHARDS_AUDITED_TOTAL, P2P_SHARDS_REPAIRED_TOTAL,
    P2P_SHARDS_REPAIR_FAILED_TOTAL, P2P_ORPHANED_SHARDS_CLEANED_TOTAL,
};
use deadpool_postgres::Pool;
use np2p::network::P2PService;
use std::collections::HashSet;
use std::sync::Arc;
use std::time::Duration;

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
    let _worker_guard = crate::utils::P2P_WORKER_LOCK.lock().await;
    crate::shard_rebalance_worker::sync_db_nodes_to_registry(pool, p2p_service).await;

    // Purge orphaned shard rows in DB for soft-deleted media files on every cycle
    if let Ok(deleted) = cleanup_orphaned_shards_with_service(pool, Some(p2p_service)).await {
        if deleted > 0 {
            info!("Consistency check: purged {} orphaned shard records for deleted files", deleted);
            P2P_ORPHANED_SHARDS_CLEANED_TOTAL.inc_by(deleted);
        }
    }

    // Periodically sweep unreferenced physical shard files from storage nodes (every 24h)
    static LAST_ORPHAN_SWEEP: std::sync::atomic::AtomicI64 = std::sync::atomic::AtomicI64::new(0);
    let now = chrono::Utc::now().timestamp();
    if now - LAST_ORPHAN_SWEEP.load(std::sync::atomic::Ordering::Relaxed) > 86400 {
        LAST_ORPHAN_SWEEP.store(now, std::sync::atomic::Ordering::Relaxed);
        let pruned = sweep_storage_node_orphans(pool, p2p_service).await.unwrap_or(0);
        if pruned > 0 {
            info!("Consistency check: pruned {} unreferenced shard files across storage nodes", pruned);
        }
    }

    let client = pool.get().await.map_err(|e| e.to_string())?;
    let rows = client.query(
        "SELECT id, file_hash, shard_index, node_id, shard_hash
         FROM p2p_shards
         WHERE last_checked_at IS NULL OR last_checked_at < NOW() - INTERVAL '7 days'
         ORDER BY COALESCE(last_audit_attempt_at, last_checked_at) ASC NULLS FIRST, id ASC
         LIMIT 50",
        &[],
    ).await.map_err(|e| e.to_string())?;

    static LAST_CONSISTENCY_CHECK: std::sync::atomic::AtomicI64 = std::sync::atomic::AtomicI64::new(0);

    if rows.is_empty() {
        LAST_CONSISTENCY_CHECK.store(now, std::sync::atomic::Ordering::Relaxed);
        return check_consistency(pool, config, p2p_service).await;
    }

    info!("Auditing {} distributed shards", rows.len());

    for row in rows {
        let shard_db_id: i64 = row.get(0);
        let file_hash: String = row.get(1);
        let shard_index: i32 = row.get(2);
        let node_id: String = row.get(3);
        let expected_shard_hash: String = row.get(4);

        // Stamp every attempt first so unreachable nodes or failed repairs rotate
        // cleanly instead of head-of-line blocking the remaining shards.
        let _ = client.execute(
            "UPDATE p2p_shards SET last_audit_attempt_at = NOW() WHERE id = $1",
            &[&shard_db_id],
        ).await;

        let addr = match lookup_node_addr(pool, p2p_service, &node_id).await {
            Some(a) => a,
            None => {
                warn!("Cannot audit shard: unknown addr for node {}", node_id);
                continue;
            }
        };

        let verified = match crate::p2p_upload::verify_shard_proof(p2p_service, addr, &expected_shard_hash).await {
            Ok(true) => true,
            Ok(false) => {
                warn!("Shard {} index {} on node {} is MISSING or CORRUPTED!", file_hash, shard_index, node_id);
                false
            }
            Err(e) => {
                warn!("Shard {} index {} verification failed on node {}: {}", file_hash, shard_index, node_id, e);
                false
            }
        };

        P2P_SHARDS_AUDITED_TOTAL.inc();
        if verified {
            let _ = client.execute(
                "UPDATE p2p_shards SET last_checked_at = NOW(), last_audit_attempt_at = NOW() WHERE id = $1",
                &[&shard_db_id],
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

    // Periodically run undersharded repair even while the 7-day audit queue is non-empty.
    if now - LAST_CONSISTENCY_CHECK.load(std::sync::atomic::Ordering::Relaxed) > 21600 {
        LAST_CONSISTENCY_CHECK.store(now, std::sync::atomic::Ordering::Relaxed);
        if let Err(e) = check_consistency(pool, config, p2p_service).await {
            warn!("Periodic consistency check error: {}", e);
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
        "SELECT id, node_id, shard_hash
         FROM p2p_shards s
         WHERE NOT EXISTS (SELECT 1 FROM images i WHERE i.hash = s.file_hash AND i.deleted_at IS NULL)
           AND NOT EXISTS (SELECT 1 FROM videos v WHERE v.hash = s.file_hash AND v.deleted_at IS NULL)
         LIMIT 1000",
        &[],
    ).await.map_err(|e| e.to_string())?;

    if rows.is_empty() {
        return Ok(0);
    }

    let ids: Vec<i64> = rows.iter().map(|r| r.get(0)).collect();
    if let Some(p2p) = p2p_service {
        for row in &rows {
            let node_id: String = row.get(1);
            let shard_hash: String = row.get(2);
            if let Some(addr) = crate::shard_rebalance_worker::lookup_node_addr(pool, p2p, &node_id).await {
                let _ = crate::p2p_upload::delete_shard_remote(p2p, addr, &node_id, &shard_hash).await;
            }
        }
    }

    client.execute(
        "DELETE FROM p2p_shards WHERE id = ANY($1)",
        &[&ids],
    ).await.map_err(|e| e.to_string())
}

/// Delete p2p_shards rows whose file_hash has no matching non-deleted image/video.
/// Returns the count of purged rows.
pub async fn cleanup_orphaned_shards(pool: &Pool) -> Result<u64, String> {
    cleanup_orphaned_shards_with_service(pool, None).await
}

/// Proactively audits each registered storage node, lists stored shard hashes in a
/// single request, compares against Postgres (`p2p_shards` + `db_backup_shards`), and
/// deletes any unreferenced shards (honoring the recent-store grace period).
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

        let (shards, available_space) = match crate::p2p_upload::list_remote_shards(p2p_service, addr, None).await {
            Ok(res) => res,
            Err(e) => {
                warn!("Failed to list shards from node {}: {}", node_id, e);
                continue;
            }
        };

        crate::metrics::record_peer_write(&node_id, true, Some(available_space));
        if shards.is_empty() {
            continue;
        }

        let hex_shards: Vec<String> = shards.into_iter().map(hex::encode).collect();
        for batch in hex_shards.chunks(5000) {
            let batch_vec = batch.to_vec();
            let valid_rows = client.query(
                "SELECT shard_hash FROM p2p_shards WHERE shard_hash = ANY($1)
                 UNION
                 SELECT shard_hash FROM db_backup_shards WHERE shard_hash = ANY($1)",
                &[&batch_vec],
            ).await.map_err(|e| e.to_string())?;

            let valid_set: HashSet<String> = valid_rows.into_iter().map(|r| r.get(0)).collect();

            for shard_hash_hex in batch {
                if !valid_set.contains(shard_hash_hex) {
                    if crate::p2p_upload::is_recently_stored(&node_id, shard_hash_hex) {
                        continue;
                    }
                    match crate::p2p_upload::delete_shard_remote(p2p_service, addr, &node_id, shard_hash_hex).await {
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

/// Return hashes of files marked as `p2p_synced` with fewer than full parity shards in `p2p_shards`.
pub async fn find_undersharded_files(pool: &Pool, limit: i64) -> Result<Vec<String>, String> {
    let client = pool.get().await.map_err(|e| e.to_string())?;
    let rows = client.query(
        "WITH synced_files AS (
             SELECT hash FROM images WHERE p2p_synced_at IS NOT NULL AND deleted_at IS NULL
             UNION
             SELECT hash FROM videos WHERE p2p_synced_at IS NOT NULL AND deleted_at IS NULL
         ),
         shard_counts AS (
             SELECT file_hash, COUNT(*) AS count FROM p2p_shards GROUP BY file_hash
         )
         SELECT s.hash
         FROM synced_files s
         LEFT JOIN shard_counts c ON s.hash = c.file_hash
         WHERE c.count IS NULL OR c.count < COALESCE(
             (SELECT p2p_data_shards + p2p_parity_shards FROM images WHERE hash = s.hash AND deleted_at IS NULL LIMIT 1),
             (SELECT p2p_data_shards + p2p_parity_shards FROM videos WHERE hash = s.hash AND deleted_at IS NULL LIMIT 1),
             5
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
    let file_hashes = find_undersharded_files(pool, 10).await?;
    if file_hashes.is_empty() {
        return Ok(false);
    }

    let client = pool.get().await.map_err(|e| e.to_string())?;
    info!("Consistency check: Found {} undersharded files", file_hashes.len());

    for file_hash in file_hashes {
        let existing_rows = client.query(
            "SELECT shard_index FROM p2p_shards WHERE file_hash = $1",
            &[&file_hash],
        ).await.map_err(|e| e.to_string())?;

        if existing_rows.is_empty() {
            info!("Resetting p2p_synced_at for un-sharded file {} to trigger fresh replication", file_hash);
            let _ = client.execute("UPDATE images SET p2p_synced_at = NULL WHERE hash = $1", &[&file_hash]).await;
            let _ = client.execute("UPDATE videos SET p2p_synced_at = NULL WHERE hash = $1", &[&file_hash]).await;
            continue;
        }

        let existing: HashSet<i32> = existing_rows.into_iter().map(|r| r.get(0)).collect();
        let total_shards = client.query_opt(
            "SELECT p2p_data_shards + p2p_parity_shards FROM images WHERE hash = $1 \
             UNION ALL SELECT p2p_data_shards + p2p_parity_shards FROM videos WHERE hash = $1 LIMIT 1",
            &[&file_hash],
        ).await.ok().flatten().and_then(|r| r.get::<_, Option<i32>>(0)).unwrap_or(SHARD_COUNT as i32) as usize;

        info!("Consistency check: Healing missing shard slots for file {} (have {}/{})", file_hash, existing.len(), total_shards);
        for i in 0..total_shards {
            if existing.contains(&(i as i32)) {
                continue;
            }
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

    let mut active_nodes: Vec<(String, std::net::SocketAddr)> = p2p_service.registry.all()
        .into_iter().map(|p| (p.node_id, p.addr)).collect();

    if active_nodes.is_empty() {
        let node_rows = client.query(
            "SELECT node_id FROM p2p_nodes WHERE is_active = true",
            &[],
        ).await?;
        for row in node_rows {
            let nid: String = row.get(0);
            if let Some(addr) = lookup_node_addr(pool, p2p_service, &nid).await {
                active_nodes.push((nid, addr));
            }
        }
    }

    if active_nodes.len() < MIN_NODES_REQUIRED {
        return Err("Not enough active nodes for repair".into());
    }

    let api_key = config.get_api_key().map_err(|e| format!("Failed to retrieve API key: {}", e))?;
    let file_info = find_file_info(&client, file_hash, api_key).await?;

    let (data_shards, parity_shards) = match file_info {
        Some(ref info) => (info.data_shards, info.parity_shards),
        None => (3, 2),
    };
    let total_shards = data_shards + parity_shards;
    if failed_shard_index >= total_shards {
        return Err(format!("Shard index {} out of range (total {})", failed_shard_index, total_shards).into());
    }

    let ideal_nodes = rendezvous_select(file_hash, &active_nodes, total_shards.min(active_nodes.len()));
    let (target_node_id, target_node_addr) = &ideal_nodes[failed_shard_index % ideal_nodes.len()];

    if let Some(info) = file_info {
        if let Some(key) = info.encryption_key {
            if let Some(local_path) = crate::p2p_upload::find_local_media_path(config, file_hash, &info.ext) {
                info!("Repairing shard {} of {} (segments={}) via streaming reshard", failed_shard_index, file_hash, info.segment_count);
                let new_shard_hash = crate::p2p_upload::stream_reshard_single_index(
                    p2p_service,
                    *target_node_addr,
                    &local_path,
                    file_hash,
                    &key,
                    info.segment_count,
                    failed_shard_index,
                    info.data_shards,
                    info.parity_shards,
                ).await?;

                client.execute(
                    "INSERT INTO p2p_shards (file_hash, shard_index, node_id, shard_hash, last_checked_at, last_audit_attempt_at)
                     VALUES ($1, $2, $3, $4, NOW(), NOW())
                     ON CONFLICT (file_hash, shard_index) DO UPDATE SET node_id = $3, shard_hash = $4, last_checked_at = NOW(), last_audit_attempt_at = NOW()",
                    &[&file_hash, &(failed_shard_index as i32), target_node_id, &new_shard_hash],
                ).await?;

                info!("Repaired shard {} of {} on node {}", failed_shard_index, file_hash, target_node_id);
                return Ok(());
            }
        }
    }

    // Fallback: stream shard from another active node if present.
    info!("No local file/key for {} — searching other nodes for shard {}", file_hash, failed_shard_index);
    let shard_row = client.query_opt(
        "SELECT shard_hash FROM p2p_shards WHERE file_hash = $1 AND shard_index = $2",
        &[&file_hash, &(failed_shard_index as i32)],
    ).await?;

    let Some(shard_row) = shard_row else {
        return Err("No shard record found in DB".into());
    };
    let expected_shard_hash: String = shard_row.get(0);

    for (node_id, node_addr) in &active_nodes {
        if node_id == target_node_id {
            continue;
        }
        if let Ok(new_hash) = crate::p2p_upload::copy_shard_between_nodes(
            p2p_service,
            *node_addr,
            *target_node_addr,
            &expected_shard_hash,
            failed_shard_index as u8,
        ).await {
            client.execute(
                "UPDATE p2p_shards SET node_id = $1, shard_hash = $2, last_checked_at = NOW(), last_audit_attempt_at = NOW()
                 WHERE file_hash = $3 AND shard_index = $4",
                &[target_node_id, &new_hash, &file_hash, &(failed_shard_index as i32)],
            ).await?;
            info!("Repaired shard {} of {} via fallback from node {}", failed_shard_index, file_hash, node_id);
            return Ok(());
        }
    }

    Err(format!("Unrecoverable: shard {} of {} not found on any node and no local file/key available", failed_shard_index, file_hash).into())
}
