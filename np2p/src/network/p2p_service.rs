use std::net::SocketAddr;
use std::sync::Arc;
use crate::error::Result;
use crate::crypto::NodeIdentity;
use crate::network::transport::Node;
use tokio::time::Duration;
use tracing::{info, error};

pub struct P2PService {
    node: Node,
    identity: Arc<NodeIdentity>,
}

impl P2PService {
    pub async fn new(listen_addr: SocketAddr, identity: NodeIdentity) -> Result<Self> {
        let std_socket = std::net::UdpSocket::bind(listen_addr)?;
        std_socket.set_nonblocking(true)?;
        let node = Node::from_socket(std_socket, identity.clone())?;

        Ok(Self {
            node,
            identity: Arc::new(identity),
        })
    }

    /// Connects directly to a specific socket address (useful for overlay networks like NetBird).
    pub async fn connect_to_addr(&self, addr: SocketAddr) -> Result<quinn::Connection> {
        info!("[P2P] Direct connection attempt to {}", addr);
        let res = tokio::time::timeout(Duration::from_secs(10), self.node.connect(addr)).await;
        match res {
            Ok(Ok(conn)) => {
                info!("[P2P] Direct connection established with {}", addr);
                Ok(conn)
            }
            Ok(Err(e)) => {
                error!("[P2P] Direct connection to {} failed: {}", addr, e);
                Err(e.into())
            }
            Err(_) => {
                error!("[P2P] Direct connection to {} timed out", addr);
                Err(crate::error::Np2pError::Network(format!("Timeout connecting to {}", addr)))
            }
        }
    }

    pub fn node(&self) -> &Node {
        &self.node
    }

    pub fn identity(&self) -> Arc<NodeIdentity> {
        self.identity.clone()
    }
}
