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
}

impl ConnectionHandler {
    pub fn new(connection: Connection, storage: DiskStorage, identity: Arc<NodeIdentity>) -> Self {
        Self {
            connection,
            storage,
            identity,
            custom_handler: None,
        }
    }

    pub fn with_custom_handler(mut self, handler: Arc<dyn P2PHandler>) -> Self {
        self.custom_handler = Some(handler);
        self
    }

    /// The main loop for handling a connection.
    /// Accepts incoming bidirectional streams and processes messages.
    pub async fn run(self) {
        info!("[CONN] Handling connection from {}", self.connection.remote_address());

        loop {
            match self.connection.accept_bi().await {
                Ok((send, recv)) => {
                    let storage = self.storage.clone();
                    let identity = self.identity.clone();
                    let custom = self.custom_handler.clone();

                    tokio::spawn(async move {
                        if let Err(e) = Self::handle_stream(send, recv, storage, identity, custom).await {
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
                let response = Message::HandshakeAck {
                    node_id: identity.node_id(),
                };
                Protocol::send(&mut send, &response).await?;
            }

            Message::StoreShardRequest { shard_hash, data } => {
                let success = storage.store(shard_hash, &data).await.is_ok();
                let response = Message::StoreShardResponse { shard_hash, success };
                Protocol::send(&mut send, &response).await?;
            }

            Message::StoreShardStreamInit { file_hash, shard_index, .. } => {
                // Stable temp-file ID derived from (file_hash, shard_index) avoids collisions
                // between concurrent uploads of different files or shards.
                let temp_id: [u8; 32] = blake3::hash(
                    &[file_hash.as_slice(), &[shard_index]].concat()
                ).into();
                let temp_path = storage.temp_path(&temp_id);

                Protocol::send(&mut send, &Message::StoreShardStreamAck { ready: true }).await?;

                // Accumulate BLAKE3 as chunks arrive — avoids re-reading the full shard at finalize.
                let mut hasher = blake3::Hasher::new();
                loop {
                    match Protocol::receive(&mut recv).await {
                        Ok(Message::StoreShardChunk { data }) => {
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

            Message::RetrieveShardRequest { shard_hash } => {
                let data = storage.get(shard_hash).await?;
                let response = Message::RetrieveShardResponse { shard_hash, data };
                Protocol::send(&mut send, &response).await?;
            }

            Message::Heartbeat { available_space_bytes } => {
                info!("[CONN] Heartbeat: {} bytes available", available_space_bytes);
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
