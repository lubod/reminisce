//! Shared P2P shard-upload helpers for backup-style workers.
//!
//! Centralizes the connect → StoreShard → retry → track flow that was previously
//! duplicated across `media_replication_worker` and `db_backup_worker` (and the
//! rendezvous node-selection shared with the audit / rebalance workers).

use np2p::network::{Message, P2PService, Protocol};
use np2p::storage::StorageEngine;
use std::net::SocketAddr;
use std::path::Path;
use std::sync::Arc;
use std::time::Duration;
use tokio::io::AsyncReadExt;
use tokio::sync::mpsc;
use log::warn;

/// Total shards produced per file (3 data + 2 parity).
pub const SHARD_COUNT: usize = 5;
/// Minimum storage nodes required before attempting replication.
pub const MIN_NODES_REQUIRED: usize = 1;
/// Files larger than this are uploaded via segmented streaming (caps peak RAM).
pub const SEGMENT_THRESHOLD: usize = 256 * 1024 * 1024; // 256 MB
/// Max bytes per StoreShardChunk protocol message. Kept safely under the protocol
/// receive cap (see np2p::network::protocol::Protocol::receive) so bincode framing
/// overhead never trips it — 16 MiB chunks leave generous headroom under 128 MiB.
pub const CHUNK_MSG_SIZE: usize = 16 * 1024 * 1024; // 16 MiB

/// One successfully uploaded shard.
#[derive(Clone, Debug)]
pub struct UploadedShard {
    pub idx: usize,
    pub node_id: String,
    pub addr: String,
    pub shard_hash_hex: String,
}

/// Rendezvous / highest-random-weight (HRW) node selection.
/// Ranks nodes by hash(id || node_id) and returns the top `count`. Uses the stable
/// hex node_id (public key) so assignment is consistent across restarts — adding a
/// node only displaces ~1/N of ids, minimizing rebalance work.
pub fn rendezvous_select(id_hash: &str, nodes: &[(String, SocketAddr)], count: usize) -> Vec<(String, SocketAddr)> {
    let mut scored: Vec<(u64, usize)> = nodes.iter().enumerate().map(|(i, (node_id, _))| {
        let h = blake3::hash(format!("{}:{}", id_hash, node_id).as_bytes());
        (u64::from_le_bytes(h.as_bytes()[0..8].try_into().unwrap()), i)
    }).collect();
    scored.sort_by_key(|&x| std::cmp::Reverse(x.0));
    scored.into_iter().take(count).map(|(_, i)| nodes[i].clone()).collect()
}

/// Store a single in-memory shard on a node with retries.
///
/// Streams the shard as bounded `StoreShardChunk` messages instead of one whole
/// `StoreShardRequest` — a single message would exceed the protocol size cap for
/// any shard larger than a few tens of MB (i.e. plaintext files above ~60 MB).
/// Returns the shard hash hex.
///
/// The stream token is bound to `blake3(file_hash || shard_index)` (file_hash is
/// the rendezvous id hash) so the storage node authenticates the stream exactly
/// like it does for the segmented upload path.
pub async fn store_shard(
    p2p_service: &Arc<P2PService>,
    addr: SocketAddr,
    file_hash_bytes: [u8; 32],
    shard_index: u8,
    shard_data: &[u8],
) -> Result<String, String> {
    let shard_hash_bytes: [u8; 32] = blake3::hash(shard_data).into();
    let shard_hash_hex = blake3::Hash::from(shard_hash_bytes).to_hex().to_string();
    let binding: [u8; 32] = blake3::hash(
        &[file_hash_bytes.as_slice(), &[shard_index]].concat()
    ).into();
    let mut last_err = String::new();
    let node_id = p2p_service.registry.find_by_addr(addr)
        .unwrap_or_else(|| addr.to_string());

    for attempt in 1..=3 {
        if attempt > 1 {
            tokio::time::sleep(Duration::from_millis(500 * (attempt - 1))).await;
        }
        let conn = match p2p_service.connect_to_addr(addr).await {
            Ok(c) => c,
            Err(e) => { last_err = format!("connect failed: {}", e); continue; }
        };
        let (mut send, mut recv) = match conn.open_bi().await {
            Ok(s) => s,
            Err(e) => { last_err = format!("open_bi failed: {}", e); conn.close(0u32.into(), b"error"); continue; }
        };

        let token = p2p_service.identity().create_shard_token(np2p::crypto::ShardOp::Store, &binding);
        let attempt_result: Result<bool, String> = async {
            Protocol::send(&mut send, &Message::StoreShardStreamInit {
                file_hash: file_hash_bytes,
                shard_index,
                total_shard_bytes: 0,
                segment_count: 0,
                token,
            }).await.map_err(|e| e.to_string())?;
            match Protocol::receive(&mut recv).await.map_err(|e| e.to_string())? {
                Message::StoreShardStreamAck { ready: true } => {}
                other => return Err(format!("unexpected stream ack: {:?}", other)),
            }
            let mut hasher = blake3::Hasher::new();
            for chunk in shard_data.chunks(CHUNK_MSG_SIZE) {
                hasher.update(chunk);
                Protocol::send(&mut send, &Message::StoreShardChunk { data: chunk.to_vec() })
                    .await.map_err(|e| e.to_string())?;
            }
            let shard_hash: [u8; 32] = hasher.finalize().into();
            Protocol::send(&mut send, &Message::StoreShardStreamFinal { shard_hash })
                .await.map_err(|e| e.to_string())?;
            match Protocol::receive(&mut recv).await.map_err(|e| e.to_string())? {
                Message::StoreShardStreamResponse { success, available_space_bytes } => {
                    crate::metrics::P2P_PEER_WRITE_LAST_STATUS
                        .with_label_values(&[&node_id]).set(if success { 1 } else { 0 });
                    crate::metrics::P2P_PEER_AVAILABLE_SPACE_BYTES
                        .with_label_values(&[&node_id]).set(available_space_bytes as f64);
                    if success {
                        crate::metrics::P2P_PEER_WRITE_SUCCESS_TOTAL
                            .with_label_values(&[&node_id]).inc();
                    } else {
                        crate::metrics::P2P_PEER_WRITE_FAILURES_TOTAL
                            .with_label_values(&[&node_id]).inc();
                    }
                    Ok(success)
                },
                other => Err(format!("unexpected response: {:?}", other)),
            }
        }.await;

        let _ = send.finish();
        match attempt_result {
            Ok(true) => { conn.close(0u32.into(), b"done"); return Ok(shard_hash_hex); }
            Ok(false) => last_err = "node rejected shard".to_string(),
            Err(e) => last_err = e,
        }
        conn.close(0u32.into(), b"error");
    }
    Err(last_err)
}

