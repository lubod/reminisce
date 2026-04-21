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
use std::path::PathBuf;
use futures::stream::{self, StreamExt};
use std::time::Duration;
use crate::utils::{get_load_average, get_cpu_count, calculate_worker_concurrency};
use np2p::network::{P2PService, Message, Protocol};
use np2p::storage::StorageEngine;
use tokio::sync::{Mutex, mpsc};
use tokio::io::AsyncReadExt;

// Constants
const BATCH_SIZE: i64 = 10; // Smaller batches for sharding as it is more CPU intensive
pub const SHARD_COUNT: usize = 5;
pub const MIN_NODES_REQUIRED: usize = 1;

/// Files larger than this are processed in segments instead of all at once.
/// Keeps peak RAM at ~940 MB (256 MB segment + 256 MB encrypted + ~427 MB for 5 sub-shards).
const SEGMENT_THRESHOLD: usize = 256 * 1024 * 1024; // 256 MB

/// Max bytes per StoreShardChunk protocol message. Must stay under the 100 MB protocol limit.
const CHUNK_MSG_SIZE: usize = 32 * 1024 * 1024; // 32 MB

struct MediaToReplicate {
    hash: String,
    ext: String,
}

struct ShardResult {
    idx: usize,
    node_id: String,
    addr_str: String,
    shard_hash_hex: String,
}

/// Rendezvous hashing (HRW): rank nodes by hash(file_hash || node_id),
/// return top `count`. Uses the stable hex node_id (public key) for consistent
/// assignment across restarts — not the socket address which may change.
pub fn rendezvous_select_nodes(file_hash: &str, nodes: &[(String, SocketAddr)], count: usize) -> Vec<(String, SocketAddr)> {
    let mut scored: Vec<(u64, usize)> = nodes.iter().enumerate().map(|(i, (node_id, _))| {
        let hash = blake3::hash(format!("{}:{}", file_hash, node_id).as_bytes());
        let score = u64::from_le_bytes(hash.as_bytes()[0..8].try_into().unwrap());
        (score, i)
    }).collect();
    scored.sort_by(|a, b| b.0.cmp(&a.0));
    scored.into_iter().take(count).map(|(_, i)| nodes[i].clone()).collect()
}

