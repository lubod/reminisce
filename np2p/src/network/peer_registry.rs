//! In-memory peer registry mapping node IDs to socket addresses.
//!
//! Prefers LAN (private) addresses over public ones when both are known.
//! Supports TTL-based eviction via remove_stale(). Thread-safe via RwLock.
//! Populated by LAN UDP discovery broadcasts and coordinator peer syncs.

use std::collections::HashMap;
use std::net::SocketAddr;
use std::sync::{Arc, RwLock};
use std::time::Instant;

#[derive(Debug, Clone)]
pub struct PeerInfo {
    pub node_id: String,
    pub addr: SocketAddr,
    pub last_seen: Instant,
}

/// Thread-safe map of discovered peers, keyed by node_id hex string.
#[derive(Clone, Default)]
pub struct PeerRegistry {
    peers: Arc<RwLock<HashMap<String, PeerInfo>>>,
}

fn is_private_ip(ip: std::net::IpAddr) -> bool {
    match ip {
        std::net::IpAddr::V4(v4) => v4.is_private() || v4.is_loopback() || v4.is_link_local(),
        std::net::IpAddr::V6(v6) => v6.is_loopback(),
    }
}

impl PeerRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn upsert(&self, node_id: String, addr: SocketAddr) {
        let mut peers = self.peers.write().unwrap();
        if let Some(existing) = peers.get_mut(&node_id) {
            if is_private_ip(existing.addr.ip()) && !is_private_ip(addr.ip()) {
                // Keep the working LAN address; only refresh last_seen so the
                // entry does not expire while the coordinator keeps pinging.
                existing.last_seen = Instant::now();
                return;
            }
        }
        peers.insert(
            node_id.clone(),
            PeerInfo {
                node_id,
                addr,
                last_seen: Instant::now(),
            },
        );
    }

    pub fn remove_stale(&self, timeout_secs: u64) {
        let mut peers = self.peers.write().unwrap();
        peers.retain(|_, p| p.last_seen.elapsed().as_secs() < timeout_secs);
    }

    pub fn get(&self, node_id: &str) -> Option<PeerInfo> {
        self.peers.read().unwrap().get(node_id).cloned()
    }

    pub fn all(&self) -> Vec<PeerInfo> {
        self.peers.read().unwrap().values().cloned().collect()
    }

    pub fn len(&self) -> usize {
        self.peers.read().unwrap().len()
    }

    pub fn is_empty(&self) -> bool {
        self.peers.read().unwrap().is_empty()
    }
}
