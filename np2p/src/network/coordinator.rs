use std::net::SocketAddr;
use tracing::{info, warn};
use crate::network::peer_registry::PeerRegistry;
use crate::network::protocol::{Message, Protocol};
use crate::network::transport::Node;

const REGISTER_INTERVAL_SECS: u64 = 30;

/// Send a message to `target_node_id` via the coordinator relay.
///
/// Use this when direct connection to the target is not possible.
/// The coordinator forwards the serialized message and pipes back the response.
pub async fn relay_message(
    coordinator_addr: SocketAddr,
    node: &Node,
    target_node_id: &str,
    message: &Message,
) -> crate::error::Result<Message> {
    let payload = bincode::serialize(message)?;

    let conn = tokio::time::timeout(
        std::time::Duration::from_secs(10),
        node.connect(coordinator_addr),
    )
    .await
    .map_err(|_| crate::error::Np2pError::Network("Relay connect timed out".into()))??;

    let (mut send, mut recv) = conn.open_bi().await?;

    Protocol::send(&mut send, &Message::RelayRequest {
        target_node_id: target_node_id.to_string(),
        payload,
    })
    .await?;
    let _ = send.finish();

    match Protocol::receive(&mut recv).await? {
        Message::RelayResponse { payload } => {
            let msg: Message = bincode::deserialize(&payload)?;
            Ok(msg)
        }
        Message::Error { code, message } => Err(crate::error::Np2pError::Network(
            format!("Relay error {}: {}", code, message),
        )),
        _ => Err(crate::error::Np2pError::Protocol(
            "Unexpected relay response".into(),
        )),
    }
}

/// Called by storage nodes (quic_port = Some) and the main server (quic_port = None).
///
/// Connects to the coordinator over QUIC, registers (if quic_port is set),
/// fetches the peer list, and merges it into the registry. Repeats every 30s.
pub fn start_coordinator_client(
    coordinator_addr: SocketAddr,
    node: Node,
    node_id: String,
    quic_port: Option<u16>,
    registry: PeerRegistry,
) {
    tokio::spawn(async move {
        info!("[COORDINATOR] Client started for {}", coordinator_addr);

        loop {
            match run_once(&node, coordinator_addr, &node_id, quic_port, &registry).await {
                Ok(n) => {
                    if n > 0 {
                        info!("[COORDINATOR] Merged {} peers from coordinator", n);
                    }
                }
                Err(e) => warn!("[COORDINATOR] Exchange failed: {}", e),
            }

            tokio::time::sleep(std::time::Duration::from_secs(REGISTER_INTERVAL_SECS)).await;
        }
    });
}

async fn run_once(
    node: &Node,
    coordinator_addr: SocketAddr,
    node_id: &str,
    quic_port: Option<u16>,
    registry: &PeerRegistry,
) -> crate::error::Result<usize> {
    let conn = tokio::time::timeout(
        std::time::Duration::from_secs(10),
        node.connect(coordinator_addr),
    )
    .await
    .map_err(|_| crate::error::Np2pError::Network("Coordinator connect timed out".into()))??;

    let (mut send, mut recv) = conn.open_bi().await?;

    let msg = if let Some(port) = quic_port {
        Message::RegisterNode {
            node_id: node_id.to_string(),
            quic_port: port,
        }
    } else {
        Message::GetPeers
    };

    Protocol::send(&mut send, &msg).await?;
    let _ = send.finish();

    let response = Protocol::receive(&mut recv).await?;

    match response {
        Message::PeerList { peers } => {
            let mut added = 0usize;
            for (peer_node_id, addr_str) in peers {
                if peer_node_id == node_id {
                    continue;
                }
                match addr_str.parse::<SocketAddr>() {
                    Ok(addr) => {
                        registry.upsert(peer_node_id, addr);
                        added += 1;
                    }
                    Err(_) => warn!("[COORDINATOR] Unparseable addr: {}", addr_str),
                }
            }
            Ok(added)
        }
        Message::Error { code, message } => Err(crate::error::Np2pError::Network(
            format!("Coordinator error {}: {}", code, message),
        )),
        _ => Err(crate::error::Np2pError::Protocol("Unexpected response from coordinator".into())),
    }
}
