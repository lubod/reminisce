use std::net::SocketAddr;
use std::sync::Arc;
use tracing::{debug, info, warn};
use crate::crypto::NodeIdentity;
use crate::network::handler::ConnectionHandler;
use crate::network::protocol::{Message, Protocol};
use crate::network::transport::Node;
use crate::network::utils::ReconnectBackoff;
use crate::storage::DiskStorage;

const RECONNECT_DELAY_SECS: u64 = 5;
const MAX_RECONNECT_DELAY_SECS: u64 = 60;

/// Start a persistent reverse channel to the coordinator.
/// Storage nodes call this so the coordinator can relay messages to them even when behind NAT.
/// `authorized_owner_id` is the storage node's `--authorized-node-id` pin; relayed requests
/// are checked against it exactly like direct ones (prevents the relay path bypassing the pin).
pub fn start_channel_client(
    coordinator_addr: SocketAddr,
    coordinator_node_id: &str,
    node: Node,
    identity: NodeIdentity,
    storage: DiskStorage,
    authorized_owner_id: Option<[u8; 32]>,
) {
    let node_id = hex::encode(identity.node_id());
    let coordinator_node_id = coordinator_node_id.to_string();
    tokio::spawn(async move {
        info!("[CHANNEL] Client starting — coordinator={}", coordinator_addr);
        let mut backoff = ReconnectBackoff::new(RECONNECT_DELAY_SECS, MAX_RECONNECT_DELAY_SECS);
        loop {
            match run_channel(&node, coordinator_addr, &coordinator_node_id, &node_id, &identity, &storage, authorized_owner_id).await {
                Ok(_) => {
                    info!("[CHANNEL] Connection ended cleanly");
                    // A completed session means connect + registration succeeded:
                    // start the next retry sequence from the initial delay again.
                    backoff.reset();
                }
                Err(crate::error::Np2pError::UnknownMessage(msg)) => {
                    debug!("[CHANNEL] Protocol version mismatch with coordinator ({})", msg);
                    backoff.skip_to_max();
                }
                Err(e) => {
                    warn!("[CHANNEL] Connection lost: {}", e);
                }
            }
            let delay = backoff.next_delay();
            info!("[CHANNEL] Reconnecting in {}s", delay.as_secs());
            tokio::time::sleep(delay).await;
        }
    });
}

async fn run_channel(
    node: &Node,
    coordinator_addr: SocketAddr,
    coordinator_node_id: &str,
    node_id: &str,
    identity: &NodeIdentity,
    storage: &DiskStorage,
    authorized_owner_id: Option<[u8; 32]>,
) -> crate::error::Result<()> {
    let conn = tokio::time::timeout(
        std::time::Duration::from_secs(10),
        node.connect(coordinator_addr, coordinator_node_id),
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
                // Same global inbound-stream gate as the QUIC listener path, so
                // relayed requests cannot bypass the storage node's stream bound.
                let permit = match crate::network::handler::inbound_stream_semaphore().acquire_owned().await {
                    Ok(permit) => permit,
                    Err(_) => continue,
                };
                tokio::spawn(async move {
                    let _permit = permit;
                    if let Err(e) = ConnectionHandler::handle_stream(send, recv, storage, identity, authorized_owner_id).await {
                        warn!("[CHANNEL] Stream error: {}", e);
                    }
                });
            }
            Err(e) => return Err(e.into()),
        }
    }
}
