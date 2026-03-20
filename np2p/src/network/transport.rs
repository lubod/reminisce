use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;
use quinn::{Endpoint, Connection};
use crate::error::Result;
use crate::crypto::NodeIdentity;

/// Represents a network node in the np2p network.
/// Handles QUIC connections and peer identity verification.
#[derive(Clone)]
pub struct Node {
    endpoint: Endpoint,
    #[allow(dead_code)]
    identity: Arc<NodeIdentity>,
}

impl Node {
    /// Starts a new node bound to the given address.
    pub fn new(addr: SocketAddr, identity: NodeIdentity) -> Result<Self> {
        let socket = std::net::UdpSocket::bind(addr)?;
        Self::from_socket(socket, identity)
    }

    /// Creates a node from an existing UDP socket.
    pub fn from_socket(socket: std::net::UdpSocket, identity: NodeIdentity) -> Result<Self> {
        let (server_config, mut client_config) = identity.generate_tls_config()?;

        // Keep long-lived connections (e.g. tunnel) alive with QUIC PING frames
        let mut transport = quinn::TransportConfig::default();
        transport.keep_alive_interval(Some(Duration::from_secs(15)));
        transport.max_idle_timeout(Some(Duration::from_secs(60).try_into().unwrap()));
        client_config.transport_config(Arc::new(transport));

        let mut endpoint = Endpoint::new(
            quinn::EndpointConfig::default(),
            Some(server_config),
            socket,
            Arc::new(quinn::TokioRuntime),
        )?;
        endpoint.set_default_client_config(client_config);

        Ok(Self {
            endpoint,
            identity: Arc::new(identity),
        })
    }

    pub fn local_addr(&self) -> Result<SocketAddr> {
        Ok(self.endpoint.local_addr()?)
    }

    /// Connects to a remote node.
    pub async fn connect(&self, addr: SocketAddr) -> Result<Connection> {
        let conn = self.endpoint.connect(addr, "reminisce")?.await?;
        Ok(conn)
    }

    /// Accepts an incoming connection.
    pub async fn accept(&self) -> Option<quinn::Incoming> {
        self.endpoint.accept().await
    }
}
