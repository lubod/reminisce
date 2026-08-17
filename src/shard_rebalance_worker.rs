//! Migrates P2P shards to their ideal nodes when node topology changes.
//!
//! After adding or removing a storage node the rendezvous hash assignments shift.
//! This worker moves shards from their current location to the correct node so the
//! distribution stays balanced. Triggered via POST /api/p2p/backup/rebalance.

use crate::config::Config;
use crate::p2p_upload::{rendezvous_select, MIN_NODES_REQUIRED};
use deadpool_postgres::Pool;
use log::{info, warn, error};
use np2p::network::{P2PService, Message, Protocol};
use np2p::storage::StorageEngine;
use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use std::sync::atomic::{AtomicBool, Ordering};

pub static REBALANCE_ACTIVE: AtomicBool = AtomicBool::new(false);

const REBALANCE_BATCH_SIZE: i64 = 50;
const UPLOAD_STREAM_THRESHOLD: usize = 64 * 1024 * 1024; // 64 MB — stay under Pi's 100 MB recv limit
const UPLOAD_CHUNK_SIZE: usize = 32 * 1024 * 1024;       // 32 MB chunks

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
    REBALANCE_ACTIVE.store(true, Ordering::Relaxed);
    let res = rebalance_cycle_inner(pool, config, p2p_service).await;
    if let Ok(false) | Err(_) = &res {
        REBALANCE_ACTIVE.store(false, Ordering::Relaxed);
    }
    res
}

async fn rebalance_cycle_inner(
    pool: &Pool,
    config: &Config,
    p2p_service: &Arc<P2PService>,
) -> Result<bool, String> {
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
        Some((_, _, _, ds, ps)) => (ds, ps),
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

/// Migrate a shard to a new node. Prefers re-sharding from local file (when encryption key is stored).
/// Falls back to retrieving the shard from the old node if no key is available.
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

    // Prefer re-sharding from the local file. If the local file no longer matches its
    // content hash (reshard_from_local refuses to re-shard a modified file), or local
    // re-sharding fails for any other reason, fall back to pulling the shard from the
    // old node.
    let shard_data = match file_info {
        Some((ext, Some(key), _enc_size, data_shards, parity_shards)) => {
            match reshard_from_local(config, file_hash, &ext, &key, shard_index, data_shards, parity_shards).await {
                Ok(data) => data,
                Err(e) => {
                    warn!("reshard_from_local failed for {}: {} — falling back to old node", file_hash, e);
                    retrieve_shard_from_old_node(pool, p2p_service, old_node_id, old_shard_hash).await?
                }
            }
        }
        _ => {
            retrieve_shard_from_old_node(pool, p2p_service, old_node_id, old_shard_hash).await?
        }
    };

    let shard_hash = blake3::hash(&shard_data).to_hex().to_string();
    upload_shard_to_node(p2p_service, new_node_addr, &shard_data).await?;

    Ok(shard_hash)
}

