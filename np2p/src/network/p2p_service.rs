//! Top-level P2P service: QUIC transport, peer registry, and message dispatch.
//!
//! P2PService owns the Node (QUIC listener/dialer), the PeerRegistry (in-memory
//! address book), and the node identity (Ed25519 keypair). All higher-level
//! operations (store shard, retrieve shard, announce) go through this facade.

use std::net::SocketAddr;
use std::sync::Arc;
use crate::error::Result;
use crate::crypto::NodeIdentity;
use crate::network::transport::Node;
use crate::network::peer_registry::PeerRegistry;
use crate::network::protocol::Message;
use crate::network::coordinator;
use tokio::time::Duration;
use tracing::{info, warn, error};

pub struct P2PService {
    node: Node,
    identity: Arc<NodeIdentity>,
    pub registry: PeerRegistry,
    pub coordinator_addr: Option<SocketAddr>,
    pub coordinator_node_id: Option<String>,
}

impl P2PService {
    pub async fn new(listen_addr: SocketAddr, identity: NodeIdentity) -> Result<Self> {
        let std_socket = std::net::UdpSocket::bind(listen_addr)?;
        std_socket.set_nonblocking(true)?;
        let node = Node::from_socket(std_socket, identity.clone())?;

        Ok(Self {
            node,
            identity: Arc::new(identity),
            registry: PeerRegistry::new(),
            coordinator_addr: None,
            coordinator_node_id: None,
        })
    }

    /// Configure the coordinator for cross-network relay. `coordinator_node_id` is the
    /// coordinator's 64-hex Node ID, bound to the QUIC connection so a spoofed
    /// "coordinator" cannot MITM relayed messages.
    pub fn set_coordinator(&mut self, addr: SocketAddr, node_id: String) {
        self.coordinator_addr = Some(addr);
        self.coordinator_node_id = Some(node_id);
    }

    /// Connect directly to a known socket address.
    ///
    /// The peer's Node ID is resolved from the registry by address and bound to the
    /// QUIC connection. If the address is not a registered peer, the connection is
    /// refused (identity cannot be verified).
    pub async fn connect_to_addr(&self, addr: SocketAddr) -> Result<quinn::Connection> {
        let node_id = self.registry.find_by_addr(addr).ok_or_else(|| {
            crate::error::Np2pError::Network(format!(
                "Cannot verify peer identity: {} not in registry",
                addr
            ))
        })?;
        info!("[P2P] Connecting to {} as node {}", addr, node_id);
        let res = tokio::time::timeout(Duration::from_secs(10), self.node.connect(addr, &node_id)).await;
        match res {
            Ok(Ok(conn)) => {
                info!("[P2P] Connected to {} ({})", addr, node_id);
                Ok(conn)
            }
            Ok(Err(e)) => {
                error!("[P2P] Connection to {} failed: {}", addr, e);
                Err(e)
            }
            Err(_) => {
                error!("[P2P] Connection to {} timed out", addr);
                Err(crate::error::Np2pError::Network(format!("Timeout connecting to {}", addr)))
            }
        }
    }

    /// Connect to a peer by node_id, looking up its address in the registry.
    pub async fn connect_to_peer(&self, node_id: &str) -> Result<quinn::Connection> {
        let peer = self.registry.get(node_id).ok_or_else(|| {
            crate::error::Np2pError::Network(format!("Peer not in registry: {}", node_id))
        })?;
        info!("[P2P] Connecting to peer {} at {}", node_id, peer.addr);
        let res = tokio::time::timeout(Duration::from_secs(10), self.node.connect(peer.addr, node_id)).await;
        match res {
            Ok(Ok(conn)) => {
                info!("[P2P] Connected to peer {} at {}", node_id, peer.addr);
                Ok(conn)
            }
            Ok(Err(e)) => {
                error!("[P2P] Connection to peer {} failed: {}", node_id, e);
                Err(e)
            }
            Err(_) => {
                error!("[P2P] Connection to peer {} timed out", node_id);
                Err(crate::error::Np2pError::Network(format!("Timeout connecting to peer {}", node_id)))
            }
        }
    }

    /// Send a single request Message and receive a response.
    ///
    /// Tries direct connection first. If the peer is unknown or unreachable and a
    /// coordinator is configured, falls back to relay automatically.
    pub async fn send_message(
        &self,
        node_id: &str,
        message: &Message,
    ) -> Result<Message> {
        // Try direct first if we know the peer's address
        if let Some(peer) = self.registry.get(node_id) {
            match self.try_direct(peer.addr, message).await {
                Ok(response) => return Ok(response),
                Err(e) => warn!("[P2P] Direct to {} failed ({}), trying relay", node_id, e),
            }
        }

        // Fall back to coordinator relay
        match (self.coordinator_addr, self.coordinator_node_id.clone()) {
            (Some(coord), Some(coord_id)) => {
                coordinator::relay_message(coord, &coord_id, &self.node, node_id, message).await
            }
            _ => Err(crate::error::Np2pError::Network(format!(
                "Peer {} unreachable and no coordinator configured (coordinator_node_id required)",
                node_id
            ))),
        }
    }

    async fn try_direct(&self, addr: SocketAddr, message: &Message) -> Result<Message> {
        use crate::network::protocol::Protocol;
        let conn = self.connect_to_addr(addr).await?;
        let (mut send, mut recv) = conn.open_bi().await?;
        Protocol::send(&mut send, message).await?;
        let _ = send.finish();
        Protocol::receive(&mut recv).await
    }

    pub fn node(&self) -> &Node {
        &self.node
    }

    pub fn identity(&self) -> Arc<NodeIdentity> {
        self.identity.clone()
    }
}
