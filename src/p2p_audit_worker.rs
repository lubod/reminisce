use crate::config::Config;
use crate::media_replication_worker::{rendezvous_select_nodes, SHARD_COUNT, MIN_NODES_REQUIRED};
use crate::shard_rebalance_worker::{find_file_info, upload_shard_to_node, lookup_node_addr};
use log::{info, warn, error};
use deadpool_postgres::Pool;
use np2p::network::{P2PService, Message, Protocol};
use np2p::storage::StorageEngine;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

pub async fn start_audit_worker(
    pool: Pool,
    config: Config,
    p2p_service: Arc<P2PService>,
) {
    info!("P2P Audit & Repair Worker started");

    crate::utils::run_worker_loop(
        "P2P Audit Worker",
        Duration::from_secs(60),
        Duration::from_secs(3600),
        || {
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

        let addr = match lookup_node_addr(&pool, p2p_service, &node_id).await {
            Some(a) => a,
            None => {
                warn!("Cannot audit shard: unknown addr for node {}", node_id);
                continue;
            }
        };
        let connection = p2p_service.connect_to_addr(addr).await;

        let mut success = false;
        match connection {
            Ok(conn) => {
                let decoded = match hex::decode(&expected_shard_hash) {
                    Ok(d) if d.len() == 32 => d,
                    Ok(_) => {
                        warn!("Shard hash {} has wrong length, skipping", expected_shard_hash);
                        conn.close(0u32.into(), b"invalid hash");
                        continue;
                    }
                    Err(e) => {
                        warn!("Invalid shard hash hex {}: {}", expected_shard_hash, e);
                        conn.close(0u32.into(), b"invalid hash");
                        continue;
                    }
                };
                let mut shard_hash_bytes = [0u8; 32];
                shard_hash_bytes.copy_from_slice(&decoded);

                match conn.open_bi().await {
                    Ok((mut send, mut recv)) => {
                        let req = Message::RetrieveShardRequest { shard_hash: shard_hash_bytes };
                        if Protocol::send(&mut send, &req).await.is_ok() {
                            if let Ok(Message::RetrieveShardResponse { data: Some(data), .. }) = Protocol::receive(&mut recv).await {
                                let actual_hash = blake3::hash(&data).to_hex().to_string();
                                if actual_hash == expected_shard_hash {
                                    success = true;
                                } else {
                                    warn!("Shard {} index {} on node {} is CORRUPTED!", file_hash, shard_index, node_id);
                                }
                            } else {
                                warn!("Shard {} index {} on node {} is MISSING!", file_hash, shard_index, node_id);
                            }
                        }
                    }
                    Err(e) => {
                        warn!("Failed to open stream to node {} for shard {}: {}", node_id, expected_shard_hash, e);
                    }
                }
                conn.close(0u32.into(), b"done");
            }
            Err(e) => {
                warn!("Failed to reach node {} for audit: {}", node_id, e);
            }
        }

        if success {
            let _ = client.execute(
                "UPDATE p2p_shards SET last_checked_at = NOW() WHERE id = $1",
                &[&shard_db_id]
            ).await;
        } else {
            info!("Triggering repair for file {} (shard {} lost)", file_hash, shard_index);
            if let Err(e) = repair_file(pool, config, p2p_service, &file_hash, shard_index as usize).await {
                error!("Repair failed for {}: {}", file_hash, e);
            }
        }
    }

    Ok(true)
}

async fn check_consistency(
    pool: &Pool,
    config: &Config,
    p2p_service: &Arc<P2PService>,
) -> Result<bool, String> {
    let client = pool.get().await.map_err(|e| e.to_string())?;
    
    // Find images or videos that are marked as synced but have no/few shards in p2p_shards
    let query = "
        WITH synced_files AS (
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
        WHERE c.count IS NULL OR c.count < 3
        LIMIT 10";

    let rows = client.query(query, &[]).await.map_err(|e| e.to_string())?;
    
    if rows.is_empty() {
        return Ok(false);
    }

    info!("Consistency check: Found {} files with missing/incomplete shards", rows.len());

    for row in rows {
        let file_hash: String = row.get(0);
        info!("Consistency check: Fixing missing shards for file {}", file_hash);
        
        for i in 0..SHARD_COUNT {
            if let Err(e) = repair_file(pool, config, p2p_service, &file_hash, i).await {
                error!("Consistency check: Failed to fix shard {} for {}: {}", i, file_hash, e);
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

    // Determine the correct target node for this shard using live registry peers
    let active_nodes: Vec<(String, std::net::SocketAddr)> = p2p_service.registry.all()
        .into_iter().map(|p| (p.node_id, p.addr)).collect();
    if active_nodes.len() < MIN_NODES_REQUIRED {
        return Err("Not enough active nodes for repair".into());
    }

    let ideal_nodes = rendezvous_select_nodes(file_hash, &active_nodes, SHARD_COUNT.min(active_nodes.len()));
    if failed_shard_index >= ideal_nodes.len() {
        return Err(format!("Shard index {} exceeds available nodes", failed_shard_index).into());
    }
    let (target_node_id, target_node_addr) = &ideal_nodes[failed_shard_index];

    // Check if this specific shard record already exists and is healthy
    let existing_shard = client.query_opt(
        "SELECT id FROM p2p_shards WHERE file_hash = $1 AND shard_index = $2 AND node_id = $3",
        &[&file_hash, &(failed_shard_index as i32), target_node_id]
    ).await?;

    if existing_shard.is_some() {
        return Ok(());
    }

    // Try re-sharding from local file if encryption key is stored
    let file_info = find_file_info(&client, file_hash).await?;

    match file_info {
        Some((ext, Some(key), _enc_size)) => {
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

            let (shards, _) = StorageEngine::process_for_backup(&file_data, &key)?;

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

                if let Ok(conn) = p2p_service.connect_to_addr(*node_addr).await {
                    let mut shard_hash_bytes = [0u8; 32];
                    if let Ok(decoded) = hex::decode(&expected_shard_hash) {
                        shard_hash_bytes.copy_from_slice(&decoded);
                    } else {
                        continue;
                    }

                    if let Ok((mut send, mut recv)) = conn.open_bi().await {
                        let req = Message::RetrieveShardRequest { shard_hash: shard_hash_bytes };
                        if Protocol::send(&mut send, &req).await.is_ok() {
                            if let Ok(Message::RetrieveShardResponse { data: Some(data), .. }) = Protocol::receive(&mut recv).await {
                                let actual_hash = blake3::hash(&data).to_hex().to_string();
                                if actual_hash == expected_shard_hash {
                                    conn.close(0u32.into(), b"done");

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
                    }
                    conn.close(0u32.into(), b"done");
                }
            }

            Err(format!("Unrecoverable: shard {} of {} not found on any node and no encryption key stored", failed_shard_index, file_hash).into())
        }
    }
}
