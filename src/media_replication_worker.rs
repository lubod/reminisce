use crate::config::Config;
use log::{info, warn, error};
use deadpool_postgres::Pool;
use std::sync::Arc;
use std::path::PathBuf;
use futures::stream::{self, StreamExt};
use std::time::Duration;
use crate::utils::{get_load_average, get_cpu_count, calculate_worker_concurrency, parse_peer_addr};
use np2p::network::{P2PService, Message, Protocol};
use np2p::storage::StorageEngine;
use tokio::sync::Mutex;

// Constants
const BATCH_SIZE: i64 = 10; // Smaller batches for sharding as it is more CPU intensive
pub const SHARD_COUNT: usize = 5;
pub const MIN_NODES_REQUIRED: usize = 3;

struct MediaToReplicate {
    hash: String,
    ext: String,
}

/// Rendezvous hashing (HRW): for a given file, rank all nodes by
/// hash(file_hash || node_addr) and return the top `count` nodes.
/// Shard i is assigned to the i-th ranked node, guaranteeing each shard
/// lands on a different node (when count <= nodes.len()).
pub fn rendezvous_select_nodes(file_hash: &str, nodes: &[String], count: usize) -> Vec<String> {
    let mut scored: Vec<(u64, &String)> = nodes.iter().map(|node| {
        let hash = blake3::hash(format!("{}:{}", file_hash, node).as_bytes());
        let score = u64::from_le_bytes(hash.as_bytes()[0..8].try_into().unwrap());
        (score, node)
    }).collect();
    scored.sort_by(|a, b| b.0.cmp(&a.0));
    scored.into_iter().take(count).map(|(_, node)| node.clone()).collect()
}

pub async fn media_replication_loop(
    pool: Pool,
    config: Config,
    p2p_service: Arc<P2PService>,
) {
    info!("P2P Media Replication Worker started (3/5 EC, rendezvous hashing)");

    crate::utils::run_worker_loop(
        "Media Replication Worker",
        Duration::from_secs(10),
        Duration::from_secs(60),
        || replicate_all(&pool, &config, &p2p_service)
    ).await;
}

async fn replicate_all(
    pool: &Pool,
    config: &Config,
    p2p_service: &Arc<P2PService>,
) -> Result<bool, String> {
    // Exclusively use static peers from config
    let nodes = config.p2p_peers.clone();

    if nodes.is_empty() {
        return Ok(false);
    }

    if nodes.len() < MIN_NODES_REQUIRED {
        warn!("Only {} P2P nodes defined in config. Minimum {} required for 3/5 EC replication.", nodes.len(), MIN_NODES_REQUIRED);
        return Ok(false);
    }

    // 2. Process images
    let images_done = replicate_batch(pool, config, p2p_service, &nodes, "images").await
        .map_err(|e| format!("Failed to replicate image batch: {}", e))?;

    // 3. Process videos
    let videos_done = replicate_batch(pool, config, p2p_service, &nodes, "videos").await
        .map_err(|e| format!("Failed to replicate video batch: {}", e))?;

    Ok(images_done || videos_done)
}

async fn replicate_batch(
    pool: &Pool,
    config: &Config,
    p2p_service: &Arc<P2PService>,
    nodes: &[String],
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
    let rows = client.query(&query, &[&BATCH_SIZE]).await?;

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

    let successes = std::sync::atomic::AtomicUsize::new(0);

    stream::iter(files)
        .for_each_concurrent(limits.verification, |file| {
            let pool_clone = pool.clone();
            let p2p_service_clone = p2p_service.clone();
            let base_dir_owned = base_dir.to_string();
            let table_owned = table.to_string();
            let nodes_owned = nodes.to_vec();
            let success_counter = &successes;

            async move {
                match replicate_single_file(
                    &pool_clone,
                    &p2p_service_clone,
                    &base_dir_owned,
                    &table_owned,
                    &nodes_owned,
                    &file,
                ).await {
                    Ok(_) => { success_counter.fetch_add(1, std::sync::atomic::Ordering::Relaxed); },
                    Err(e) => error!("Failed to replicate {}: {}", file.hash, e),
                }
            }
        })
        .await;

    Ok(successes.load(std::sync::atomic::Ordering::Relaxed) > 0)
}

