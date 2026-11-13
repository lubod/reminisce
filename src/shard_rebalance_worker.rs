//! Migrates P2P shards to their ideal nodes when node topology changes.
//!
//! After adding or removing a storage node the rendezvous hash assignments shift.
//! This worker moves shards from their current location to the correct node so the
//! distribution stays balanced. Triggered via POST /api/p2p/backup/rebalance.

use crate::config::Config;
use crate::p2p_upload::{rendezvous_select, MIN_NODES_REQUIRED};
use deadpool_postgres::Pool;
use log::{info, warn, error};
use np2p::network::P2PService;
use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;

use std::sync::atomic::{AtomicBool, Ordering};

pub static REBALANCE_ACTIVE: AtomicBool = AtomicBool::new(false);

const REBALANCE_BATCH_SIZE: i64 = 50;

/// Sync dynamically discovered peers into p2p_nodes.
/// Upserts active nodes and marks stale nodes inactive only after 10m of inactivity.
pub async fn ensure_peers_registered(pool: &Pool, nodes: &[(String, SocketAddr)]) -> Result<(), String> {
    let client = pool.get().await.map_err(|e| e.to_string())?;

    for (node_id, addr) in nodes {
        client.execute(
            "INSERT INTO p2p_nodes (node_id, public_addr, is_active)
             VALUES ($1, $2, TRUE)
             ON CONFLICT (node_id) DO UPDATE SET public_addr = $2, is_active = TRUE, last_seen = NOW()",
            &[node_id, &addr.to_string()],
        ).await.map_err(|e| format!("Failed to upsert peer {}: {}", node_id, e))?;
    }

    // Mark peers that have not been seen in the last 10 minutes as inactive
    client.execute(
        "UPDATE p2p_nodes SET is_active = FALSE WHERE last_seen < NOW() - INTERVAL '10 minutes'",
        &[],
    ).await.map_err(|e| e.to_string())?;

    Ok(())
}

/// Runs rebalance cycles continuously until all un-rebalanced shards have been distributed.
pub async fn run_full_rebalance(
    pool: &Pool,
    config: &Config,
    p2p_service: &Arc<P2PService>,
) -> Result<usize, String> {
    let mut total_batches = 0;
    info!("Starting full P2P shard rebalance sweep...");
    loop {
        match rebalance_cycle(pool, config, p2p_service).await {
            Ok(true) => {
                total_batches += 1;
                info!("Full rebalance sweep: completed batch {} (~{} files evaluated)", total_batches, total_batches * (REBALANCE_BATCH_SIZE as usize));
                tokio::task::yield_now().await;
            }
            Ok(false) => {
                info!("Full rebalance sweep completed. Processed {} total batches.", total_batches);
                break;
            }
            Err(e) => {
                error!("Rebalance cycle error: {}", e);
                return Err(e);
            }
        }
    }
    Ok(total_batches)
}

pub async fn start_rebalance_worker(
    pool: Pool,
    config: Config,
    p2p_service: Arc<P2PService>,
    shutdown_token: tokio_util::sync::CancellationToken,
) {
    info!("Shard Rebalance Worker started");

    let pool = pool.clone();
    let config = config.clone();
    let p2p_service = p2p_service.clone();
    let min_dur = Duration::from_secs(config.workers.rebalance_min_secs);
    let max_dur = Duration::from_secs(config.workers.rebalance_max_secs);
    crate::utils::run_worker_loop(
        "Shard Rebalance Worker",
        min_dur,
        max_dur,
        shutdown_token,
        move || {
            let pool = pool.clone();
            let config = config.clone();
            let p2p_service = p2p_service.clone();
            async move { rebalance_cycle(&pool, &config, &p2p_service).await }
        }
    ).await;
}

