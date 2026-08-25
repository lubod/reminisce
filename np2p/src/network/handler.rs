use quinn::{Connection, SendStream, RecvStream};
use crate::error::Result;
use crate::network::protocol::{Message, Protocol};
use crate::storage::DiskStorage;
use crate::crypto::NodeIdentity;
use tracing::{debug, info, warn, error};
use std::sync::Arc;
use std::sync::OnceLock;
use hex;

/// Global cap on concurrently-executing inbound stream handlers across the whole
/// storage node (QUIC listener + reverse-channel relays). Bounds task/memory
/// blowup when a peer (or coordinator) opens unbounded numbers of streams.
/// Once exhausted, new streams wait for a permit before being spawned.
pub const MAX_CONCURRENT_STREAMS: usize = 64;

static STREAM_SEMAPHORE: OnceLock<Arc<tokio::sync::Semaphore>> = OnceLock::new();

/// The shared inbound-stream gate for this process.
pub fn inbound_stream_semaphore() -> Arc<tokio::sync::Semaphore> {
    STREAM_SEMAPHORE
        .get_or_init(|| Arc::new(tokio::sync::Semaphore::new(MAX_CONCURRENT_STREAMS)))
        .clone()
}

/// Handles a single connection to a peer.
pub struct ConnectionHandler {
    connection: Connection,
    storage: DiskStorage,
    identity: Arc<NodeIdentity>,
    pub allowed_owner_id: Option<[u8; 32]>,
}

impl ConnectionHandler {
    pub fn new(connection: Connection, storage: DiskStorage, identity: Arc<NodeIdentity>) -> Self {
        Self {
            connection,
            storage,
            identity,
            allowed_owner_id: None,
        }
    }

    pub fn with_allowed_owner(mut self, allowed_owner_id: Option<[u8; 32]>) -> Self {
        self.allowed_owner_id = allowed_owner_id;
        self
    }

    /// The main loop for handling a connection.
    /// Accepts incoming bidirectional streams and processes messages.
    pub async fn run(self) {
        info!("[CONN] Handling connection from {}", self.connection.remote_address());
        let allowed_owner = self.allowed_owner_id;

        loop {
            match self.connection.accept_bi().await {
                Ok((send, recv)) => {
                    let storage = self.storage.clone();
                    let identity = self.identity.clone();
                    let remote_addr = self.connection.remote_address();

                    // Backpressure: wait for a permit BEFORE spawning so concurrent
                    // stream handlers never exceed MAX_CONCURRENT_STREAMS. The permit
                    // is held by the supervisor task and released when it completes.
                    let permit = match inbound_stream_semaphore().acquire_owned().await {
                        Ok(permit) => permit,
                        Err(_) => {
                            warn!("[CONN] Stream semaphore closed — dropping stream from {}", remote_addr);
                            continue;
                        }
                    };

                    // Supervisor task: awaits the worker's JoinHandle so a panic in
                    // handle_stream is logged with the peer address instead of the
                    // task dying silently.
                    tokio::spawn(async move {
                        let _permit = permit;
                        let worker = tokio::spawn(async move {
                            if let Err(e) = Self::handle_stream(send, recv, storage, identity, allowed_owner).await {
                                if matches!(e, crate::error::Np2pError::UnknownMessage(_)) {
                                    debug!("[CONN] Unknown message from peer (version mismatch): {}", e);
                                } else {
                                    error!("[CONN] Stream error: {}", e);
                                }
                            }
                        });
                        match worker.await {
                            Ok(()) => {}
                            Err(join_err) => {
                                if join_err.is_panic() {
                                    error!("[CONN] Stream handler PANICKED (peer {}): {}", remote_addr, join_err);
                                } else {
                                    warn!("[CONN] Stream task cancelled (peer {})", remote_addr);
                                }
                            }
                        }
                    });
                }
                Err(e) => {
                    warn!("[CONN] Connection closed: {}", e);
                    break;
                }
            }
        }
    }

