use std::net::SocketAddr;
use std::time::Duration;
use tokio::net::lookup_host;
use crate::error::{Np2pError, Result};

/// Exponential reconnect backoff with jitter.
///
/// Starts at `initial`, doubles on every failed attempt up to `max`, and applies
/// ±20% jitter so many nodes that lost the same coordinator do not retry in
/// lockstep (thundering herd). Reset to the initial delay after a successful
/// connect/registration.
pub struct ReconnectBackoff {
    initial_secs: u64,
    max_secs: u64,
    current_secs: u64,
}

impl ReconnectBackoff {
    pub fn new(initial_secs: u64, max_secs: u64) -> Self {
        Self {
            initial_secs: initial_secs.max(1),
            max_secs: max_secs.max(1),
            current_secs: initial_secs.max(1),
        }
    }

    /// Call after a successful connect/registration: next retry starts over at
    /// the initial delay.
    pub fn reset(&mut self) {
        self.current_secs = self.initial_secs;
    }

    /// Jump straight to the maximum delay (e.g. for non-recoverable errors like
    /// protocol version mismatches where retrying sooner cannot help).
    pub fn skip_to_max(&mut self) {
        self.current_secs = self.max_secs;
    }

    /// Returns the delay for the current attempt and advances the sequence
    /// (current → doubled, capped at max), with ±20% uniform jitter applied.
    pub fn next_delay(&mut self) -> Duration {
        let base = self.current_secs;
        self.current_secs = self
            .current_secs
            .saturating_mul(2)
            .min(self.max_secs);
        use rand::Rng;
        let jitter = rand::thread_rng().gen_range(-0.2f64..0.2);
        let secs = ((base as f64) * (1.0 + jitter)).round().max(1.0) as u64;
        Duration::from_secs(secs)
    }
}

/// Resolves a string address (e.g. "localhost:5051" or "127.0.0.1:5051") to a SocketAddr.
pub async fn resolve_addr(addr_str: &str) -> Result<SocketAddr> {
    let addrs = lookup_host(addr_str).await
        .map_err(|e| Np2pError::Network(format!("Failed to resolve {}: {}", addr_str, e)))?;
    
    addrs.into_iter().next()
        .ok_or_else(|| Np2pError::Network(format!("No addresses found for {}", addr_str)))
}

/// Returns a list of local non-loopback IPv4 addresses.
pub fn get_local_addrs() -> Vec<std::net::IpAddr> {
    let mut addrs = Vec::new();
    if let Ok(interfaces) = get_if_addrs::get_if_addrs() {
        for iface in interfaces {
            if !iface.is_loopback() {
                if let std::net::IpAddr::V4(addr) = iface.ip() {
                    addrs.push(std::net::IpAddr::V4(addr));
                }
            }
        }
    }
    addrs
}
