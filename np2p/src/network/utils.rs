use std::net::SocketAddr;
use tokio::net::lookup_host;
use crate::error::{Np2pError, Result};

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
