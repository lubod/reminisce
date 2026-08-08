//! Shared registry type aliases for the coordinator submodules.

use std::collections::HashMap;
use std::sync::{Arc, RwLock};

/// Maps node_id → QUIC connection kept alive by the home server.
pub type TunnelMap = Arc<RwLock<HashMap<String, quinn::Connection>>>;

/// Maps node_id → persistent reverse QUIC connection from a storage node behind NAT.
pub type ChannelMap = Arc<RwLock<HashMap<String, quinn::Connection>>>;