async fn replicate_single_file(
    pool: &Pool,
    p2p_service: &Arc<P2PService>,
    base_dir: &str,
    table: &str,
    nodes: &[String],
    file: &MediaToReplicate,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let file_path = PathBuf::from(base_dir)
        .join(&file.hash[0..2])
        .join(format!("{}.{}", file.hash, file.ext));

    if !file_path.exists() {
        let client = pool.get().await?;
        let update_query = format!("UPDATE {} SET p2p_synced_at = NOW() WHERE hash = $1", table);
        client.execute(&update_query, &[&file.hash]).await?;
        return Ok(());
    }

    // 1. Encrypt and Shard
    let file_data = tokio::fs::read(&file_path).await?;
    let mut encryption_key = [0u8; 32];
    rand::fill(&mut encryption_key);

    let (shards, _enc_size) = StorageEngine::process_for_backup(&file_data, &encryption_key)?;

    // 2. Select nodes via rendezvous hashing (HRW)
    let target_nodes = rendezvous_select_nodes(&file.hash, nodes, SHARD_COUNT.min(nodes.len()));

    info!("Sharding {} into {} pieces across {} nodes (rendezvous)", file.hash, shards.len(), target_nodes.len());

    // 3. Upload Shards in Parallel
    let shard_results = Arc::new(Mutex::new(Vec::new()));
    let mut set = tokio::task::JoinSet::new();

    for (idx, target_peer) in target_nodes.iter().enumerate() {
        let shard_data = shards[idx % shards.len()].clone();
        let p2p_service = p2p_service.clone();
        let target_peer = target_peer.clone();

        let results = shard_results.clone();

        set.spawn(async move {
            let shard_hash = blake3::hash(&shard_data).to_hex().to_string();

            let addr = parse_peer_addr(&target_peer)?;
            let connection = p2p_service.connect_to_addr(addr).await;

            match connection {
                Ok(conn) => {
                    let (mut send, mut recv) = conn.open_bi().await.map_err(|e| e.to_string())?;
                    let req = Message::StoreShardRequest {
                        shard_hash: blake3::hash(&shard_data).into(),
                        data: shard_data,
                    };
                    Protocol::send(&mut send, &req).await.map_err(|e| e.to_string())?;
                    let resp = Protocol::receive(&mut recv).await.map_err(|e| e.to_string())?;

                    if let Message::StoreShardResponse { success, .. } = resp {
                        if success {
                            let mut r = results.lock().await;
                            r.push((idx, target_peer, shard_hash));
                            Ok(())
                        } else {
                            Err("Node rejected shard".to_string())
                        }
                    } else {
                        Err("Unexpected response".to_string())
                    }
                }
                Err(e) => Err(format!("Connection to {} failed: {}", target_peer, e)),
            }
        });
    }

    while let Some(res) = set.join_next().await {
        if let Err(e) = res? {
            warn!("Shard upload failed: {}", e);
        }
    }

    let final_results = shard_results.lock().await;
    if final_results.len() < MIN_NODES_REQUIRED {
        return Err(format!("Only {}/{} shards stored. Minimum {} required.", final_results.len(), SHARD_COUNT, MIN_NODES_REQUIRED).into());
    }

    // 4. Update Database
    let mut client = pool.get().await?;
    let trans = client.transaction().await?;

    for (idx, node_id, shard_hash) in final_results.iter() {
        trans.execute(
            "INSERT INTO p2p_shards (file_hash, shard_index, node_id, shard_hash)
             VALUES ($1, $2, $3, $4)
             ON CONFLICT (file_hash, shard_index) DO UPDATE SET node_id = $3, shard_hash = $4",
            &[&file.hash, &(*idx as i32), node_id, shard_hash]
        ).await?;
    }

    // Mark as synced and store the encryption key + encrypted size for future re-sharding
    let enc_size_i32 = _enc_size as i32;
    let key_bytes: &[u8] = &encryption_key;
    let update_query = format!(
        "UPDATE {} SET p2p_synced_at = NOW(), p2p_shard_hash = $1, p2p_encryption_key = $2, p2p_encrypted_size = $3 WHERE hash = $4",
        table
    );
    trans.execute(&update_query, &[&file.hash, &key_bytes, &enc_size_i32, &file.hash]).await?;

    trans.commit().await?;

    info!("Replicated {}: {} shards stored (rendezvous)", file.hash, final_results.len());
    Ok(())
}