pub async fn rebalance_cycle(
    pool: &Pool,
    config: &Config,
    p2p_service: &Arc<P2PService>,
) -> Result<bool, String> {
    // Mutual exclusion: periodic worker AND admin-triggered sweeps must never
    // migrate the same shard rows concurrently (last-writer-wins orphans the
    // losing copy with no cleanup path).
    if REBALANCE_ACTIVE
        .compare_exchange(false, true, Ordering::Acquire, Ordering::Relaxed)
        .is_err()
    {
        return Err("rebalance already in progress".to_string());
    }
    let res = rebalance_cycle_inner(pool, config, p2p_service).await;
    REBALANCE_ACTIVE.store(false, Ordering::Release);
    res
}

async fn rebalance_cycle_inner(
    pool: &Pool,
    config: &Config,
    p2p_service: &Arc<P2PService>,
) -> Result<bool, String> {
    let _worker_guard = crate::utils::P2P_WORKER_LOCK.lock().await;
    // Sync discovered peers to DB and DB peers to in-memory registry
    let discovered: Vec<(String, SocketAddr)> = p2p_service.registry.all()
        .into_iter()
        .map(|p| (p.node_id, p.addr))
        .collect();
    ensure_peers_registered(pool, &discovered).await?;
    sync_db_nodes_to_registry(pool, p2p_service).await;

    let client = pool.get().await.map_err(|e| e.to_string())?;

    let active_node_rows = client.query(
        "SELECT node_id, public_addr FROM p2p_nodes WHERE is_active = TRUE AND last_seen > NOW() - INTERVAL '10 minutes' ORDER BY node_id",
        &[],
    ).await.map_err(|e| e.to_string())?;

    let active_nodes: Vec<(String, SocketAddr)> = active_node_rows.into_iter().filter_map(|r| {
        let nid: String = r.get(0);
        let addr_str: String = r.get(1);
        addr_str.parse::<SocketAddr>().ok().map(|a| (nid, a))
    }).collect();

    if active_nodes.len() < MIN_NODES_REQUIRED {
        return Ok(false);
    }

    let target_node_count = active_nodes.len().min(crate::p2p_upload::SHARD_COUNT) as i64;
    // Find files needing rebalance, ordered by least-recently-checked so missing files never stall the sweep
    let rows = client.query(
        "SELECT s.file_hash, MIN(s.last_checked_at) as min_checked
         FROM p2p_shards s
         GROUP BY s.file_hash
         HAVING COUNT(DISTINCT s.node_id) < $1
            AND (MIN(s.last_checked_at) IS NULL OR MIN(s.last_checked_at) < NOW() - INTERVAL '1 hour')
         ORDER BY min_checked NULLS FIRST
         LIMIT $2",
        &[&target_node_count, &REBALANCE_BATCH_SIZE],
    ).await.map_err(|e| e.to_string())?;

    if rows.is_empty() {
        return Ok(false);
    }

    info!("Rebalancing batch of {} files across {} active nodes ({:?})", rows.len(), active_nodes.len(), active_nodes.iter().map(|n| n.1.to_string()).collect::<Vec<_>>());

    for row in &rows {
        let file_hash: String = row.get(0);
        match rebalance_file(pool, config, p2p_service, &file_hash, &active_nodes).await {
            Ok(_) => {}
            Err(e) => {
                error!("Rebalance error for {}: {}", file_hash, e);
            }
        }
    }

    Ok(true)
}

