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
static RECENT_STORES: std::sync::OnceLock<std::sync::Mutex<std::collections::HashMap<(String, String), std::time::Instant>>> = std::sync::OnceLock::new();
const RECENT_STORE_GRACE: Duration = Duration::from_secs(15 * 60);

fn recent_stores_map() -> &'static std::sync::Mutex<std::collections::HashMap<(String, String), std::time::Instant>> {
    RECENT_STORES.get_or_init(|| std::sync::Mutex::new(std::collections::HashMap::new()))
}

/// Recently completed remote shard stores, used by the audit sweep's orphan GC
/// as a grace period: a shard uploaded moments ago may not be committed to
/// `p2p_shards` yet (upload ACK precedes the DB transaction), and the sweep
/// would otherwise delete it as an "orphan". Bounded by prune-on-insert.
pub fn note_recent_store(node_id: &str, shard_hash_hex: &str) {
    let mut g = recent_stores_map().lock().unwrap_or_else(|e| e.into_inner());
    let now = std::time::Instant::now();
    g.retain(|_, t| now.duration_since(*t) < RECENT_STORE_GRACE);
    if g.len() > 50_000 {
        return; // bound memory under pathological load; sweep just runs conservative
    }
    g.insert((node_id.to_string(), shard_hash_hex.to_string()), now);
}

/// True when (node, hash) was stored recently enough that the audit sweep must
/// not treat it as an orphan.
pub fn is_recently_stored(node_id: &str, shard_hash_hex: &str) -> bool {
    let g = recent_stores_map().lock().unwrap_or_else(|e| e.into_inner());
    match g.get(&(node_id.to_string(), shard_hash_hex.to_string())) {
        Some(t) => t.elapsed() < RECENT_STORE_GRACE,
        None => false,
    }
}

/// Opens a bi-directional stream, sends a signed `StoreShardStreamInit`, and waits for ready Ack.
async fn begin_shard_stream(
    conn: &np2p::quinn::Connection,
    identity: &np2p::crypto::NodeIdentity,
    file_hash_bytes: [u8; 32],
    shard_index: u8,
    total_shard_bytes: u64,
    segment_count: u32,
) -> Result<(np2p::quinn::SendStream, np2p::quinn::RecvStream), String> {
    let (mut send, mut recv) = conn.open_bi().await.map_err(|e| format!("open_bi failed: {}", e))?;
    let binding: [u8; 32] = blake3::hash(&[file_hash_bytes.as_slice(), &[shard_index]].concat()).into();
    let token = identity.create_shard_token(np2p::crypto::ShardOp::Store, &binding);
    Protocol::send(&mut send, &Message::StoreShardStreamInit {
        file_hash: file_hash_bytes,
        shard_index,
        total_shard_bytes,
        segment_count,
        token,
    }).await.map_err(|e| e.to_string())?;
    match Protocol::receive(&mut recv).await.map_err(|e| e.to_string())? {
        Message::StoreShardStreamAck { ready: true } => Ok((send, recv)),
        other => Err(format!("unexpected stream ack: {:?}", other)),
    }
}

/// Finalizes a chunked shard stream, records peer write health, and returns node acceptance.
async fn finish_shard_stream(
    send: &mut np2p::quinn::SendStream,
    recv: &mut np2p::quinn::RecvStream,
    node_id: &str,
    shard_hash: [u8; 32],
) -> Result<bool, String> {
    Protocol::send(send, &Message::StoreShardStreamFinal { shard_hash })
        .await.map_err(|e| e.to_string())?;
    match Protocol::receive(recv).await.map_err(|e| e.to_string())? {
        Message::StoreShardStreamResponse { success, available_space_bytes } => {
            crate::metrics::record_peer_write(node_id, success, Some(available_space_bytes));
            Ok(success)
        }
        other => Err(format!("unexpected response: {:?}", other)),
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
    let declared_bytes = if (shard_data.len() as u64) <= (np2p::network::protocol::MAX_MESSAGE_LEN as u64 - 1024) {
        shard_data.len() as u64
    } else {
        0
    };
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

        let attempt_result: Result<bool, String> = async {
            let (mut send, mut recv) = begin_shard_stream(
                &conn,
                &p2p_service.identity(),
                file_hash_bytes,
                shard_index,
                declared_bytes,
                0,
            ).await?;

            let mut hasher = blake3::Hasher::new();
            for chunk in shard_data.chunks(CHUNK_MSG_SIZE) {
                hasher.update(chunk);
                Protocol::send(&mut send, &Message::StoreShardChunk { data: chunk.to_vec() })
                    .await.map_err(|e| e.to_string())?;
            }
            let shard_hash: [u8; 32] = hasher.finalize().into();
            let res = finish_shard_stream(&mut send, &mut recv, &node_id, shard_hash).await;
            let _ = send.finish();
            res
        }.await;

        match attempt_result {
            Ok(true) => {
                conn.close(0u32.into(), b"done");
                note_recent_store(&node_id, &shard_hash_hex);
                return Ok(shard_hash_hex);
            }
            Ok(false) => last_err = "node rejected shard".to_string(),
            Err(e) => last_err = e,
        }
        conn.close(0u32.into(), b"error");
    }
    Err(last_err)
}

