use crate::config::Config;
use crate::media_replication_worker::{rendezvous_select_nodes, SHARD_COUNT, MIN_NODES_REQUIRED};
use deadpool_postgres::Pool;
use log::{info, warn, error};
use np2p::network::{P2PService, Message, Protocol};
use np2p::storage::StorageEngine;
use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

const REBALANCE_BATCH_SIZE: i64 = 20;

/// Sync dynamically discovered peers into p2p_nodes.
/// Upserts active nodes; marks any node not currently in the registry as inactive.
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

    // Mark peers not currently seen as inactive
    if !nodes.is_empty() {
        let placeholders: Vec<String> = nodes.iter().enumerate()
            .map(|(i, _)| format!("${}", i + 1))
            .collect();
        let query = format!(
            "UPDATE p2p_nodes SET is_active = FALSE WHERE node_id NOT IN ({})",
            placeholders.join(", ")
        );
        let node_ids: Vec<&str> = nodes.iter().map(|(id, _)| id.as_str()).collect();
        let params: Vec<&(dyn tokio_postgres::types::ToSql + Sync)> = node_ids.iter()
            .map(|id| id as &(dyn tokio_postgres::types::ToSql + Sync))
            .collect();
        client.execute(&query, &params).await.map_err(|e| e.to_string())?;
    }

    Ok(())
}

pub async fn start_rebalance_worker(
    pool: Pool,
    config: Config,
    p2p_service: Arc<P2PService>,
) {
    info!("Shard Rebalance Worker started");

    crate::utils::run_worker_loop(
        "Shard Rebalance Worker",
        Duration::from_secs(120),
        Duration::from_secs(3600),
        || {
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
    // Get currently discovered peers and sync to DB
    let active_nodes: Vec<(String, SocketAddr)> = p2p_service.registry.all()
        .into_iter()
        .map(|p| (p.node_id, p.addr))
        .collect();

    ensure_peers_registered(pool, &active_nodes).await?;

    if active_nodes.len() < MIN_NODES_REQUIRED {
        return Ok(false);
    }

    let client = pool.get().await.map_err(|e| e.to_string())?;

    // Get a batch of file hashes that have shard assignments
    let rows = client.query(
        "SELECT DISTINCT file_hash FROM p2p_shards LIMIT $1",
        &[&REBALANCE_BATCH_SIZE],
    ).await.map_err(|e| e.to_string())?;

    if rows.is_empty() {
        return Ok(false);
    }

    let mut did_work = false;

    for row in &rows {
        let file_hash: String = row.get(0);

        match rebalance_file(pool, config, p2p_service, &file_hash, &active_nodes).await {
            Ok(migrated) => {
                if migrated {
                    did_work = true;
                }
            }
            Err(e) => {
                error!("Rebalance failed for {}: {}", file_hash, e);
            }
        }
    }

    Ok(did_work)
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

    // Compute ideal placement
    let ideal_nodes = rendezvous_select_nodes(file_hash, active_nodes, SHARD_COUNT.min(active_nodes.len()));

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
            }
            Err(e) => {
                warn!("Failed to migrate shard {} of {}: {}", shard_index, file_hash, e);
            }
        }
    }

    Ok(migrated_any)
}

/// Migrate a shard to a new node. Prefers re-sharding from local file (when encryption key is stored).
/// Falls back to retrieving the shard from the old node if no key is available.
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

    let file_info = find_file_info(&client, file_hash).await?;

    let shard_data = match file_info {
        Some((ext, Some(key), _enc_size)) => {
            reshard_from_local(config, file_hash, &ext, &key, shard_index).await?
        }
        _ => {
            // Look up old node's addr from DB public_addr or in-memory registry
            let old_addr = lookup_node_addr(pool, p2p_service, old_node_id).await;
            match old_addr {
                Some(addr) => retrieve_shard_from_node(p2p_service, addr, old_shard_hash).await
                    .or_else(|_| -> Result<Vec<u8>, Box<dyn std::error::Error + Send + Sync>> {
                        Err("Cannot migrate: no encryption key and old node unreachable".into())
                    })?,
                None => return Err("Cannot migrate: no encryption key and old node addr unknown".into()),
            }
        }
    };

    let shard_hash = blake3::hash(&shard_data).to_hex().to_string();
    upload_shard_to_node(p2p_service, new_node_addr, &shard_data).await?;

    Ok(shard_hash)
}

