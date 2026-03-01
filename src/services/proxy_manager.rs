use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::{mpsc, RwLock};
use bytes::Bytes;

/// Manages active proxy streams for multiplexed WebSocket tunneling.
/// Maps request_id -> Sender for pushing request body chunks.
#[derive(Clone)]
pub struct ProxyManager {
    streams: Arc<RwLock<HashMap<String, mpsc::Sender<Bytes>>>>,
}

impl ProxyManager {
    pub fn new() -> Self {
        Self {
            streams: Arc::new(RwLock::new(HashMap::new())),
        }
    }

    /// Registers a new stream. Returns the receiver for the request body.
    /// Use bounded channel (32 chunks) to provide backpressure.
    pub async fn register(&self, request_id: String) -> mpsc::Receiver<Bytes> {
        let (tx, rx) = mpsc::channel(32);
        let mut streams = self.streams.write().await;
        streams.insert(request_id, tx);
        rx
    }

    /// Pushes a data chunk to an active stream.
    /// Returns error if stream not found or closed.
    pub async fn push_chunk(&self, request_id: &str, chunk: Bytes) -> Result<(), ()> {
        let streams = self.streams.read().await;
        if let Some(tx) = streams.get(request_id) {
            // We await send to apply backpressure to the WebSocket reader
            tx.send(chunk).await.map_err(|_| ())
        } else {
            Err(())
        }
    }

    /// Removes a stream, closing the channel.
    pub async fn remove(&self, request_id: &str) {
        let mut streams = self.streams.write().await;
        streams.remove(request_id);
    }
}