async fn rebalance_file(
    pool: &Pool,
    config: &Config,
    p2p_service: &Arc<P2PService>,
    file_hash: &str,
    active_nodes: &[(String, SocketAddr)],
) -> Result<bool, Box<dyn std::error::Error + Send + Sync>> {
    let client = pool.get().await?;



    // Load current shard assignments
    let shard_rows = client.query(
        "SELECT shard_index, node_id, shard_hash FROM p2p_shards WHERE file_hash = $1 ORDER BY shard_index",
        &[&file_hash],
    ).await?;

    let api_key = config.get_api_key().map_err(|e| format!("Failed to retrieve API key: {}", e))?;
    let file_info = find_file_info(&client, file_hash, api_key).await?;
    let (data_shards, parity_shards) = match file_info {
        Some(ref info) => (info.data_shards, info.parity_shards),
        None => (3, 2),
    };
    let total_shards = data_shards + parity_shards;

    // Compute ideal placement
    let ideal_nodes = rendezvous_select(file_hash, active_nodes, total_shards.min(active_nodes.len()));

    let mut migrated_any = false;

    for shard_row in &shard_rows {
        let shard_index: i32 = shard_row.get(0);
        let current_node: String = shard_row.get(1);
        let _current_shard_hash: String = shard_row.get(2);

        let idx = shard_index as usize;
        // With fewer nodes than shards, we use round-robin to pick from the ideal nodes.
        let (ideal_node_id, ideal_node_addr) = &ideal_nodes[idx % ideal_nodes.len()];
        if &current_node == ideal_node_id {
            continue; // Already on the correct node
        }

        info!("Rebalancing file {} shard {} from {} to {}", file_hash, shard_index, current_node, ideal_node_id);

        match migrate_shard(pool, config, p2p_service, file_hash, idx, ideal_node_id, *ideal_node_addr, &current_node, &_current_shard_hash).await {
            Ok(new_shard_hash) => {
                client.execute(
                    "UPDATE p2p_shards SET node_id = $1, shard_hash = $2 WHERE file_hash = $3 AND shard_index = $4",
                    &[ideal_node_id, &new_shard_hash, &file_hash, &shard_index],
                ).await?;
                migrated_any = true;
                info!("Migrated shard {} of {} to {}", shard_index, file_hash, ideal_node_id);

                // Clean up the old shard on the previous node
                if let Some(old_addr) = lookup_node_addr(pool, p2p_service, &current_node).await {
                    let _ = crate::p2p_upload::delete_shard_remote(p2p_service, old_addr, &current_node, &_current_shard_hash).await;
                }
            }
            Err(e) => {
                warn!("Failed to migrate shard {} of {}: {}", shard_index, file_hash, e);
            }
        }
    }

    // Always update last_checked_at so un-migratable files do not block future sweeps
    let _ = client.execute(
        "UPDATE p2p_shards SET last_checked_at = NOW() WHERE file_hash = $1",
        &[&file_hash],
    ).await;

    Ok(migrated_any)
}

/// Migrate a shard to a new node. Prefers streaming re-shard from local file
/// (handles single- and multi-segment files in bounded RAM); falls back to streaming
/// the shard directly from the old node.
#[allow(clippy::too_many_arguments)]
async fn migrate_shard(
    pool: &Pool,
    config: &Config,
    p2p_service: &Arc<P2PService>,
    file_hash: &str,
    shard_index: usize,
    _new_node_id: &str,
    new_node_addr: SocketAddr,
    old_node_id: &str,
    old_shard_hash: &str,
) -> Result<String, Box<dyn std::error::Error + Send + Sync>> {
    let client = pool.get().await?;
    let api_key = config.get_api_key().map_err(|e| format!("Failed to retrieve API key: {}", e))?;
    let file_info = find_file_info(&client, file_hash, api_key).await?;

    if let Some(info) = file_info {
        if let Some(key) = info.encryption_key {
            if let Some(local_path) = crate::p2p_upload::find_local_media_path(config, file_hash, &info.ext) {
                match crate::p2p_upload::stream_reshard_single_index(
                    p2p_service,
                    new_node_addr,
                    &local_path,
                    file_hash,
                    &key,
                    info.segment_count,
                    shard_index,
                    info.data_shards,
                    info.parity_shards,
                ).await {
                    Ok(new_shard_hash) => {
                        if new_shard_hash == old_shard_hash {
                            return Ok(new_shard_hash);
                        }
                        // Integrity gate: unexpected shard hash mismatch — delete uploaded copy and try fallback.
                        let nid = p2p_service.registry.find_by_addr(new_node_addr).unwrap_or_default();
                        let _ = crate::p2p_upload::delete_shard_remote(p2p_service, new_node_addr, &nid, &new_shard_hash).await;
                        warn!(
                            "stream_reshard integrity mismatch for {}[{}]: {} != {} — falling back to old node",
                            file_hash, shard_index, &new_shard_hash[..16.min(new_shard_hash.len())], &old_shard_hash[..16.min(old_shard_hash.len())]
                        );
                    }
                    Err(e) => {
                        warn!("Local streamed reshard failed for {}: {} — falling back to old node", file_hash, e);
                    }
                }
            }
        }
    }

    let old_addr = lookup_node_addr(pool, p2p_service, old_node_id)
        .await
        .ok_or("Cannot migrate: old node addr unknown")?;
    let transferred = crate::p2p_upload::copy_shard_between_nodes(
        p2p_service,
        old_addr,
        new_node_addr,
        old_shard_hash,
        shard_index as u8,
    )
    .await?;
    Ok(transferred)
}

