use std::net::SocketAddr;
use std::sync::Arc;
use tracing::{info, warn};
use crate::crypto::NodeIdentity;
use crate::network::handler::ConnectionHandler;
use crate::network::protocol::{Message, Protocol};
use crate::network::transport::Node;
use crate::storage::DiskStorage;

const RECONNECT_DELAY_SECS: u64 = 5;

/// Start a persistent reverse channel to the coordinator.
/// Storage nodes call this so the coordinator can relay messages to them even when behind NAT.
pub fn start_channel_client(
    coordinator_addr: SocketAddr,
    node: Node,
    identity: NodeIdentity,
    storage: DiskStorage,
) {
    let node_id = hex::encode(identity.node_id());
    tokio::spawn(async move {
        info!("[CHANNEL] Client starting — coordinator={}", coordinator_addr);
        loop {
            match run_channel(&node, coordinator_addr, &node_id, &identity, &storage).await {
                Ok(_) => info!("[CHANNEL] Connection ended cleanly"),
                Err(e) => warn!("[CHANNEL] Connection lost: {} — reconnecting in {}s", e, RECONNECT_DELAY_SECS),
            }
            tokio::time::sleep(std::time::Duration::from_secs(RECONNECT_DELAY_SECS)).await;
        }
    });
}

async fn run_channel(
    node: &Node,
    coordinator_addr: SocketAddr,
    node_id: &str,
    identity: &NodeIdentity,
    storage: &DiskStorage,
) -> crate::error::Result<()> {
    let conn = tokio::time::timeout(
        std::time::Duration::from_secs(10),
        node.connect(coordinator_addr),
    )
    .await
    .map_err(|_| crate::error::Np2pError::Network("Channel connect timed out".into()))??;

    // Challenge-response authentication
    let (mut send, mut recv) = conn.open_bi().await?;

    // Step 1: register
    Protocol::send(&mut send, &Message::NodeChannelRegister { node_id: node_id.to_string() }).await?;

    // Step 2: receive challenge nonce
    let nonce = match Protocol::receive(&mut recv).await? {
        Message::NodeChannelChallenge { nonce } => nonce,
        other => return Err(crate::error::Np2pError::Protocol(
            format!("Expected NodeChannelChallenge, got {:?}", other),
        )),
    };

    // Step 3: sign and send response
    let signature = identity.sign(&nonce);
    Protocol::send(&mut send, &Message::NodeChannelChallengeResponse { signature }).await?;
    let _ = send.finish();

    // Step 4: wait for acceptance
    match Protocol::receive(&mut recv).await? {
        Message::NodeChannelAccepted => info!("[CHANNEL] Registered with coordinator as {}", node_id),
        Message::Error { code, message } => return Err(crate::error::Np2pError::Protocol(
            format!("Coordinator rejected channel: {} {}", code, message),
        )),
        other => return Err(crate::error::Np2pError::Protocol(
            format!("Expected NodeChannelAccepted, got {:?}", other),
        )),
    }

    info!("[CHANNEL] Ready — waiting for relayed requests from coordinator");

    let identity_arc = Arc::new(identity.clone());
    loop {
        match conn.accept_bi().await {
            Ok((send, recv)) => {
                let storage = storage.clone();
                let identity = identity_arc.clone();
                tokio::spawn(async move {
                    if let Err(e) = ConnectionHandler::handle_stream(send, recv, storage, identity, None).await {
                        warn!("[CHANNEL] Stream error: {}", e);
                    }
                });
            }
            Err(e) => return Err(e.into()),
        }
    }
}
