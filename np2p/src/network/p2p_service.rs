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
        })
    }

    /// Connect directly to a known socket address.
    pub async fn connect_to_addr(&self, addr: SocketAddr) -> Result<quinn::Connection> {
        info!("[P2P] Connecting to {}", addr);
        let res = tokio::time::timeout(Duration::from_secs(10), self.node.connect(addr)).await;
        match res {
            Ok(Ok(conn)) => {
                info!("[P2P] Connected to {}", addr);
                Ok(conn)
            }
            Ok(Err(e)) => {
                error!("[P2P] Connection to {} failed: {}", addr, e);
                Err(e.into())
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
        self.connect_to_addr(peer.addr).await
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
        match self.coordinator_addr {
            Some(coord) => {
                coordinator::relay_message(coord, &self.node, node_id, message).await
            }
            None => Err(crate::error::Np2pError::Network(format!(
                "Peer {} unreachable and no coordinator configured",
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
