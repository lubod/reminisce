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

    /// Builds the shared QUIC transport settings used for both server and client
    /// roles (see the comment in `from_socket` for the chosen values).
    fn hardened_transport() -> quinn::TransportConfig {
        let mut transport = quinn::TransportConfig::default();
        transport.keep_alive_interval(Some(Duration::from_secs(15)));
        transport.max_idle_timeout(Some(Duration::from_secs(300).try_into().unwrap()));
        transport.stream_receive_window(quinn::VarInt::from_u32(4 * 1024 * 1024));
        transport.receive_window(quinn::VarInt::from_u32(4 * 1024 * 1024));
        transport.max_concurrent_bidi_streams(quinn::VarInt::from_u32(32));
        transport.max_concurrent_uni_streams(quinn::VarInt::from_u32(0));
        transport
    }

    /// Creates a node from an existing UDP socket.
    pub fn from_socket(socket: std::net::UdpSocket, identity: NodeIdentity) -> Result<Self> {
        let (mut server_config, mut client_config) = identity.generate_tls_config()?;

        // Keep long-lived connections (e.g. tunnel) alive with QUIC PING frames.
        // idle timeout 300s so large segmented uploads (encrypt/erasure-code of each
        // 256MB segment can pause streaming >60s under CPU/disk load) don't get
        // silently dropped mid-stream — previously caused "0/5 shards stored" for
        // big files even though the sender reported success.
        //
        // Resource caps (apply to both roles):
        //  * 4 MiB per-stream and per-connection flow-control receive windows bound
        //    how much unread data a misbehaving peer can make us buffer.
        //  * max_concurrent_bidi_streams(32) bounds concurrent inbound streams;
        //    uni streams are refused entirely (the protocol is bidi-only).
        client_config.transport_config(Arc::new(Self::hardened_transport()));

        // The SERVER side previously ran on quinn's default TransportConfig with no
        // caps at all — incoming connections could buffer/open far more.
        server_config.transport_config(Arc::new(Self::hardened_transport()));

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

    /// Connects to a remote node, verifying its certificate against `server_name`,
    /// which must be the peer's 64-hex Node ID (identity binding is mandatory).
    pub async fn connect(&self, addr: SocketAddr, server_name: &str) -> Result<Connection> {
        let sni = crate::crypto::sni_for_node_id(server_name)?;
        let conn = self.endpoint.connect(addr, &sni)?.await?;
        Ok(conn)
    }

    /// Accepts an incoming connection.
    pub async fn accept(&self) -> Option<quinn::Incoming> {
        self.endpoint.accept().await
    }
}
