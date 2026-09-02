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
/// Recently completed remote shard stores, used by the audit sweep's orphan GC
/// as a grace period: a shard uploaded moments ago may not be committed to
/// `p2p_shards` yet (upload ACK precedes the DB transaction), and the sweep
/// would otherwise delete it as an "orphan". Bounded by prune-on-insert.
pub fn note_recent_store(node_id: &str, shard_hash_hex: &str) {
    use std::collections::HashMap;
    use std::sync::Mutex;
    use std::time::{Duration, Instant};
    static RECENT: std::sync::OnceLock<Mutex<HashMap<(String, String), Instant>>> = std::sync::OnceLock::new();
    const GRACE: Duration = Duration::from_secs(15 * 60);
    let map = RECENT.get_or_init(|| Mutex::new(HashMap::new()));
    let mut g = map.lock().unwrap_or_else(|e| e.into_inner());
    let now = Instant::now();
    g.retain(|_, t| now.duration_since(*t) < GRACE);
    if g.len() > 50_000 {
        return; // bound memory under pathological load; sweep just runs conservative
    }
    g.insert((node_id.to_string(), shard_hash_hex.to_string()), now);
}

/// True when (node, hash) was stored recently enough that the audit sweep must
/// not treat it as an orphan.
pub fn is_recently_stored(node_id: &str, shard_hash_hex: &str) -> bool {
    use std::collections::HashMap;
    use std::sync::Mutex;
    use std::time::{Duration, Instant};
    static RECENT: std::sync::OnceLock<Mutex<HashMap<(String, String), Instant>>> = std::sync::OnceLock::new();
    const GRACE: Duration = Duration::from_secs(15 * 60);
    let map = RECENT.get_or_init(|| Mutex::new(HashMap::new()));
    let g = map.lock().unwrap_or_else(|e| e.into_inner());
    match g.get(&(node_id.to_string(), shard_hash_hex.to_string())) {
        Some(t) => t.elapsed() < GRACE,
        None => false,
    }
}

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
                    crate::metrics::record_peer_write(&node_id, success, Some(available_space_bytes));
                    Ok(success)
                },
                other => Err(format!("unexpected response: {:?}", other)),
            }
        }.await;

        let _ = send.finish();
        match attempt_result {
            Ok(true) => {
                conn.close(0u32.into(), b"done");
                // Record for the audit sweep grace period (node_id falls back to
                // addr string when unresolved — sweep compares the same way).
                let owner = p2p_service.registry.find_by_addr(addr)
                    .unwrap_or_else(|| addr.to_string());
                note_recent_store(&owner, &shard_hash_hex);
                return Ok(shard_hash_hex);
            }
            Ok(false) => last_err = "node rejected shard".to_string(),
            Err(e) => last_err = e,
        }
        conn.close(0u32.into(), b"error");
    }
    Err(last_err)
}