/// Sync all active nodes from the database into the in-memory P2P service registry.
pub async fn sync_db_nodes_to_registry(pool: &Pool, p2p_service: &Arc<P2PService>) {
    if let Ok(client) = pool.get().await {
        if let Ok(rows) = client.query(
            "SELECT node_id, public_addr FROM p2p_nodes WHERE is_active = true",
            &[],
        ).await {
            for row in rows {
                let node_id: String = row.get(0);
                let addr_str: String = row.get(1);
                if let Ok(addr) = addr_str.parse::<SocketAddr>() {
                    p2p_service.registry.upsert(node_id, addr);
                }
            }
        }
    }
}

/// Resolve a node's socket address: check in-memory registry first, then DB public_addr (upserting on DB hit).
pub async fn lookup_node_addr(pool: &Pool, p2p_service: &Arc<P2PService>, node_id: &str) -> Option<SocketAddr> {
    // 1. Fast path: in-memory registry
    if let Some(peer) = p2p_service.registry.get(node_id) {
        return Some(peer.addr);
    }
    // 2. Slow path: DB
    if let Ok(client) = pool.get().await {
        if let Ok(Some(row)) = client.query_opt(
            "SELECT public_addr FROM p2p_nodes WHERE node_id = $1 AND is_active = true",
            &[&node_id],
        ).await {
            let addr_str: String = row.get(0);
            if let Ok(addr) = addr_str.parse::<SocketAddr>() {
                p2p_service.registry.upsert(node_id.to_string(), addr);
                return Some(addr);
            }
        }
    }
    None
}

/// Resolved metadata required to re-shard or validate a local media file.
#[derive(Debug, Clone)]
pub struct LocalFileInfo {
    pub ext: String,
    pub encryption_key: Option<Vec<u8>>,
    pub encrypted_size: Option<i64>,
    pub segment_count: i32,
    pub data_shards: usize,
    pub parity_shards: usize,
}

/// Look up file info (`LocalFileInfo`) from `images` or `videos`.
pub async fn find_file_info(
    client: &tokio_postgres::Client,
    file_hash: &str,
    api_secret: &str,
) -> Result<Option<LocalFileInfo>, Box<dyn std::error::Error + Send + Sync>> {
    for table in ["images", "videos"] {
        crate::utils::validate_table_name(table)?;
        let query = format!(
            "SELECT ext, p2p_encryption_key, p2p_encrypted_size::BIGINT, \
                    COALESCE(p2p_segment_count, 1), p2p_data_shards, p2p_parity_shards \
             FROM {} WHERE hash = $1 LIMIT 1",
            table
        );
        if let Some(row) = client.query_opt(&query, &[&file_hash]).await? {
            let ext: String = row.get(0);
            let key_enc: Option<Vec<u8>> = row.get(1);
            let encrypted_size: Option<i64> = row.get(2);
            let segment_count: i32 = row.get(3);
            let data_shards: usize = row.get::<_, Option<i32>>(4).unwrap_or(3) as usize;
            let parity_shards: usize = row.get::<_, Option<i32>>(5).unwrap_or(2) as usize;
            let encryption_key = match key_enc {
                Some(k) => Some(crate::utils::decrypt_key(&k, api_secret)?),
                None => None,
            };
            return Ok(Some(LocalFileInfo {
                ext,
                encryption_key,
                encrypted_size,
                segment_count,
                data_shards,
                parity_shards,
            }));
        }
    }

    Ok(None)
}
