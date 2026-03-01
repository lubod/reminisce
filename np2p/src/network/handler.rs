use quinn::{Connection, SendStream, RecvStream};
use crate::error::Result;
use crate::network::protocol::{Message, Protocol};
use crate::storage::DiskStorage;
use crate::crypto::NodeIdentity;
use tracing::{info, warn, error};
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
                            error!("[CONN] Stream error: {}", e);
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

    async fn handle_stream(
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
