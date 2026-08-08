//! Peer registry: in-memory map + JSON persistence.

use std::collections::HashMap;
use std::io::BufReader;
use std::net::IpAddr;
use std::sync::{Arc, RwLock};
use std::time::Instant;
use tracing::{info, warn};

// ── Peer registry ────────────────────────────────────────────────────────────

pub struct PeerEntry {
    pub node_id: String,
    pub ip: IpAddr,
    pub quic_port: u16,
    pub last_seen: Instant,
}

/// Key: (namespace, node_id)
pub type PeerMap = Arc<RwLock<HashMap<(String, String), PeerEntry>>>;

#[derive(serde::Serialize, serde::Deserialize)]
struct PersistedPeer {
    namespace: String,
    node_id: String,
    ip: String,
    quic_port: u16,
    last_seen_secs: u64,
}

pub fn load_persisted_peers(data_dir: &std::path::Path) -> HashMap<(String, String), PeerEntry> {
    let path = data_dir.join("peers.json");
    if !path.exists() {
        return HashMap::new();
    }
    let file = match std::fs::File::open(&path) {
        Ok(f) => f,
        Err(e) => {
            warn!("[COORD] Failed to open peers.json: {}", e);
            return HashMap::new();
        }
    };
    let reader = BufReader::new(file);
    let list: Vec<PersistedPeer> = match serde_json::from_reader(reader) {
        Ok(l) => l,
        Err(e) => {
            warn!("[COORD] Failed to deserialize peers.json: {}", e);
            return HashMap::new();
        }
    };

    let mut map = HashMap::new();
    let current_secs = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();

    for p in list {
        if let Ok(ip) = p.ip.parse() {
            let elapsed = current_secs.saturating_sub(p.last_seen_secs);
            let last_seen = Instant::now().checked_sub(std::time::Duration::from_secs(elapsed)).unwrap_or(Instant::now());
            map.insert(
                (p.namespace, p.node_id.clone()),
                PeerEntry {
                    node_id: p.node_id,
                    ip,
                    quic_port: p.quic_port,
                    last_seen,
                },
            );
        }
    }
    info!("[COORD] Loaded {} peers from peers.json", map.len());
    map
}

pub fn save_persisted_peers(peers: &PeerMap, data_dir: &std::path::Path) {
    let dir = data_dir.to_path_buf();
    let list: Vec<PersistedPeer> = {
        let map = peers.read().unwrap_or_else(|e| e.into_inner());
        let current_secs = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();

        map.iter().map(|((ns, _), p)| {
            let elapsed = p.last_seen.elapsed().as_secs();
            let last_seen_secs = current_secs.saturating_sub(elapsed);
            PersistedPeer {
                namespace: ns.clone(),
                node_id: p.node_id.clone(),
                ip: p.ip.to_string(),
                quic_port: p.quic_port,
                last_seen_secs,
            }
        }).collect()
    };

    let temp_name = format!("peers.{}.tmp", rand::random::<u64>());
    let temp_path = dir.join(temp_name);
    let target_path = dir.join("peers.json");
    static SAVE_LOCK: std::sync::OnceLock<std::sync::Mutex<()>> = std::sync::OnceLock::new();
    let lock = SAVE_LOCK.get_or_init(|| std::sync::Mutex::new(()));

    tokio::task::spawn_blocking(move || {
        let _guard = lock.lock().unwrap_or_else(|e| e.into_inner());
        let write_ok = if let Ok(file) = std::fs::File::create(&temp_path) {
            if let Err(e) = serde_json::to_writer_pretty(file, &list) {
                warn!("[COORD] Failed to write temp peers file: {}", e);
                false
            } else {
                true
            }
        } else {
            false
        };

        if write_ok {
            if let Err(e) = std::fs::rename(&temp_path, &target_path) {
                warn!("[COORD] Failed to rename temp peers file to peers.json: {}", e);
                let _ = std::fs::remove_file(&temp_path);
            }
        }
    });
}

pub fn current_peer_list(peers: &PeerMap, namespace: &str, ttl: u64) -> Vec<(String, String)> {
    peers
        .read()
        .unwrap()
        .iter()
        .filter(|((ns, _), p)| ns == namespace && p.last_seen.elapsed().as_secs() < ttl)
        .map(|(_, p)| (p.node_id.clone(), format!("{}:{}", p.ip, p.quic_port)))
        .collect()
}