/// Retrieve a shard from a specific node by its 64-hex hash.
///
/// Shared by the audit, rebalance and restore paths: identity-authenticated
/// RetrieveShardRequest, verifying the token is bound to the shard hash, then
/// returning the raw bytes. Errors if the shard is missing or the token/query
/// fails.
pub async fn retrieve_shard(
    p2p_service: &Arc<P2PService>,
    addr: SocketAddr,
    shard_hash_hex: &str,
) -> Result<Vec<u8>, String> {
    let shard_hash_bytes: [u8; 32] = hex::decode(shard_hash_hex)
        .map_err(|e| crate::p2p_error::P2pError::InvalidShardHash { hash: shard_hash_hex.to_string(), message: e.to_string() })?
        .try_into()
        .map_err(|_| crate::p2p_error::P2pError::InvalidShardHash { hash: shard_hash_hex.to_string(), message: "wrong byte length".into() })?;

    let conn = p2p_service
        .connect_to_addr(addr)
        .await
        .map_err(|e| crate::p2p_error::P2pError::connect(addr.to_string(), e.to_string()))?;

    let token = p2p_service
        .identity()
        .create_shard_token(np2p::crypto::ShardOp::Retrieve, &shard_hash_bytes);

    // Attempt 1: Streaming retrieval (chunked, bypasses 128 MB frame limit for large shards)
    let stream_attempt = async {
        let (mut send, mut recv) = conn
            .open_bi()
            .await
            .map_err(|e| crate::p2p_error::P2pError::open_bi(addr.to_string(), e.to_string()))?;

        Protocol::send(&mut send, &Message::RetrieveShardStreamInit {
            shard_hash: shard_hash_bytes,
            token: token.clone(),
        })
        .await
        .map_err(|e| crate::p2p_error::P2pError::Send { message: e.to_string() })?;

        match Protocol::receive(&mut recv).await {
            Ok(Message::RetrieveShardStreamAck { found: true, total_bytes }) => {
                let mut data = Vec::with_capacity(total_bytes as usize);
                let mut hasher = blake3::Hasher::new();
                loop {
                    match Protocol::receive(&mut recv).await {
                        Ok(Message::RetrieveShardChunk { data: chunk }) => {
                            hasher.update(&chunk);
                            data.extend_from_slice(&chunk);
                        }
                        Ok(Message::RetrieveShardStreamFinal { shard_hash: expected_hash }) => {
                            let actual_hash: [u8; 32] = hasher.finalize().into();
                            let _ = send.finish();
                            if actual_hash == expected_hash && actual_hash == shard_hash_bytes {
                                conn.close(0u32.into(), b"done");
                                return Ok(Some(data));
                            } else {
                                conn.close(1u32.into(), b"hash_mismatch");
                                return Err(crate::p2p_error::P2pError::Receive { message: "Hash mismatch in stream".to_string() });
                            }
                        }
                        Ok(other) => {
                            let _ = send.finish();
                            return Err(crate::p2p_error::P2pError::Receive { message: format!("Unexpected stream message: {:?}", other) });
                        }
                        Err(e) => {
                            let _ = send.finish();
                            return Err(crate::p2p_error::P2pError::Receive { message: e.to_string() });
                        }
                    }
                }
            }
            Ok(Message::RetrieveShardStreamAck { found: false, .. }) => {
                let _ = send.finish();
                conn.close(0u32.into(), b"not_found");
                Err(crate::p2p_error::P2pError::ShardNotFound)
            }
            _ => {
                let _ = send.finish();
                // Peer may not support streaming (older node) -> fallback
                Ok(None)
            }
        }
    }.await;

    match stream_attempt {
        Ok(Some(data)) => return Ok(data),
        Err(crate::p2p_error::P2pError::ShardNotFound) => {
            return Err(crate::p2p_error::P2pError::ShardNotFound.to_string());
        }
        _ => {} // Fall through to legacy attempt
    }

    // Attempt 2: Legacy single-frame RetrieveShardRequest
    let (mut send2, mut recv2) = conn
        .open_bi()
        .await
        .map_err(|e| crate::p2p_error::P2pError::open_bi(addr.to_string(), e.to_string()))?;

    Protocol::send(&mut send2, &Message::RetrieveShardRequest {
        shard_hash: shard_hash_bytes,
        token,
    })
    .await
    .map_err(|e| crate::p2p_error::P2pError::Send { message: e.to_string() })?;

    let result = match Protocol::receive(&mut recv2).await {
        Ok(Message::RetrieveShardResponse { data: Some(data), .. }) => Ok(data),
        Ok(_) => Err(crate::p2p_error::P2pError::ShardNotFound),
        Err(e) => Err(crate::p2p_error::P2pError::Receive { message: e.to_string() }),
    };
    let _ = send2.finish();
    conn.close(0u32.into(), if result.is_ok() { b"done" } else { b"error" });
    result.map_err(|e| e.to_string())
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
                crate::metrics::record_peer_write(&node_id, success, Some(available_space_bytes));
                if success {
                    Ok(blake3::Hash::from(shard_hash).to_hex().to_string())
                } else {
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

/// Send a DeleteShardRequest to a remote node holding a shard (best-effort).
pub async fn delete_shard_remote(
    p2p_service: &Arc<P2PService>,
    addr: SocketAddr,
    _node_id: &str,
    shard_hash_hex: &str,
) -> Result<bool, String> {
    let shard_hash_bytes: [u8; 32] = match hex::decode(shard_hash_hex).ok().and_then(|b| b.try_into().ok()) {
        Some(h) => h,
        None => return Err(format!("invalid shard hash hex {}", shard_hash_hex)),
    };

    let conn = p2p_service.connect_to_addr(addr).await.map_err(|e| e.to_string())?;
    let (mut send, mut recv) = conn.open_bi().await.map_err(|e| e.to_string())?;
    let token = p2p_service.identity().create_shard_token(np2p::crypto::ShardOp::Delete, &shard_hash_bytes);
    Protocol::send(&mut send, &Message::DeleteShardRequest { shard_hash: shard_hash_bytes, token }).await.map_err(|e| e.to_string())?;
    let msg = Protocol::receive(&mut recv).await.map_err(|e| e.to_string())?;
    let _ = send.finish();
    conn.close(0u32.into(), b"done");
    match msg {
        Message::DeleteShardResponse { success, .. } => Ok(success),
        other => Err(format!("unexpected delete response: {:?}", other)),
    }
}

/// Request the list of stored shard hashes from a remote node, optionally scoped by prefix.
pub async fn list_remote_shards(
    p2p_service: &Arc<P2PService>,
    addr: SocketAddr,
    prefix: Option<String>,
) -> Result<(Vec<[u8; 32]>, u64), String> {
    let conn = p2p_service.connect_to_addr(addr).await.map_err(|e| e.to_string())?;
    let (mut send, mut recv) = conn.open_bi().await.map_err(|e| e.to_string())?;
    let scope_bytes: [u8; 32] = blake3::hash(prefix.as_deref().unwrap_or("").as_bytes()).into();
    let token = p2p_service.identity().create_shard_token(np2p::crypto::ShardOp::List, &scope_bytes);
    Protocol::send(&mut send, &Message::ListShardsRequest { prefix, token }).await.map_err(|e| e.to_string())?;
    let msg = Protocol::receive(&mut recv).await.map_err(|e| e.to_string())?;
    let _ = send.finish();
    conn.close(0u32.into(), b"done");
    match msg {
        Message::ListShardsResponse { shards, available_space_bytes, .. } => Ok((shards, available_space_bytes)),
        Message::Error { message, .. } => Err(message),
        other => Err(format!("unexpected list shards response: {:?}", other)),
    }
}
