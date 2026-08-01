use quinn::{Connection, SendStream, RecvStream};
use crate::error::Result;
use crate::network::protocol::{Message, Protocol};
use crate::storage::DiskStorage;
use crate::crypto::NodeIdentity;
use tracing::{debug, info, warn, error};
use std::sync::Arc;
use async_trait::async_trait;
use hex;

#[async_trait]
pub trait P2PHandler: Send + Sync {
    async fn handle_message(&self, msg: Message) -> Result<Option<Message>>;
}

/// Handles a single connection to a peer.
pub struct ConnectionHandler {
    connection: Connection,
    storage: DiskStorage,
    identity: Arc<NodeIdentity>,
    custom_handler: Option<Arc<dyn P2PHandler>>,
    pub allowed_owner_id: Option<[u8; 32]>,
}

impl ConnectionHandler {
    pub fn new(connection: Connection, storage: DiskStorage, identity: Arc<NodeIdentity>) -> Self {
        Self {
            connection,
            storage,
            identity,
            custom_handler: None,
            allowed_owner_id: None,
        }
    }

    pub fn with_custom_handler(mut self, handler: Arc<dyn P2PHandler>) -> Self {
        self.custom_handler = Some(handler);
        self
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
                    let custom = self.custom_handler.clone();

                    tokio::spawn(async move {
                        if let Err(e) = Self::handle_stream(send, recv, storage, identity, custom, allowed_owner).await {
                            if matches!(e, crate::error::Np2pError::UnknownMessage(_)) {
                                debug!("[CONN] Unknown message from peer (version mismatch): {}", e);
                            } else {
                                error!("[CONN] Stream error: {}", e);
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
        custom_handler: Option<Arc<dyn P2PHandler>>,
        allowed_owner_id: Option<[u8; 32]>,
    ) -> Result<()> {
        let msg = Protocol::receive(&mut recv).await?;

        // 1. Try custom handler first
        if let Some(handler) = &custom_handler {
            if let Some(response) = handler.handle_message(msg.clone()).await? {
                Protocol::send(&mut send, &response).await?;
                let _ = send.finish();
                return Ok(());
            }
        }

        // 2. Fallback to default shard handling
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
                } else if crate::crypto::verify_shard_token(&token, &shard_hash, allowed_owner_id.as_ref()) {
                    storage.store(shard_hash, &data).await.is_ok()
                } else {
                    warn!("StoreShardRequest: token verification failed for shard {}", hex::encode(shard_hash));
                    false
                };
                let response = Message::StoreShardResponse { shard_hash, success };
                Protocol::send(&mut send, &response).await?;
            }

            Message::StoreShardStreamInit { file_hash, shard_index, token, .. } => {
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
                if !crate::crypto::verify_shard_token(&token, &binding, allowed_owner_id.as_ref()) {
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
                    match Protocol::receive(&mut recv).await {
                        Ok(Message::StoreShardChunk { data }) => {
                            received_bytes = received_bytes.saturating_add(data.len() as u64);
                            // Bound total disk used per shard stream regardless of what the
                            // init declared (defense against disk-exhaustion DoS).
                            if received_bytes > crate::storage::MAX_SHARD_BYTES {
                                warn!("[CONN] Shard {} exceeded max size ({} bytes) — aborting stream", shard_index, received_bytes);
                                let _ = tokio::fs::remove_file(&temp_path).await;
                                let _ = Protocol::send(&mut send, &Message::StoreShardStreamResponse { success: false }).await;
                                break;
                            }
                            hasher.update(&data);
                            if let Err(e) = storage.store_stream_chunk(&temp_path, &data).await {
                                error!("[CONN] Chunk write failed for shard {}: {}", shard_index, e);
                                let _ = tokio::fs::remove_file(&temp_path).await;
                                Protocol::send(&mut send, &Message::StoreShardStreamResponse { success: false }).await?;
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
                            Protocol::send(&mut send, &Message::StoreShardStreamResponse { success: ok }).await?;
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
                if crate::crypto::verify_shard_token(&token, &shard_hash, allowed_owner_id.as_ref()) {
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
                let success = if crate::crypto::verify_shard_token(&token, &shard_hash, allowed_owner_id.as_ref()) {
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
                let success = if crate::crypto::verify_shard_token(&token, &name_hash, allowed_owner_id.as_ref()) {
                    storage.store_pinned(&name, &data).await.is_ok()
                } else {
                    warn!("StorePinnedObject: token verification failed for '{}'", name);
                    false
                };
                Protocol::send(&mut send, &Message::StorePinnedResponse { success }).await?;
            }

            Message::GetPinnedObject { name, token } => {
                let name_hash: [u8; 32] = blake3::hash(name.as_bytes()).into();
                if crate::crypto::verify_shard_token(&token, &name_hash, allowed_owner_id.as_ref()) {
                    let data = storage.get_pinned(&name).await?;
                    Protocol::send(&mut send, &Message::PinnedObjectResponse { data }).await?;
                } else {
                    warn!("GetPinnedObject: token verification failed for '{}'", name);
                    Protocol::send(&mut send, &Message::Error { code: 401, message: "Unauthorized pinned object access".to_string() }).await?;
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