pub async fn media_replication_loop(
    pool: Pool,
    config: Config,
    p2p_service: Arc<P2PService>,
) {
    info!("P2P Media Replication Worker started (3/5 EC, rendezvous hashing)");
    // Give LAN discovery time to register the Pi before the first batch.
    tokio::time::sleep(Duration::from_secs(20)).await;

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
    // Use dynamically discovered peers from the in-memory registry (LAN + coordinator)
    let nodes: Vec<(String, SocketAddr)> = p2p_service.registry.all()
        .into_iter()
        .map(|p| (p.node_id, p.addr))
        .collect();

    if nodes.is_empty() {
        return Ok(false);
    }

    if nodes.len() < MIN_NODES_REQUIRED {
        return Ok(false);
    }

    if nodes.len() < 3 {
        warn!("Only {} P2P nodes discovered. 3+ nodes recommended for 3/5 EC redundancy.", nodes.len());
    }

    let images_done = replicate_batch(pool, config, p2p_service, &nodes, "images").await
        .map_err(|e| format!("Failed to replicate image batch: {}", e))?;

    let videos_done = replicate_batch(pool, config, p2p_service, &nodes, "videos").await
        .map_err(|e| format!("Failed to replicate video batch: {}", e))?;

    Ok(images_done || videos_done)
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
    nodes: &[(String, SocketAddr)],
    file: &MediaToReplicate,
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

    // Route large files through the segmented streaming path to cap peak RAM at ~940 MB.
    if file_size > SEGMENT_THRESHOLD {
        return replicate_large_file(pool, p2p_service, table, nodes, file, &file_path).await;
    }

    // 1. Encrypt and Shard (entire file fits in memory for files ≤ 256 MB)
    let file_data = tokio::fs::read(&file_path).await?;
    let mut encryption_key = [0u8; 32];
    rand::fill(&mut encryption_key);

    // nonce_context = key: key is randomly generated once per file, ensuring unique nonce per file.
    let (shards, _enc_size) = StorageEngine::process_for_backup(&file_data, &encryption_key, &encryption_key)?;

    // 2. Select nodes via rendezvous hashing (HRW)
    // We always want to distribute among available nodes, but we MUST store ALL shards.
    let target_nodes = rendezvous_select_nodes(&file.hash, nodes, SHARD_COUNT.min(nodes.len()));

    info!("Sharding {} into {} pieces across {} nodes (rendezvous)", file.hash, shards.len(), target_nodes.len());

    // 3. Upload Shards in Parallel
    // Results: (shard_index, node_id, addr_str, shard_hash)
    let shard_results: Arc<Mutex<Vec<(usize, String, String, String)>>> = Arc::new(Mutex::new(Vec::new()));
    let mut set = tokio::task::JoinSet::new();

    for (idx, shard_data) in shards.into_iter().enumerate() {
        let (node_id, addr) = &target_nodes[idx % target_nodes.len()];
        let p2p_service = p2p_service.clone();
        let node_id = node_id.clone();
        let addr = *addr;
        let results = shard_results.clone();

        set.spawn(async move {
            let shard_hash = blake3::hash(&shard_data).to_hex().to_string();

            match p2p_service.connect_to_addr(addr).await {
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
                            r.push((idx, node_id, addr.to_string(), shard_hash));
                            Ok(())
                        } else {
                            Err("Node rejected shard".to_string())
                        }
                    } else {
                        Err("Unexpected response".to_string())
                    }
                }
                Err(e) => Err(format!("Connection to {} failed: {}", addr, e)),
            }
        });
    }

    while let Some(res) = set.join_next().await {
        if let Err(e) = res? {
            warn!("Shard upload failed: {}", e);
        }
    }

    let final_results = shard_results.lock().await;
    if final_results.len() < np2p::storage::DATA_SHARDS {
        return Err(format!("Only {}/{} shards stored. Minimum {} required (for reconstruction).", final_results.len(), SHARD_COUNT, np2p::storage::DATA_SHARDS).into());
    }

    // 4. Update Database
    let mut client = pool.get().await?;

    // Upsert nodes first in a separate statement (outside the per-file transaction)
    // so concurrent file transactions don't race on the same p2p_nodes rows.
    for (_, node_id, addr_str, _) in final_results.iter() {
        client.execute(
            "INSERT INTO p2p_nodes (node_id, public_addr, is_active)
             VALUES ($1, $2, TRUE)
             ON CONFLICT (node_id) DO UPDATE SET public_addr = $2, is_active = TRUE, last_seen = NOW()",
            &[node_id, addr_str],
        ).await?;
    }

    let trans = client.transaction().await?;

    for (idx, node_id, _addr_str, shard_hash) in final_results.iter() {
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
    // Compute a manifest hash: BLAKE3 over all stored shard hashes concatenated.
    let mut manifest_hasher = blake3::Hasher::new();
    for (_, _, _, shard_hash) in final_results.iter() { manifest_hasher.update(shard_hash.as_bytes()); }
    let manifest_hash = manifest_hasher.finalize().to_hex().to_string();
    let update_query = format!(
        "UPDATE {} SET p2p_synced_at = NOW(), p2p_shard_hash = $1, p2p_encryption_key = $2, p2p_encrypted_size = $3 WHERE hash = $4",
        table
    );
    trans.execute(&update_query, &[&manifest_hash, &key_bytes, &enc_size_i32, &file.hash]).await?;

    trans.commit().await?;

    info!("Replicated {}: {} shards stored (rendezvous)", file.hash, final_results.len());
    Ok(())
}

