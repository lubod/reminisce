use std::net::SocketAddr;
use tokio::net::TcpStream;
use tracing::{debug, info, warn};
use crate::crypto::NodeIdentity;
use crate::network::protocol::{Message, Protocol};
use crate::network::transport::Node;

const RECONNECT_DELAY_SECS: u64 = 5;

/// Start the tunnel client on the home server side.
///
/// Connects to the coordinator via QUIC and registers as the HTTP backend.
/// The coordinator will open new QUIC streams for each incoming TCP client (Android app).
/// Each stream is piped to `127.0.0.1:local_port` (the local HTTP/HTTPS server).
///
/// Automatically reconnects if the QUIC connection drops.
pub fn start_tunnel_client(
    coordinator_addr: SocketAddr,
    node: Node,
    identity: NodeIdentity,
    local_port: u16,
) {
    let node_id = hex::encode(identity.node_id());
    tokio::spawn(async move {
        info!(
            "[TUNNEL] Client starting — coordinator={} local_port={}",
            coordinator_addr, local_port
        );
        // Brief delay before first registration attempt: ensures the coordinator has
        // already run remove() for any stale entry from a prior connection, preventing
        // a race where our insert() is wiped by the old task's cleanup.
        tokio::time::sleep(std::time::Duration::from_secs(3)).await;
        loop {
            let delay = match run_tunnel(&node, coordinator_addr, &node_id, &identity, local_port).await {
                Ok(_) => { info!("[TUNNEL] Connection ended cleanly"); RECONNECT_DELAY_SECS }
                Err(crate::error::Np2pError::UnknownMessage(msg)) => {
                    debug!("[TUNNEL] Protocol version mismatch with coordinator ({}), retrying in 60s", msg);
                    60
                }
                Err(e) => { warn!("[TUNNEL] Connection lost: {} — reconnecting in {}s", e, RECONNECT_DELAY_SECS); RECONNECT_DELAY_SECS }
            };
            tokio::time::sleep(std::time::Duration::from_secs(delay)).await;
        }
    });
}

async fn run_tunnel(
    node: &Node,
    coordinator_addr: SocketAddr,
    node_id: &str,
    identity: &NodeIdentity,
    local_port: u16,
) -> crate::error::Result<()> {
    let conn = tokio::time::timeout(
        std::time::Duration::from_secs(10),
        node.connect(coordinator_addr),
    )
    .await
    .map_err(|_| crate::error::Np2pError::Network("Tunnel connect timed out".into()))??;

    // Challenge-response authentication
    let (mut send, mut recv) = conn.open_bi().await?;

    // Step 1: send our node_id (public key)
    Protocol::send(&mut send, &Message::TunnelRegister { node_id: node_id.to_string() }).await?;

    // Step 2: receive nonce from coordinator
    let nonce = match Protocol::receive(&mut recv).await? {
        Message::TunnelChallenge { nonce } => nonce,
        other => return Err(crate::error::Np2pError::Protocol(
            format!("Expected TunnelChallenge, got {:?}", other),
        )),
    };

    // Step 3: sign the nonce and send back
    let signature = identity.sign(&nonce);
    Protocol::send(&mut send, &Message::TunnelChallengeResponse { signature }).await?;
    let _ = send.finish();

    // Step 4: wait for acceptance
    match Protocol::receive(&mut recv).await? {
        Message::TunnelAccepted => info!("[TUNNEL] Registered with coordinator (challenge-response passed)"),
        Message::Error { code, message } => return Err(crate::error::Np2pError::Protocol(
            format!("Coordinator rejected tunnel: {} {}", code, message),
        )),
        other => return Err(crate::error::Np2pError::Protocol(
            format!("Expected TunnelAccepted, got {:?}", other),
        )),
    }

    info!("[TUNNEL] Ready — waiting for incoming connections via coordinator");

    // The coordinator opens new QUIC streams for each Android client.
    // Each stream is a transparent pipe to the local HTTP server.
    loop {
        match conn.accept_bi().await {
            Ok((quic_send, quic_recv)) => {
                tokio::spawn(pipe_to_local(quic_send, quic_recv, local_port));
            }
            Err(e) => return Err(e.into()),
        }
    }
}

async fn pipe_to_local(
    mut quic_send: quinn::SendStream,
    mut quic_recv: quinn::RecvStream,
    local_port: u16,
) {
    let addr = format!("127.0.0.1:{}", local_port);
    let mut tcp = match TcpStream::connect(&addr).await {
        Ok(t) => t,
        Err(e) => {
            warn!("[TUNNEL] Cannot connect to local server {}: {}", addr, e);
            return;
        }
    };

    let (mut tcp_recv, mut tcp_send) = tcp.split();

    let client_to_server = tokio::io::copy(&mut quic_recv, &mut tcp_send);
    let server_to_client = tokio::io::copy(&mut tcp_recv, &mut quic_send);

    // 5-minute timeout: prevents stalled Android connections from holding streams open.
    let timeout = tokio::time::sleep(std::time::Duration::from_secs(300));
    tokio::select! {
        _ = client_to_server => {},
        _ = server_to_client => {},
        _ = timeout => {},
    }
}