/// Upload already-in-memory shards to rendezvous-selected nodes in parallel.
/// `id_hash` drives node selection. Returns the shards that succeeded (may be fewer
/// than requested — the caller enforces the minimum-reconstruction threshold).
pub async fn upload_inmemory(
    p2p_service: &Arc<P2PService>,
    nodes: &[(String, SocketAddr)],
    id_hash: &str,
    shards: Vec<Vec<u8>>,
) -> Result<Vec<UploadedShard>, String> {
    let file_hash_bytes: [u8; 32] = blake3::hash(id_hash.as_bytes()).into();
    let target_nodes = rendezvous_select(id_hash, nodes, shards.len().min(nodes.len()).max(1));
    let mut set = tokio::task::JoinSet::new();

    for (idx, shard_data) in shards.into_iter().enumerate() {
        let (node_id, addr) = target_nodes[idx % target_nodes.len()].clone();
        let svc = p2p_service.clone();
        set.spawn(async move {
            store_shard(&svc, addr, file_hash_bytes, idx as u8, &shard_data).await.map(|shard_hash_hex| UploadedShard {
                idx,
                node_id,
                addr: addr.to_string(),
                shard_hash_hex,
            })
        });
    }

    let mut uploaded = Vec::new();
    while let Some(res) = set.join_next().await {
        match res {
            Ok(Ok(s)) => uploaded.push(s),
            Ok(Err(e)) => warn!("shard upload failed: {}", e),
            Err(e) => warn!("shard upload task panicked: {}", e),
        }
    }
    Ok(uploaded)
}

/// Upload a large file via segmented streaming: per-segment encrypt+shard
/// (`data_shards`+`parity_shards` RS), with one persistent QUIC stream per shard.
/// Peak RAM stays bounded regardless of file size. Returns the uploaded shards and
/// per-segment encrypted sizes.
pub async fn upload_segmented(
    p2p_service: &Arc<P2PService>,
    nodes: &[(String, SocketAddr)],
    id_hash: &str,
    file_path: &Path,
    encryption_key: &[u8; 32],
    data_shards: usize,
    parity_shards: usize,
) -> Result<(Vec<UploadedShard>, Vec<i64>), String> {
    let file_hash_bytes: [u8; 32] = blake3::hash(id_hash.as_bytes()).into();
    let total_shards = data_shards + parity_shards;
    let target_nodes = rendezvous_select(id_hash, nodes, total_shards.min(nodes.len()).max(1));

    // One bounded channel + task per shard (provides backpressure).
    let mut senders: Vec<mpsc::Sender<Vec<u8>>> = Vec::with_capacity(total_shards);
    let mut handles = Vec::with_capacity(total_shards);
    for idx in 0..total_shards {
        let (tx, rx) = mpsc::channel::<Vec<u8>>(2);
        senders.push(tx);
        let (node_id, addr) = target_nodes[idx % target_nodes.len()].clone();
        let svc = p2p_service.clone();
        handles.push(tokio::spawn(async move {
            stream_one_shard(&svc, addr, file_hash_bytes, idx, rx)
                .await
                .map(|shard_hash_hex| UploadedShard { idx, node_id, addr: addr.to_string(), shard_hash_hex })
        }));
    }

    // Read the file in segments; encrypt+shard each; fan sub-shards out to the streams.
    let mut file = tokio::fs::File::open(file_path).await.map_err(|e| e.to_string())?;
    let mut buf = vec![0u8; SEGMENT_THRESHOLD];
    let mut segment_enc_sizes: Vec<i64> = Vec::new();
    loop {
        let n = read_full(&mut file, &mut buf).await.map_err(|e| e.to_string())?;
        if n == 0 { break; }
        let seg_idx = segment_enc_sizes.len() as u32;
        let nonce_ctx: Vec<u8> = encryption_key.iter().chain(seg_idx.to_le_bytes().iter()).cloned().collect();
        let (sub_shards, enc_size) = StorageEngine::process_for_backup(&buf[..n], encryption_key, &nonce_ctx, data_shards, parity_shards)
            .map_err(|e| e.to_string())?;
        segment_enc_sizes.push(enc_size as i64);
        for (idx, sub_shard) in sub_shards.iter().enumerate() {
            for chunk in sub_shard.chunks(CHUNK_MSG_SIZE) {
                if senders[idx].send(chunk.to_vec()).await.is_err() { break; }
            }
        }
    }
    drop(senders); // signal EOF to all shard tasks

    let mut uploaded = Vec::new();
    for handle in handles {
        match handle.await {
            Ok(Ok(s)) => uploaded.push(s),
            Ok(Err(e)) => warn!("segmented shard failed: {}", e),
            Err(e) => warn!("segmented shard task panicked: {}", e),
        }
    }
    Ok((uploaded, segment_enc_sizes))
}

