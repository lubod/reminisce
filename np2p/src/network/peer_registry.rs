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

/// Mesh admission control applied at the single choke point every peer enters
/// through (`upsert`): LAN discovery, coordinator syncs, and any other source.
#[derive(Debug, Clone, Default)]
pub enum AdmissionPolicy {
    /// Any signed announcement may join (legacy behavior).
    #[default]
    Open,
    /// Only these node IDs (64-hex) may join; others are refused.
    AllowList(Vec<String>),
}

impl AdmissionPolicy {
    pub fn allows(&self, node_id: &str) -> bool {
        match self {
            AdmissionPolicy::Open => true,
            AdmissionPolicy::AllowList(list) => list.iter().any(|id| id == node_id),
        }
    }
}

/// How long a freshly-seen LAN address holds primary priority before yielding
/// to a newer Tailscale/CGNAT announcement when LAN goes silent.
const LAN_PRIMARY_HOLD_SECS: u64 = 60;

#[derive(Default)]
struct RegistryState {
    peers: HashMap<String, PeerInfo>,
    primary_seen: HashMap<String, Instant>,
    known_addrs: HashMap<SocketAddr, (String, Instant)>,
}

/// Thread-safe map of discovered peers, keyed by node_id hex string.
#[derive(Clone, Default)]
pub struct PeerRegistry {
    state: Arc<RwLock<RegistryState>>,
    admission: Arc<AdmissionPolicy>,
}

fn ip_tier(ip: std::net::IpAddr) -> u8 {
    match ip {
        std::net::IpAddr::V4(v4) => {
            let octets = v4.octets();
            let is_cgnat = octets[0] == 100 && (octets[1] & 0b1100_0000) == 64;
            if v4.is_private() || v4.is_loopback() || v4.is_link_local() {
                2
            } else if is_cgnat {
                1
            } else {
                0
            }
        }
        std::net::IpAddr::V6(v6) => {
            let segs = v6.segments();
            let is_tailscale_v6 = segs[0] == 0xfd7a && segs[1] == 0x115c && segs[2] == 0xa1e0;
            let is_ula = (segs[0] & 0xfe00) == 0xfc00;
            let is_link_local = (segs[0] & 0xffc0) == 0xfe80;
            if is_tailscale_v6 {
                1
            } else if v6.is_loopback() || is_ula || is_link_local {
                2
            } else {
                0
            }
        }
    }
}

impl PeerRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    /// Registry that refuses to admit node IDs outside `allowed`.
    pub fn with_allow_list(allowed: Vec<String>) -> Self {
        Self {
            state: Arc::new(RwLock::new(RegistryState::default())),
            admission: Arc::new(AdmissionPolicy::AllowList(allowed)),
        }
    }

    pub fn admission(&self) -> &AdmissionPolicy {
        &self.admission
    }

    pub fn upsert(&self, node_id: String, addr: SocketAddr) {
        if !self.admission.allows(&node_id) {
            tracing::warn!(
                "[REGISTRY] Refused unlisted peer {} at {} (admission allow-list active)",
                node_id, addr
            );
            return;
        }
        let now = Instant::now();
        let mut state = self.state.write().unwrap_or_else(|e| e.into_inner());
        state.known_addrs.insert(addr, (node_id.clone(), now));

        let replace_primary = match state.peers.get(&node_id) {
            Some(existing) => {
                let existing_tier = ip_tier(existing.addr.ip());
                let new_tier = ip_tier(addr.ip());
                if existing.addr == addr || new_tier >= existing_tier {
                    true
                } else if new_tier == 0 {
                    // Never overwrite a private or Tailscale primary IP with a public one.
                    false
                } else {
                    // Tailscale vs LAN: keep fresh LAN primary to avoid 10s oscillation,
                    // failing over automatically if LAN went silent.
                    state
                        .primary_seen
                        .get(&node_id)
                        .is_none_or(|t| now.duration_since(*t).as_secs() >= LAN_PRIMARY_HOLD_SECS)
                }
            }
            None => true,
        };

        if let Some(existing) = state.peers.get_mut(&node_id) {
            existing.last_seen = now;
            if replace_primary {
                existing.addr = addr;
                state.primary_seen.insert(node_id, now);
            }
            return;
        }

        state.primary_seen.insert(node_id.clone(), now);
        state.peers.insert(
            node_id.clone(),
            PeerInfo {
                node_id,
                addr,
                last_seen: now,
            },
        );
    }

    pub fn remove_stale(&self, timeout_secs: u64) {
        let mut state = self.state.write().unwrap_or_else(|e| e.into_inner());
        let RegistryState {
            peers,
            primary_seen,
            known_addrs,
        } = &mut *state;
        peers.retain(|_, p| p.last_seen.elapsed().as_secs() < timeout_secs);
        known_addrs.retain(|_, (nid, seen)| {
            seen.elapsed().as_secs() < timeout_secs && peers.contains_key(nid)
        });
        primary_seen.retain(|nid, _| peers.contains_key(nid));
    }

    pub fn get(&self, node_id: &str) -> Option<PeerInfo> {
        self.state.read().unwrap_or_else(|e| e.into_inner()).peers.get(node_id).cloned()
    }

    /// Reverse lookup: return the node_id registered at `addr`, if any.
    /// Matches both the current primary address and any recently verified alternate interface address.
    pub fn find_by_addr(&self, addr: SocketAddr) -> Option<String> {
        let state = self.state.read().unwrap_or_else(|e| e.into_inner());
        if let Some((node_id, _)) = state.known_addrs.get(&addr) {
            return Some(node_id.clone());
        }
        state
            .peers
            .iter()
            .find(|(_, p)| p.addr == addr)
            .map(|(node_id, _)| node_id.clone())
    }

    pub fn all(&self) -> Vec<PeerInfo> {
        self.state.read().unwrap_or_else(|e| e.into_inner()).peers.values().cloned().collect()
    }

    pub fn len(&self) -> usize {
        self.state.read().unwrap_or_else(|e| e.into_inner()).peers.len()
    }

    pub fn is_empty(&self) -> bool {
        self.state.read().unwrap_or_else(|e| e.into_inner()).peers.is_empty()
    }
}