    pub async fn handle_stream(
        mut send: SendStream,
        mut recv: RecvStream,
        storage: DiskStorage,
        identity: Arc<NodeIdentity>,
        allowed_owner_id: Option<[u8; 32]>,
    ) -> Result<()> {
        // Read deadline for EVERY inbound message: without it a peer that opens
        // streams and goes silent pins a semaphore permit indefinitely (our own
        // 15s keep-alive pings keep the QUIC idle timeout from ever firing).
        const STREAM_READ_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(120);
        let msg = tokio::time::timeout(STREAM_READ_TIMEOUT, Protocol::receive(&mut recv))
            .await
            .map_err(|_| {
                warn!("[CONN] First message timeout from peer — dropping stream");
                crate::error::Np2pError::Network("stream read timeout".to_string())
            })??;

        match msg {
            Message::Handshake { node_id, version } => {
                info!("[CONN] Handshake from {}, version {}", hex::encode(node_id), version);
                // Enforce protocol version compatibility (wire format stability).
                if version != crate::PROTOCOL_VERSION {
                    warn!(
                        "[CONN] Protocol version mismatch: peer {}, peer version {}, local {} — rejecting",
                        hex::encode(node_id), version, crate::PROTOCOL_VERSION
                    );
                    let _ = Protocol::send(&mut send, &Message::Error {
                        code: 426,
                        message: format!(
                            "Protocol version mismatch: peer {}, local {}",
                            version, crate::PROTOCOL_VERSION
                        ),
                    }).await;
                    return Ok(());
                }
                let response = Message::HandshakeAck {
                    node_id: identity.node_id(),
                };
                Protocol::send(&mut send, &response).await?;
            }

            Message::StoreShardRequest { shard_hash, data, token } => {
                let computed_hash: [u8; 32] = blake3::hash(&data).into();
                let success = if computed_hash != shard_hash {
                    warn!(
                        "StoreShardRequest: hash mismatch: computed {}, request {}",
                        hex::encode(computed_hash),
                        hex::encode(shard_hash)
                    );
                    false
                } else if crate::crypto::verify_shard_token(&token, crate::crypto::ShardOp::Store, &shard_hash, allowed_owner_id.as_ref()) {
                    storage.store(shard_hash, &data).await.is_ok()
                } else {
                    warn!("StoreShardRequest: token verification failed for shard {}", hex::encode(shard_hash));
                    false
                };
                let response = Message::StoreShardResponse { shard_hash, success, available_space_bytes: storage.available_space() };
                Protocol::send(&mut send, &response).await?;
            }

            Message::StoreShardStreamInit { file_hash, shard_index, token, total_shard_bytes, .. } => {
                // Temp-file ID derived from (file_hash, shard_index) PLUS a per-connection
                // random salt: deterministic addressing avoids collisions between different
                // streams, while the salt prevents two concurrent duplicate uploads of the
                // same (file_hash, shard_index) from interleaving appends into one file.
                let mut salt = [0u8; 16];
                rand::RngCore::fill_bytes(&mut rand::thread_rng(), &mut salt);
                let temp_id: [u8; 32] = blake3::hash(
                    &[file_hash.as_slice(), &[shard_index], &salt].concat()
                ).into();
                let temp_path = storage.temp_path(&temp_id);

                // Authenticate the stream BEFORE accepting any data: the token is bound
                // to blake3(file_hash || shard_index). Without a valid token an
                // unauthenticated peer could stream unbounded data to fill the disk.
                let binding: [u8; 32] = blake3::hash(
                    &[file_hash.as_slice(), &[shard_index]].concat()
                ).into();
                // Retrieval ships the shard as ONE protocol frame capped at
                // MAX_MESSAGE_LEN — anything larger can be stored but never
                // retrieved. Refuse at store time instead of stranding bytes.
                const MAX_RETRIEVABLE: u64 = (crate::network::protocol::MAX_MESSAGE_LEN as u64) - 1024;
                if total_shard_bytes > MAX_RETRIEVABLE {
                    warn!("[CONN] Shard {} declares {} bytes > retrievable cap {} — rejecting", shard_index, total_shard_bytes, MAX_RETRIEVABLE);
                    let _ = Protocol::send(&mut send, &Message::Error {
                        code: 413,
                        message: format!("Shard exceeds retrievable size cap ({} MB)", MAX_RETRIEVABLE / (1024 * 1024)),
                    }).await;
                    return Ok(());
                }

                if !crate::crypto::verify_shard_token(&token, crate::crypto::ShardOp::Store, &binding, allowed_owner_id.as_ref()) {
                    warn!("[CONN] StoreShardStreamInit token verification failed (shard {}) — rejecting", shard_index);
                    let _ = Protocol::send(&mut send, &Message::Error {
                        code: 401,
                        message: "Unauthorized shard stream".to_string(),
                    }).await;
                    return Ok(());
                }

                Protocol::send(&mut send, &Message::StoreShardStreamAck { ready: true }).await?;

                // Accumulate BLAKE3 as chunks arrive — avoids re-reading the full shard at finalize.
                let mut hasher = blake3::Hasher::new();
                let mut received_bytes: u64 = 0;
                loop {
                    let frame = match tokio::time::timeout(STREAM_READ_TIMEOUT, Protocol::receive(&mut recv)).await {
                        Ok(r) => r,
                        Err(_) => {
                            error!("[CONN] Chunk timeout on shard {} — aborting stream", shard_index);
                            let _ = tokio::fs::remove_file(&temp_path).await;
                            let _ = Protocol::send(&mut send, &Message::StoreShardStreamResponse { success: false, available_space_bytes: storage.available_space() }).await;
                            break;
                        }
                    };
                    match frame {
                        Ok(Message::StoreShardChunk { data }) => {
                            received_bytes = received_bytes.saturating_add(data.len() as u64);
                            // Bound total disk used per shard stream regardless of what the
                            // init declared (defense against disk-exhaustion DoS).
                            if received_bytes > crate::storage::MAX_SHARD_BYTES {
                                warn!("[CONN] Shard {} exceeded max size ({} bytes) — aborting stream", shard_index, received_bytes);
                                let _ = tokio::fs::remove_file(&temp_path).await;
                                let _ = Protocol::send(&mut send, &Message::StoreShardStreamResponse { success: false, available_space_bytes: storage.available_space() }).await;
                                break;
                            }
                            hasher.update(&data);
                            if let Err(e) = storage.store_stream_chunk(&temp_path, &data).await {
                                error!("[CONN] Chunk write failed for shard {}: {}", shard_index, e);
                                let _ = tokio::fs::remove_file(&temp_path).await;
                                Protocol::send(&mut send, &Message::StoreShardStreamResponse { success: false, available_space_bytes: storage.available_space() }).await?;
                                break;
                            }
                        }
                        Ok(Message::StoreShardStreamFinal { shard_hash }) => {
                            let computed: [u8; 32] = hasher.finalize().into();
                            let ok = if computed == shard_hash {
                                match storage.finalize_stream_temp(&temp_path, shard_hash).await {
                                    Ok(()) => true,
                                    Err(e) => {
                                        error!("[CONN] finalize_stream_temp failed: {}", e);
                                        false
                                    }
                                }
                            } else {
                                warn!("[CONN] Hash mismatch for shard {} — discarding temp file", shard_index);
                                let _ = tokio::fs::remove_file(&temp_path).await;
                                false
                            };
                            Protocol::send(&mut send, &Message::StoreShardStreamResponse { success: ok, available_space_bytes: storage.available_space() }).await?;
                            break;
                        }
                        Ok(_) | Err(_) => {
                            let _ = tokio::fs::remove_file(&temp_path).await;
                            break;
                        }
                    }
                }
            }

            Message::RetrieveShardRequest { shard_hash, token } => {
                if crate::crypto::verify_shard_token(&token, crate::crypto::ShardOp::Retrieve, &shard_hash, allowed_owner_id.as_ref()) {
                    let data = storage.get(shard_hash).await?;
                    let response = Message::RetrieveShardResponse { shard_hash, data };
                    Protocol::send(&mut send, &response).await?;
                } else {
                    warn!("RetrieveShardRequest: token verification failed for shard {}", hex::encode(shard_hash));
                    let response = Message::Error {
                        code: 401,
                        message: "Unauthorized shard retrieval".to_string(),
                    };
                    Protocol::send(&mut send, &response).await?;
                }
            }

            Message::Heartbeat { available_space_bytes } => {
                info!("[CONN] Heartbeat: {} bytes available", available_space_bytes);
            }

            Message::DeleteShardRequest { shard_hash, token } => {
                let success = if crate::crypto::verify_shard_token(&token, crate::crypto::ShardOp::Delete, &shard_hash, allowed_owner_id.as_ref()) {
                    storage.delete(shard_hash).await.is_ok()
                } else {
                    warn!("DeleteShardRequest: token verification failed for shard {}", hex::encode(shard_hash));
                    false
                };
                let response = Message::DeleteShardResponse { shard_hash, success };
                Protocol::send(&mut send, &response).await?;
            }

            Message::StorePinnedObject { name, data, token } => {
                let name_hash: [u8; 32] = blake3::hash(name.as_bytes()).into();
                let success = if crate::crypto::verify_shard_token(&token, crate::crypto::ShardOp::Store, &name_hash, allowed_owner_id.as_ref()) {
                    storage.store_pinned(&name, &data).await.is_ok()
                } else {
                    warn!("StorePinnedObject: token verification failed for '{}'", name);
                    false
                };
                Protocol::send(&mut send, &Message::StorePinnedResponse { success }).await?;
            }

            Message::GetPinnedObject { name, token } => {
                let name_hash: [u8; 32] = blake3::hash(name.as_bytes()).into();
                if crate::crypto::verify_shard_token(&token, crate::crypto::ShardOp::Retrieve, &name_hash, allowed_owner_id.as_ref()) {
                    let data = storage.get_pinned(&name).await?;
                    Protocol::send(&mut send, &Message::PinnedObjectResponse { data }).await?;
                } else {
                    warn!("GetPinnedObject: token verification failed for '{}'", name);
                    Protocol::send(&mut send, &Message::Error { code: 401, message: "Unauthorized pinned object access".to_string() }).await?;
                }
            }

            Message::ListShardsRequest { prefix, token } => {
                let scope_bytes: [u8; 32] = blake3::hash(prefix.as_deref().unwrap_or("").as_bytes()).into();
                if crate::crypto::verify_shard_token(&token, crate::crypto::ShardOp::List, &scope_bytes, allowed_owner_id.as_ref()) {
                    let shards = storage.list_shards(prefix.as_deref()).await.unwrap_or_default();
                    let response = Message::ListShardsResponse {
                        prefix,
                        shards,
                        available_space_bytes: storage.available_space(),
                    };
                    Protocol::send(&mut send, &response).await?;
                } else {
                    warn!("ListShardsRequest: token verification failed");
                    let response = Message::Error {
                        code: 401,
                        message: "Unauthorized shard listing".to_string(),
                    };
                    Protocol::send(&mut send, &response).await?;
                }
            }

            _ => {
                warn!("[CONN] Received unexpected or unhandled message type");
                let response = Message::Error {
                    code: 400,
                    message: "Unhandled message type".to_string(),
                };
                Protocol::send(&mut send, &response).await?;
            }
        }

        let _ = send.finish();
        Ok(())
    }
}