/// Resolve a node's socket address: check in-memory registry first, then DB public_addr.
pub async fn lookup_node_addr(pool: &Pool, p2p_service: &Arc<P2PService>, node_id: &str) -> Option<SocketAddr> {
    // Fast path: in-memory registry
    if let Some(peer) = p2p_service.registry.get(node_id) {
        return Some(peer.addr);
    }
    // Slow path: DB
    if let Ok(client) = pool.get().await {
        if let Ok(Some(row)) = client.query_opt(
            "SELECT public_addr FROM p2p_nodes WHERE node_id = $1",
            &[&node_id],
        ).await {
            let addr_str: String = row.get(0);
            if let Ok(addr) = addr_str.parse() {
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
) -> Result<Option<(String, Option<Vec<u8>>, Option<i32>)>, Box<dyn std::error::Error + Send + Sync>> {
    // Try images first
    let row = client.query_opt(
        "SELECT ext, p2p_encryption_key, p2p_encrypted_size FROM images WHERE hash = $1 LIMIT 1",
        &[&file_hash],
    ).await?;

    if let Some(row) = row {
        let ext: String = row.get(0);
        let key: Option<Vec<u8>> = row.get(1);
        let enc_size: Option<i32> = row.get(2);
        return Ok(Some((ext, key, enc_size)));
    }

    // Try videos
    let row = client.query_opt(
        "SELECT ext, p2p_encryption_key, p2p_encrypted_size FROM videos WHERE hash = $1 LIMIT 1",
        &[&file_hash],
    ).await?;

    if let Some(row) = row {
        let ext: String = row.get(0);
        let key: Option<Vec<u8>> = row.get(1);
        let enc_size: Option<i32> = row.get(2);
        return Ok(Some((ext, key, enc_size)));
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

    let (shards, _enc_size) = StorageEngine::process_for_backup(&file_data, encryption_key)?;

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
    let conn = p2p_service.connect_to_addr(addr).await
        .map_err(|e| format!("Connection to {} failed: {}", addr, e))?;

    let mut shard_hash_bytes = [0u8; 32];
    let decoded = hex::decode(shard_hash_hex)?;
    shard_hash_bytes.copy_from_slice(&decoded);

    let (mut send, mut recv) = conn.open_bi().await?;
    Protocol::send(&mut send, &Message::RetrieveShardRequest { shard_hash: shard_hash_bytes }).await
        .map_err(|e| e.to_string())?;

    match Protocol::receive(&mut recv).await.map_err(|e| e.to_string())? {
        Message::RetrieveShardResponse { data: Some(data), .. } => {
            conn.close(0u32.into(), b"done");
            Ok(data)
        }
        _ => {
            conn.close(0u32.into(), b"done");
            Err("Shard not found on node".into())
        }
    }
}

/// Upload a shard to a remote node.
pub async fn upload_shard_to_node(
    p2p_service: &Arc<P2PService>,
    addr: SocketAddr,
    shard_data: &[u8],
) -> Result<String, Box<dyn std::error::Error + Send + Sync>> {
    let conn = p2p_service.connect_to_addr(addr).await
        .map_err(|e| format!("Connection to {} failed: {}", addr, e))?;

    let shard_hash = blake3::hash(shard_data);
    let shard_hash_hex = shard_hash.to_hex().to_string();

    let (mut send, mut recv) = conn.open_bi().await?;
    let req = Message::StoreShardRequest {
        shard_hash: shard_hash.into(),
        data: shard_data.to_vec(),
    };
    Protocol::send(&mut send, &req).await.map_err(|e| e.to_string())?;

    match Protocol::receive(&mut recv).await.map_err(|e| e.to_string())? {
        Message::StoreShardResponse { success: true, .. } => {
            conn.close(0u32.into(), b"done");
            Ok(shard_hash_hex)
        }
        _ => {
            conn.close(0u32.into(), b"done");
            Err(format!("Node {} rejected shard", addr).into())
        }
    }
}
