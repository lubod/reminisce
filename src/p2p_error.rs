//! Typed errors for the P2P shard-client helpers.
//!
//! Consolidates the ad-hoc `String` / `Box<dyn Error>` errors previously threaded
//! through `p2p_upload` into a single `thiserror` enum, so callers can match on
//! specific failure modes (notably disk-full rejections) without parsing strings.

use thiserror::Error;

/// Errors produced by the shared shard store / retrieve / upload helpers.
#[derive(Debug, Error)]
pub enum P2pError {
    /// Could not establish or authenticate a QUIC connection to a peer.
    #[error("connection to {addr} failed: {err}")]
    Connect { addr: String, err: String },

    /// Opening a bidirectional stream on the peer connection failed.
    #[error("open_bi to {addr} failed: {message}")]
    OpenBidirectional { addr: String, message: String },

    /// A request could not be serialized / sent over the wire.
    #[error("send failed: {message}")]
    Send { message: String },

    /// A response could not be read from the wire.
    #[error("receive failed: {message}")]
    Receive { message: String },

    /// The peer returned a response we did not expect at this stage.
    #[error("unexpected protocol response: {message}")]
    UnexpectedResponse { message: String },

    /// The requested shard is not present on the node (or was refused).
    #[error("shard not found / rejected by node")]
    ShardNotFound,

    /// The shard hash string was not valid 32-byte hex.
    #[error("invalid shard hash `{hash}`: {message}")]
    InvalidShardHash { hash: String, message: String },
}

impl From<P2pError> for String {
    fn from(e: P2pError) -> Self {
        e.to_string()
    }
}

impl From<P2pError> for std::io::Error {
    fn from(e: P2pError) -> Self {
        std::io::Error::other(e)
    }
}

impl P2pError {
    /// Convenient shorthand used where a generic boxed error is returned.
    pub fn connect(addr: String, source: impl Into<String>) -> Self {
        P2pError::Connect { addr, err: source.into() }
    }

    /// Convenient shorthand for an open_bi failure.
    pub fn open_bi(addr: String, message: impl Into<String>) -> Self {
        P2pError::OpenBidirectional { addr, message: message.into() }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn error_variants_display() {
        let connect = P2pError::connect("127.0.0.1:5000".into(), "refused");
        assert!(connect.to_string().contains("127.0.0.1:5000"));

        let not_found = P2pError::ShardNotFound;
        assert!(not_found.to_string().contains("not found"));

        let hash = P2pError::InvalidShardHash { hash: "zz".into(), message: "hex error".into() };
        assert!(hash.to_string().contains("zz"));
    }

    #[test]
    fn p2p_error_converts_to_io_error() {
        let io: std::io::Error = P2pError::ShardNotFound.into();
        assert_eq!(io.kind(), std::io::ErrorKind::Other);
    }
}

