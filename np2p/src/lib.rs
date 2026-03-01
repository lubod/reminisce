pub mod error;
pub mod crypto;
pub mod storage;
pub mod network;

pub use error::{Np2pError, Result};

/// Version of the np2p protocol
pub const PROTOCOL_VERSION: &str = "0.1.0";