enum ShardSink<'a> {
    Vec(&'a mut Vec<u8>),
    File(&'a mut tokio::fs::File),
    HashOnly,
}

impl ShardSink<'_> {
    fn reserve_hint(&mut self, bytes: u64) {
        if let ShardSink::Vec(v) = self {
            v.reserve(bytes.min(256 * 1024 * 1024) as usize);
        }
    }

    async fn write_chunk(&mut self, chunk: &[u8]) -> Result<(), String> {
        match self {
            ShardSink::Vec(v) => v.extend_from_slice(chunk),
            ShardSink::File(f) => {
                use tokio::io::AsyncWriteExt;
                f.write_all(chunk).await.map_err(|e| format!("Write shard chunk failed: {}", e))?;
            }
            ShardSink::HashOnly => {}
        }
        Ok(())
    }
}

async fn retrieve_shard_into(
    p2p_service: &Arc<P2PService>,
    addr: SocketAddr,
    shard_hash_hex: &str,
    mut sink: ShardSink<'_>,
) -> Result<(), String> {
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

    // Attempt 1: Streaming retrieval (chunked, bypasses 128 MB frame limit and bounds RAM).
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
                sink.reserve_hint(total_bytes);
                let mut hasher = blake3::Hasher::new();
                loop {
                    match Protocol::receive(&mut recv).await {
                        Ok(Message::RetrieveShardChunk { data: chunk }) => {
                            hasher.update(&chunk);
                            sink.write_chunk(&chunk).await
                                .map_err(|e| crate::p2p_error::P2pError::Receive { message: e })?;
                        }
                        Ok(Message::RetrieveShardStreamFinal { shard_hash: expected_hash }) => {
                            let actual_hash: [u8; 32] = hasher.finalize().into();
                            let _ = send.finish();
                            if actual_hash == expected_hash && actual_hash == shard_hash_bytes {
                                conn.close(0u32.into(), b"done");
                                return Ok(true);
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
                Ok(false)
            }
        }
    }.await;

    match stream_attempt {
        Ok(true) => return Ok(()),
        Err(crate::p2p_error::P2pError::ShardNotFound) => {
            return Err(crate::p2p_error::P2pError::ShardNotFound.to_string());
        }
        Err(e) if matches!(e, crate::p2p_error::P2pError::Receive { ref message } if message == "Hash mismatch in stream") => {
            return Err(e.to_string());
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
        Ok(Message::RetrieveShardResponse { data: Some(data), .. }) => {
            let actual: [u8; 32] = blake3::hash(&data).into();
            if actual != shard_hash_bytes {
                Err(crate::p2p_error::P2pError::Receive { message: "Hash mismatch in legacy retrieve".to_string() }.to_string())
            } else {
                sink.write_chunk(&data).await
            }
        }
        Ok(_) => Err(crate::p2p_error::P2pError::ShardNotFound.to_string()),
        Err(e) => Err(crate::p2p_error::P2pError::Receive { message: e.to_string() }.to_string()),
    };
    let _ = send2.finish();
    conn.close(0u32.into(), if result.is_ok() { b"done" } else { b"error" });
    result
}

/// Retrieve a shard from a specific node by its 64-hex hash.
pub async fn retrieve_shard(
    p2p_service: &Arc<P2PService>,
    addr: SocketAddr,
    shard_hash_hex: &str,
) -> Result<Vec<u8>, String> {
    let mut data = Vec::new();
    retrieve_shard_into(p2p_service, addr, shard_hash_hex, ShardSink::Vec(&mut data)).await?;
    Ok(data)
}

/// Stream a shard into `out_path` and BLAKE3-verify it with O(1) RAM.
pub async fn retrieve_shard_to_file(
    p2p_service: &Arc<P2PService>,
    addr: SocketAddr,
    shard_hash_hex: &str,
    out_path: &Path,
) -> Result<(), String> {
    let mut file = tokio::fs::File::create(out_path).await
        .map_err(|e| format!("Failed to create shard file {}: {}", out_path.display(), e))?;
    let res = retrieve_shard_into(p2p_service, addr, shard_hash_hex, ShardSink::File(&mut file)).await;
    if res.is_err() {
        drop(file);
        let _ = tokio::fs::remove_file(out_path).await;
    } else {
        use tokio::io::AsyncWriteExt;
        file.flush().await.map_err(|e| e.to_string())?;
    }
    res
}

/// Verify a shard's integrity on a remote node without allocating the shard in RAM
/// and, on updated nodes, without transferring shard bytes over the network.
pub async fn verify_shard_proof(
    p2p_service: &Arc<P2PService>,
    addr: SocketAddr,
    shard_hash_hex: &str,
) -> Result<bool, String> {
    let shard_hash_bytes: [u8; 32] = hex::decode(shard_hash_hex)
        .map_err(|e| format!("Invalid shard hash hex '{}': {}", shard_hash_hex, e))?
        .try_into()
        .map_err(|_| format!("Invalid shard hash hex length '{}'", shard_hash_hex))?;

    let conn = p2p_service.connect_to_addr(addr).await.map_err(|e| e.to_string())?;
    let (mut send, mut recv) = conn.open_bi().await.map_err(|e| e.to_string())?;
    let token = p2p_service.identity().create_shard_token(np2p::crypto::ShardOp::Verify, &shard_hash_bytes);

    let send_res = Protocol::send(&mut send, &Message::VerifyShardRequest {
        shard_hash: shard_hash_bytes,
        challenge: [0u8; 32],
        token,
    }).await;
    if let Err(e) = send_res {
        conn.close(0u32.into(), b"error");
        return Err(e.to_string());
    }

    let recv_res = Protocol::receive(&mut recv).await;
    let _ = send.finish();
    conn.close(0u32.into(), b"done");

    match recv_res {
        Ok(Message::VerifyShardResponse { proof_hash: Some(proof), .. }) => {
            Ok(proof == shard_hash_bytes)
        }
        Ok(Message::VerifyShardResponse { proof_hash: None, .. }) => Ok(false),
        Ok(Message::Error { code: 400, .. })
        | Err(np2p::error::Np2pError::UnknownMessage(_)) => {
            // Backward compatibility: older storage node doesn't understand VerifyShardRequest.
            // Stream & hash in O(1) RAM instead of failing or pulling multi-GB Vec<u8>.
            match retrieve_shard_into(p2p_service, addr, shard_hash_hex, ShardSink::HashOnly).await {
                Ok(()) => Ok(true),
                Err(e) if e.contains("not found") || e.contains("Hash mismatch") => Ok(false),
                Err(e) => Err(e),
            }
        }
        Ok(Message::Error { code, message }) => Err(format!("Node verify error {}: {}", code, message)),
        Ok(other) => Err(format!("Unexpected verify response: {:?}", other)),
        Err(e) => Err(e.to_string()),
    }
}

/// Locates a media file on local disk across configured image and video directories.
pub fn find_local_media_path(config: &crate::config::Config, file_hash: &str, ext: &str) -> Option<std::path::PathBuf> {
    if file_hash.len() < 2 {
        return None;
    }
    for dir in [config.get_images_dir(), config.get_videos_dir()] {
        let candidate = std::path::PathBuf::from(dir)
            .join(&file_hash[0..2])
            .join(format!("{}.{}", file_hash, ext));
        if candidate.exists() {
            return Some(candidate);
        }
    }
    None
}

/// Streams a shard file from local disk to `addr` in bounded 16 MiB chunks (O(1) RAM).
pub async fn store_shard_from_file(
    p2p_service: &Arc<P2PService>,
    addr: SocketAddr,
    file_hash_bytes: [u8; 32],
    shard_index: u8,
    shard_path: &Path,
) -> Result<String, String> {
    let meta = tokio::fs::metadata(shard_path).await.map_err(|e| e.to_string())?;
    let declared_bytes = if meta.len() <= (np2p::network::protocol::MAX_MESSAGE_LEN as u64 - 1024) {
        meta.len()
    } else {
        0
    };
    let node_id = p2p_service.registry.find_by_addr(addr)
        .unwrap_or_else(|| addr.to_string());
    let conn = p2p_service.connect_to_addr(addr).await.map_err(|e| e.to_string())?;

    let result: Result<String, String> = async {
        let (mut send, mut recv) = begin_shard_stream(
            &conn,
            &p2p_service.identity(),
            file_hash_bytes,
            shard_index,
            declared_bytes,
            0,
        ).await?;

        let mut f = tokio::fs::File::open(shard_path).await.map_err(|e| e.to_string())?;
        let mut buf = vec![0u8; CHUNK_MSG_SIZE];
        let mut hasher = blake3::Hasher::new();
        loop {
            let n = read_full(&mut f, &mut buf).await.map_err(|e| e.to_string())?;
            if n == 0 {
                break;
            }
            hasher.update(&buf[..n]);
            Protocol::send(&mut send, &Message::StoreShardChunk { data: buf[..n].to_vec() })
                .await.map_err(|e| e.to_string())?;
        }
        let shard_hash: [u8; 32] = hasher.finalize().into();
        let success = finish_shard_stream(&mut send, &mut recv, &node_id, shard_hash).await?;
        let _ = send.finish();
        if success {
            let hash_hex = blake3::Hash::from(shard_hash).to_hex().to_string();
            note_recent_store(&node_id, &hash_hex);
            Ok(hash_hex)
        } else {
            Err("node rejected streamed shard file (success=false)".to_string())
        }
    }.await;

    conn.close(0u32.into(), if result.is_ok() { b"done" } else { b"error" });
    result
}

/// Re-shards a local media file (single- or multi-segment) and streams ONLY `shard_index`
/// to `addr` without materializing multi-GB shards in memory. Verifies local file BLAKE3
/// matches `expected_file_hash` before touching the network.
#[allow(clippy::too_many_arguments)]
pub async fn stream_reshard_single_index(
    p2p_service: &Arc<P2PService>,
    addr: SocketAddr,
    file_path: &Path,
    expected_file_hash: &str,
    key: &[u8],
    segment_count: i32,
    shard_index: usize,
    data_shards: usize,
    parity_shards: usize,
) -> Result<String, String> {
    let total_shards = data_shards + parity_shards;
    if shard_index >= total_shards {
        return Err(format!("Shard index {} out of range (total {})", shard_index, total_shards));
    }

    // Stream file first in O(1) RAM to verify local file still matches expected content hash.
    {
        let mut f = tokio::fs::File::open(file_path).await.map_err(|e| e.to_string())?;
        let mut hasher = blake3::Hasher::new();
        let mut buf = vec![0u8; 1024 * 1024];
        loop {
            let n = f.read(&mut buf).await.map_err(|e| e.to_string())?;
            if n == 0 { break; }
            hasher.update(&buf[..n]);
        }
        let local_hash = hasher.finalize().to_hex().to_string();
        if local_hash != expected_file_hash {
            return Err(format!(
                "Local file hash mismatch ({} != {}) — refusing to reshard modified file",
                &local_hash[..16.min(local_hash.len())],
                &expected_file_hash[..16.min(expected_file_hash.len())]
            ));
        }
    }

    let file_hash_bytes: [u8; 32] = hex::decode(expected_file_hash)
        .map_err(|e| format!("Invalid file hash '{}': {}", expected_file_hash, e))?
        .try_into()
        .map_err(|_| format!("Invalid file hash length '{}'", expected_file_hash))?;

    let is_segmented = segment_count > 1;

    if !is_segmented {
        let file_data = tokio::fs::read(file_path).await.map_err(|e| e.to_string())?;
        let (shards, _) = StorageEngine::process_for_backup(&file_data, key, key, data_shards, parity_shards)
            .map_err(|e| e.to_string())?;
        return store_shard(p2p_service, addr, file_hash_bytes, shard_index as u8, &shards[shard_index]).await;
    }

    let node_id = p2p_service.registry.find_by_addr(addr)
        .unwrap_or_else(|| addr.to_string());
    let conn = p2p_service.connect_to_addr(addr).await.map_err(|e| e.to_string())?;

    let result: Result<String, String> = async {
        let (mut send, mut recv) = begin_shard_stream(
            &conn,
            &p2p_service.identity(),
            file_hash_bytes,
            shard_index as u8,
            0,
            segment_count.max(1) as u32,
        ).await?;

        let mut file_handle = tokio::fs::File::open(file_path).await.map_err(|e| e.to_string())?;
        let mut buf = vec![0u8; SEGMENT_THRESHOLD];
        let mut shard_hasher = blake3::Hasher::new();
        let mut seg_idx: u32 = 0;

        loop {
            let total = read_full(&mut file_handle, &mut buf).await.map_err(|e| e.to_string())?;
            if total == 0 {
                break;
            }
            let nonce_ctx: Vec<u8> = key.iter().chain(seg_idx.to_le_bytes().iter()).cloned().collect();
            let (sub_shards, _) = StorageEngine::process_for_backup(&buf[..total], key, &nonce_ctx, data_shards, parity_shards)
                .map_err(|e| e.to_string())?;
            let sub = &sub_shards[shard_index];
            for chunk in sub.chunks(CHUNK_MSG_SIZE) {
                shard_hasher.update(chunk);
                Protocol::send(&mut send, &Message::StoreShardChunk { data: chunk.to_vec() })
                    .await.map_err(|e| e.to_string())?;
            }
            seg_idx += 1;
        }

        let shard_hash: [u8; 32] = shard_hasher.finalize().into();
        let success = finish_shard_stream(&mut send, &mut recv, &node_id, shard_hash).await?;
        let _ = send.finish();
        if success {
            let hash_hex = blake3::Hash::from(shard_hash).to_hex().to_string();
            note_recent_store(&node_id, &hash_hex);
            Ok(hash_hex)
        } else {
            Err("node rejected streamed reshard (success=false)".to_string())
        }
    }.await;

    conn.close(0u32.into(), if result.is_ok() { b"done" } else { b"error" });
    result
}

/// Delete a named pinned object on a peer node (best-effort across older nodes).
pub async fn delete_pinned_remote(
    p2p_service: &Arc<P2PService>,
    node_id: &str,
    name: &str,
) -> Result<bool, String> {
    let name_hash: [u8; 32] = blake3::hash(name.as_bytes()).into();
    let token = p2p_service.identity().create_shard_token(np2p::crypto::ShardOp::Delete, &name_hash);
    let msg = Message::DeletePinnedObject { name: name.to_string(), token };
    match p2p_service.send_message(node_id, &msg).await {
        Ok(Message::DeletePinnedResponse { success }) => Ok(success),
        Ok(Message::Error { code: 400, .. }) | Err(np2p::error::Np2pError::UnknownMessage(_)) => Ok(false),
        Ok(other) => Err(format!("Unexpected delete pinned response: {:?}", other)),
        Err(e) => Err(e.to_string()),
    }
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

    let result: Result<String, String> = async {
        let (mut send, mut recv) = begin_shard_stream(
            &conn,
            &p2p_service.identity(),
            file_hash_bytes,
            idx as u8,
            0,
            0,
        ).await?;

        let mut hasher = blake3::Hasher::new();
        while let Some(chunk) = rx.recv().await {
            hasher.update(&chunk);
            Protocol::send(&mut send, &Message::StoreShardChunk { data: chunk }).await.map_err(|e| e.to_string())?;
        }
        let shard_hash: [u8; 32] = hasher.finalize().into();
        let success = finish_shard_stream(&mut send, &mut recv, &node_id, shard_hash).await?;
        let _ = send.finish();
        if success {
            let hash_hex = blake3::Hash::from(shard_hash).to_hex().to_string();
            note_recent_store(&node_id, &hash_hex);
            Ok(hash_hex)
        } else {
            Err("node rejected shard stream (success=false)".to_string())
        }
    }.await;

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

/// Streams a verified shard from `from_addr` through a 0600 scratch file onto `to_addr`
/// with O(1) RAM (no multi-GB in-memory shard buffer).
pub async fn copy_shard_between_nodes(
    p2p_service: &Arc<P2PService>,
    from_addr: SocketAddr,
    to_addr: SocketAddr,
    shard_hash_hex: &str,
    shard_index: u8,
) -> Result<String, String> {
    let shard_hash_bytes: [u8; 32] = hex::decode(shard_hash_hex)
        .map_err(|e| format!("Invalid shard hash hex '{}': {}", shard_hash_hex, e))?
        .try_into()
        .map_err(|_| format!("Invalid shard hash hex length '{}'", shard_hash_hex))?;

    let tmp_name = format!(
        "reminisce_shard_xfer_{}_{}_{}.part",
        std::process::id(),
        shard_index,
        chrono::Utc::now().timestamp_nanos_opt().unwrap_or(0)
    );
    let tmp_path = std::env::temp_dir().join(tmp_name);
    retrieve_shard_to_file(p2p_service, from_addr, shard_hash_hex, &tmp_path).await?;
    let upload_res = store_shard_from_file(p2p_service, to_addr, shard_hash_bytes, shard_index, &tmp_path).await;
    let _ = tokio::fs::remove_file(&tmp_path).await;
    let uploaded_hash = upload_res?;
    if uploaded_hash != shard_hash_hex {
        return Err(format!(
            "Shard transfer hash mismatch ({} != {})",
            &uploaded_hash[..16.min(uploaded_hash.len())],
            &shard_hash_hex[..16.min(shard_hash_hex.len())]
        ));
    }
    Ok(uploaded_hash)
}