/// Replicates a file larger than SEGMENT_THRESHOLD by processing it in 256 MB segments.
/// Opens one persistent QUIC stream per shard, then streams sub-shard chunks across all
/// segments before finalising with a BLAKE3 hash. Peak RAM ≈ 940 MB regardless of file size.
async fn replicate_large_file(
    pool: &Pool,
    p2p_service: &Arc<P2PService>,
    table: &str,
    nodes: &[(String, SocketAddr)],
    file: &MediaToReplicate,
    file_path: &PathBuf,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let mut encryption_key = [0u8; 32];
    rand::fill(&mut encryption_key);

    // Stable [u8; 32] file identifier for StoreShardStreamInit (Pi uses it as temp-file key).
    let file_hash_bytes: [u8; 32] = blake3::hash(file.hash.as_bytes()).into();

    let target_nodes = rendezvous_select_nodes(&file.hash, nodes, SHARD_COUNT.min(nodes.len()));

    // One bounded channel per shard — provides backpressure so the main loop never buffers
    // more than 2 unsent chunks per shard (2 × 32 MB × 5 shards = 320 MB max channel overhead).
    let mut senders: Vec<mpsc::Sender<Vec<u8>>> = Vec::with_capacity(SHARD_COUNT);
    let mut handles = Vec::with_capacity(SHARD_COUNT);

    for idx in 0..SHARD_COUNT {
        let (tx, rx) = mpsc::channel::<Vec<u8>>(2);
        senders.push(tx);

        let (node_id, addr) = target_nodes[idx % target_nodes.len()].clone();
        let p2p_service = p2p_service.clone();

        handles.push(tokio::spawn(async move {
            let conn = p2p_service.connect_to_addr(addr).await
                .map_err(|e| e.to_string())?;
            let (mut send, mut recv) = conn.open_bi().await
                .map_err(|e| e.to_string())?;

            Protocol::send(&mut send, &Message::StoreShardStreamInit {
                file_hash: file_hash_bytes,
                shard_index: idx as u8,
                total_shard_bytes: 0, // not used by Pi handler
                segment_count: 0,     // not used by Pi handler
            }).await.map_err(|e| e.to_string())?;

            match Protocol::receive(&mut recv).await.map_err(|e| e.to_string())? {
                Message::StoreShardStreamAck { ready: true } => {}
                other => return Err(format!("Unexpected ack for shard {}: {:?}", idx, other)),
            }

            // Receive chunks from the main loop, hash and forward via QUIC.
            let mut hasher = blake3::Hasher::new();
            let mut rx = rx;
            while let Some(chunk) = rx.recv().await {
                hasher.update(&chunk);
                Protocol::send(&mut send, &Message::StoreShardChunk { data: chunk })
                    .await.map_err(|e| e.to_string())?;
            }

            // Channel closed — all segments sent. Finalise.
            let shard_b3 = hasher.finalize();
            let shard_hash: [u8; 32] = shard_b3.into();
            Protocol::send(&mut send, &Message::StoreShardStreamFinal { shard_hash })
                .await.map_err(|e| e.to_string())?;

            match Protocol::receive(&mut recv).await.map_err(|e| e.to_string())? {
                Message::StoreShardStreamResponse { success: true } => {}
                _ => return Err(format!("Pi rejected shard {}", idx)),
            }
            let _ = send.finish();

            Ok::<ShardResult, String>(ShardResult {
                idx,
                node_id,
                addr_str: addr.to_string(),
                shard_hash_hex: shard_b3.to_hex().to_string(),
            })
        }));
    }

    // Process file in SEGMENT_THRESHOLD-sized segments.
    let mut file_handle = tokio::fs::File::open(file_path).await?;
    let mut buf = vec![0u8; SEGMENT_THRESHOLD];
    let mut segment_enc_sizes: Vec<i64> = Vec::new();

    info!("Replicating large file {} using segmented streaming", file.hash);

    loop {
        let n = read_chunk(&mut file_handle, &mut buf).await?;
        if n == 0 { break; }

        // nonce_context includes segment index to prevent nonce reuse across segments.
        let seg_idx = segment_enc_sizes.len() as u32;
        let nonce_ctx: Vec<u8> = encryption_key.iter().chain(seg_idx.to_le_bytes().iter()).cloned().collect();
        let (sub_shards, enc_size) = StorageEngine::process_for_backup(&buf[..n], &encryption_key, &nonce_ctx)?;
        segment_enc_sizes.push(enc_size as i64);

        for (idx, sub_shard) in sub_shards.iter().enumerate() {
            for chunk in sub_shard.chunks(CHUNK_MSG_SIZE) {
                if senders[idx].send(chunk.to_vec()).await.is_err() {
                    break;
                }
            }
        }
        // sub_shards drops here, reclaiming ~427 MB before the next segment is read.
    }

    // Signal completion to all shard tasks by dropping the senders.
    drop(senders);

    let mut shard_results: Vec<ShardResult> = Vec::new();
    for handle in handles {
        match handle.await {
            Ok(Ok(r)) => shard_results.push(r),
            Ok(Err(e)) => warn!("Large-file shard task failed: {}", e),
            Err(e) => warn!("Large-file shard task panicked: {}", e),
        }
    }

    if shard_results.len() < np2p::storage::DATA_SHARDS {
        return Err(format!(
            "Only {}/{} shards stored for large file {}. Minimum {} required.",
            shard_results.len(), SHARD_COUNT, file.hash, np2p::storage::DATA_SHARDS
        ).into());
    }

    // Update database
    let mut client = pool.get().await?;
    for r in &shard_results {
        client.execute(
            "INSERT INTO p2p_nodes (node_id, public_addr, is_active) VALUES ($1, $2, TRUE)
             ON CONFLICT (node_id) DO UPDATE SET public_addr = $2, is_active = TRUE, last_seen = NOW()",
            &[&r.node_id, &r.addr_str],
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
    let key_bytes: &[u8] = &encryption_key;
    let update_query = format!(
        "UPDATE {} SET p2p_synced_at = NOW(), p2p_shard_hash = $1, p2p_encryption_key = $2, \
         p2p_encrypted_size = 0, p2p_segment_count = $3, p2p_segment_enc_sizes = $4 WHERE hash = $5",
        table
    );
    trans.execute(&update_query, &[&file.hash, &key_bytes, &segment_count, &segment_enc_sizes, &file.hash]).await?;
    trans.commit().await?;

    info!("Replicated {} ({} segments, {} shards stored)", file.hash, segment_count, shard_results.len());
    Ok(())
}

/// Fills `buf` from `file`, reading until the buffer is full or EOF. Returns bytes read.
async fn read_chunk(file: &mut tokio::fs::File, buf: &mut [u8]) -> std::io::Result<usize> {
    let mut total = 0;
    while total < buf.len() {
        match file.read(&mut buf[total..]).await? {
            0 => break,
            n => total += n,
        }
    }
    Ok(total)
}