/// Look up the old node's address and pull the shard from it.
async fn retrieve_shard_from_old_node(
    pool: &Pool,
    p2p_service: &Arc<P2PService>,
    old_node_id: &str,
    old_shard_hash: &str,
) -> Result<Vec<u8>, Box<dyn std::error::Error + Send + Sync>> {
    let old_addr = lookup_node_addr(pool, p2p_service, old_node_id).await;
    match old_addr {
        Some(addr) => retrieve_shard_from_node(p2p_service, addr, old_shard_hash).await
            .map_err(|_| -> Box<dyn std::error::Error + Send + Sync> {
                "Cannot migrate: old node unreachable".into()
            }),
        None => Err("Cannot migrate: old node addr unknown".into()),
    }
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

/// Look up file info (ext, encryption_key, encrypted_size) from images or videos table.
pub async fn find_file_info(
    client: &tokio_postgres::Client,
    file_hash: &str,
    api_secret: &str,
) -> Result<Option<(String, Option<Vec<u8>>, Option<i32>, usize, usize)>, Box<dyn std::error::Error + Send + Sync>> {
    // Try images first
    let row = client.query_opt(
        "SELECT ext, p2p_encryption_key, p2p_encrypted_size, p2p_data_shards, p2p_parity_shards FROM images WHERE hash = $1 LIMIT 1",
        &[&file_hash],
    ).await?;

    if let Some(row) = row {
        let ext: String = row.get(0);
        let key_enc: Option<Vec<u8>> = row.get(1);
        let enc_size: Option<i32> = row.get(2);
        let data_shards: i32 = row.get::<_, Option<i32>>(3).unwrap_or(3);
        let parity_shards: i32 = row.get::<_, Option<i32>>(4).unwrap_or(2);
        let key = match key_enc {
            Some(k) => Some(crate::utils::decrypt_key(&k, api_secret)?),
            None => None,
        };
        return Ok(Some((ext, key, enc_size, data_shards as usize, parity_shards as usize)));
    }

    // Try videos
    let row = client.query_opt(
        "SELECT ext, p2p_encryption_key, p2p_encrypted_size, p2p_data_shards, p2p_parity_shards FROM videos WHERE hash = $1 LIMIT 1",
        &[&file_hash],
    ).await?;

    if let Some(row) = row {
        let ext: String = row.get(0);
        let key_enc: Option<Vec<u8>> = row.get(1);
        let enc_size: Option<i32> = row.get(2);
        let data_shards: i32 = row.get::<_, Option<i32>>(3).unwrap_or(3);
        let parity_shards: i32 = row.get::<_, Option<i32>>(4).unwrap_or(2);
        let key = match key_enc {
            Some(k) => Some(crate::utils::decrypt_key(&k, api_secret)?),
            None => None,
        };
        return Ok(Some((ext, key, enc_size, data_shards as usize, parity_shards as usize)));
    }

    Ok(None)
}

/// Re-encrypt and re-shard a file from local disk, returning the specific shard.
async fn reshard_from_local(
    config: &Config,
    file_hash: &str,
    ext: &str,
    encryption_key: &[u8],
    shard_index: usize,
    data_shards: usize,
    parity_shards: usize,
) -> Result<Vec<u8>, Box<dyn std::error::Error + Send + Sync>> {
    // Try images dir first, then videos
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

    // Re-sharding produces shards that must be byte-compatible with the survivors
    // on the other nodes. If the local file changed since it was sharded (its BLAKE3
    // no longer matches the content hash), refuse to migrate from it so we don't end
    // up with an inconsistent shard set.
    let actual_hash = blake3::hash(&file_data).to_hex().to_string();
    if actual_hash != file_hash {
        return Err(format!(
            "Local file hash mismatch ({} != {}): refusing to migrate from a modified file",
            &actual_hash[..16], &file_hash[..16]
        ).into());
    }

    let (shards, _enc_size) = StorageEngine::process_for_backup(&file_data, encryption_key, encryption_key, data_shards, parity_shards)?;

    if shard_index >= shards.len() {
        return Err(format!("Shard index {} out of range ({})", shard_index, shards.len()).into());
    }

    Ok(shards[shard_index].clone())
}

/// Retrieve a shard from a remote node.
async fn retrieve_shard_from_node(
    p2p_service: &Arc<P2PService>,
    addr: SocketAddr,
    shard_hash_hex: &str,
) -> Result<Vec<u8>, Box<dyn std::error::Error + Send + Sync>> {
    crate::p2p_upload::retrieve_shard(p2p_service, addr, shard_hash_hex)
        .await
        .map_err(|e| e.into())
}

/// Upload a shard to a remote node.
/// Uses streaming protocol for shards > 64 MB to stay under the Pi's 100 MB message limit.
pub async fn upload_shard_to_node(
    p2p_service: &Arc<P2PService>,
    addr: SocketAddr,
    shard_data: &[u8],
) -> Result<String, Box<dyn std::error::Error + Send + Sync>> {
    let node_id = p2p_service.registry.find_by_addr(addr)
        .unwrap_or_else(|| addr.to_string());
    let result = upload_shard_to_node_inner(p2p_service, addr, &node_id, shard_data).await;

    // Surface per-peer write health from this code path too (it stores shards on
    // repair/migrate, so disk-full rejections would otherwise be invisible). The
    // inner helpers update the available-space gauge on success; this records the
    // last-write status and success/failure counters for every attempt.
    crate::metrics::record_peer_write(&node_id, result.is_ok(), None);
    result
}

async fn upload_shard_to_node_inner(
    p2p_service: &Arc<P2PService>,
    addr: SocketAddr,
    node_id: &str,
    shard_data: &[u8],
) -> Result<String, Box<dyn std::error::Error + Send + Sync>> {
    let conn = p2p_service.connect_to_addr(addr).await
        .map_err(|e| format!("Connection to {} failed: {}", addr, e))?;

    let shard_hash = blake3::hash(shard_data);
    let shard_hash_bytes: [u8; 32] = shard_hash.into();
    let shard_hash_hex = shard_hash.to_hex().to_string();

    if shard_data.len() > UPLOAD_STREAM_THRESHOLD {
        let (mut send, mut recv) = conn.open_bi().await?;
        // Token binds the stream to (file_hash, shard_index) — the storage node verifies
        // it before accepting any chunk (matches the non-streamed StoreShardRequest auth).
        let binding: [u8; 32] = blake3::hash(
            &[shard_hash_bytes.as_slice(), &[0u8]].concat()
        ).into();
        let token = p2p_service.identity().create_shard_token(np2p::crypto::ShardOp::Store, &binding);
        Protocol::send(&mut send, &Message::StoreShardStreamInit {
            file_hash: shard_hash_bytes,
            shard_index: 0,
            total_shard_bytes: shard_data.len() as u64,
            segment_count: 1,
            token,
        }).await.map_err(|e| e.to_string())?;

        match Protocol::receive(&mut recv).await.map_err(|e| e.to_string())? {
            Message::StoreShardStreamAck { ready: true } => {}
            other => return Err(format!("Unexpected stream ack from {}: {:?}", addr, other).into()),
        }

        for chunk in shard_data.chunks(UPLOAD_CHUNK_SIZE) {
            Protocol::send(&mut send, &Message::StoreShardChunk { data: chunk.to_vec() })
                .await.map_err(|e| e.to_string())?;
        }

        Protocol::send(&mut send, &Message::StoreShardStreamFinal { shard_hash: shard_hash_bytes })
            .await.map_err(|e| e.to_string())?;

        match Protocol::receive(&mut recv).await.map_err(|e| e.to_string())? {
            Message::StoreShardStreamResponse { success: true, available_space_bytes } => {
                crate::metrics::P2P_PEER_AVAILABLE_SPACE_BYTES
                    .with_label_values(&[node_id]).set(available_space_bytes as f64);
                conn.close(0u32.into(), b"done");
                Ok(shard_hash_hex)
            }
            _ => {
                conn.close(0u32.into(), b"done");
                Err(format!("Node {} rejected large shard (stream)", addr).into())
            }
        }
    } else {
        let (mut send, mut recv) = conn.open_bi().await?;
        let token = p2p_service.identity().create_shard_token(np2p::crypto::ShardOp::Store, &shard_hash_bytes);
        Protocol::send(&mut send, &Message::StoreShardRequest {
            shard_hash: shard_hash_bytes,
            data: shard_data.to_vec(),
            token,
        }).await.map_err(|e| e.to_string())?;

        match Protocol::receive(&mut recv).await.map_err(|e| e.to_string())? {
            Message::StoreShardResponse { success: true, available_space_bytes, .. } => {
                crate::metrics::P2P_PEER_AVAILABLE_SPACE_BYTES
                    .with_label_values(&[node_id]).set(available_space_bytes as f64);
                conn.close(0u32.into(), b"done");
                Ok(shard_hash_hex)
            }
            _ => {
                conn.close(0u32.into(), b"done");
                Err(format!("Node {} rejected shard", addr).into())
            }
        }
    }
}