/// Maintain one streaming upload for a single shard index. Returns the shard hash hex.
async fn stream_one_shard(
    p2p_service: &Arc<P2PService>,
    addr: SocketAddr,
    file_hash_bytes: [u8; 32],
    idx: usize,
    mut rx: mpsc::Receiver<Vec<u8>>,
) -> Result<String, String> {
    let node_id = p2p_service.registry.find_by_addr(addr)
        .unwrap_or_else(|| addr.to_string());
    let conn = p2p_service.connect_to_addr(addr).await.map_err(|e| e.to_string())?;
    let (mut send, mut recv) = conn.open_bi().await.map_err(|e| e.to_string())?;

    // Token binds the stream to (file_hash, shard_index) — the storage node verifies
    // it before accepting any chunk, matching the non-streamed StoreShardRequest auth.
    let binding: [u8; 32] = blake3::hash(
        &[file_hash_bytes.as_slice(), &[idx as u8]].concat()
    ).into();
    let token = p2p_service.identity().create_shard_token(np2p::crypto::ShardOp::Store, &binding);

    let result: Result<String, String> = async {
        Protocol::send(&mut send, &Message::StoreShardStreamInit {
            file_hash: file_hash_bytes,
            shard_index: idx as u8,
            total_shard_bytes: 0,
            segment_count: 0,
            token,
        }).await.map_err(|e| e.to_string())?;
        match Protocol::receive(&mut recv).await.map_err(|e| e.to_string())? {
            Message::StoreShardStreamAck { ready: true } => {}
            other => return Err(format!("unexpected stream ack: {:?}", other)),
        }

        let mut hasher = blake3::Hasher::new();
        while let Some(chunk) = rx.recv().await {
            hasher.update(&chunk);
            Protocol::send(&mut send, &Message::StoreShardChunk { data: chunk }).await.map_err(|e| e.to_string())?;
        }
        let shard_hash: [u8; 32] = hasher.finalize().into();
        Protocol::send(&mut send, &Message::StoreShardStreamFinal { shard_hash }).await.map_err(|e| e.to_string())?;
        match Protocol::receive(&mut recv).await.map_err(|e| e.to_string())? {
            Message::StoreShardStreamResponse { success, available_space_bytes } => {
                crate::metrics::P2P_PEER_WRITE_LAST_STATUS
                    .with_label_values(&[&node_id]).set(if success { 1 } else { 0 });
                crate::metrics::P2P_PEER_AVAILABLE_SPACE_BYTES
                    .with_label_values(&[&node_id]).set(available_space_bytes as f64);
                if success {
                    crate::metrics::P2P_PEER_WRITE_SUCCESS_TOTAL
                        .with_label_values(&[&node_id]).inc();
                    Ok(blake3::Hash::from(shard_hash).to_hex().to_string())
                } else {
                    crate::metrics::P2P_PEER_WRITE_FAILURES_TOTAL
                        .with_label_values(&[&node_id]).inc();
                    Err("node rejected shard stream (success=false)".to_string())
                }
            },
            other => Err(format!("node rejected shard stream: {:?}", other)),
        }
    }.await;

    let _ = send.finish();
    conn.close(0u32.into(), if result.is_ok() { b"done" } else { b"error" });
    result
}

/// Read until `buf` is full or EOF. Returns bytes read.
pub async fn read_full(file: &mut tokio::fs::File, buf: &mut [u8]) -> std::io::Result<usize> {
    let mut total = 0;
    while total < buf.len() {
        match file.read(&mut buf[total..]).await? {
            0 => break,
            n => total += n,
        }
    }
    Ok(total)
}
