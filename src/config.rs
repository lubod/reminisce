use serde::{Deserialize, Serialize};
use std::fs;
use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicUsize};

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct Config {
    #[serde(default)]
    pub database_url: Option<String>,
    #[serde(default = "default_geotagging_database_url")]
    pub geotagging_database_url: String,
    #[serde(default)]
    pub api_secret_key: Option<String>,
    #[serde(default)]
    pub images_dir: Option<String>,
    #[serde(default)]
    pub videos_dir: Option<String>,
    // Geocoding configuration
    #[serde(default = "default_enable_local_geocoding")]
    pub enable_local_geocoding: bool,
    #[serde(default = "default_enable_external_geocoding_fallback")]
    pub enable_external_geocoding_fallback: bool,
    // AI service configuration (gRPC; legacy HTTP URLs removed)
    #[serde(default = "default_ai_grpc_url")]
    pub ai_grpc_url: String,
    #[serde(skip)]
    pub enable_media_backup: Arc<AtomicBool>,
    #[serde(default)]
    pub allowed_import_dirs: Option<Vec<String>>,
    // CORS allowlist for browser clients. Empty (default) = same-origin only;
    // the SPA behind nginx never needs cross-origin access.
    #[serde(default)]
    pub cors_allowed_origins: Vec<String>,
    // Peer IPs whose X-Forwarded-For/X-Real-IP headers may override the
    // rate-limit key (e.g. the nginx host). Loopback is always trusted.
    #[serde(default)]
    pub rate_limit_trusted_proxies: Vec<String>,
    // P2P mesh admission control: storage-node IDs (64-hex, from each node's
    // startup log) allowed to receive shards. Empty = open admission (legacy).
    #[serde(default)]
    pub p2p_allowed_node_ids: Vec<String>,

    // Database connection pool configuration
    #[serde(default = "default_db_tls")]
    pub db_tls: bool,
    #[serde(default = "default_db_pool_max_size")]
    pub db_pool_max_size: usize,
    #[serde(default = "default_db_pool_min_size")]
    pub db_pool_min_size: usize,
    #[serde(default = "default_db_pool_timeout_secs")]
    pub db_pool_timeout_secs: u64,
    // AI processing settings (runtime configurable)
    #[serde(skip)]
    pub enable_ai_descriptions: Arc<AtomicBool>,
    #[serde(skip)]
    pub enable_embeddings: Arc<AtomicBool>,
    #[serde(skip)]
    pub embedding_parallel_count: Arc<AtomicUsize>,
    // Face detection settings (runtime configurable)
    #[serde(skip)]
    pub enable_face_detection: Arc<AtomicBool>,
    #[serde(skip)]
    pub face_detection_parallel_count: Arc<AtomicUsize>,
    // Orientation detection settings (runtime configurable)
    #[serde(skip)]
    pub enable_orientation_detection: Arc<AtomicBool>,
    #[serde(skip)]
    pub orientation_detection_parallel_count: Arc<AtomicUsize>,

    // Observability configuration (in-app; no external stack)
    #[serde(default)]
    pub log_dir: Option<String>,
    /// Where the AI container's HTTP endpoint lives (e.g. http://ai-server:8081),
    /// used to proxy GPU metrics into `/api/admin/gpu`.
    #[serde(default)]
    pub ai_http_url: Option<String>,
    #[serde(default)]
    pub environment: Option<String>,

    #[serde(default = "default_port")]
    pub port: u16,

    #[serde(default = "default_p2p_data_dir")]
    pub p2p_data_dir: String,

    /// Derive the P2P node identity deterministically from `api_secret_key` instead
    /// of a random `node.key` file. The node_id then becomes recoverable from the
    /// master secret alone — a prerequisite for true full-disk-loss recovery.
    /// NOTE: enabling changes the node_id; storage nodes must (re)pin this node_id
    /// as their authorized owner. Intended for new setups or after re-registering nodes.
    #[serde(default)]
    pub p2p_deterministic_identity: bool,

    // P2P Storage — dynamic discovery
    /// UDP port to listen on for LAN broadcast announcements from storage nodes.
    #[serde(default = "default_p2p_discovery_port")]
    pub p2p_discovery_port: u16,
    /// Coordinator QUIC address for cross-network peer discovery (e.g. 1.2.3.4:5055).
    #[serde(default)]
    pub p2p_coordinator_addr: Option<String>,
    /// Coordinator's 64-hex Node ID (printed in the coordinator startup log).
    /// Required when `p2p_coordinator_addr` is set — it is bound to the QUIC connection
    /// so a spoofed "coordinator" cannot impersonate the real one.
    #[serde(default)]
    pub p2p_coordinator_node_id: Option<String>,

    // Reverse tunnel — lets Android reach the home server through the VPS coordinator
    /// Local port to expose through the coordinator tunnel (e.g. 28444 for nginx HTTPS).
    #[serde(default)]
    pub p2p_tunnel_local_port: Option<u16>,
    /// Public URL that Android uses to reach this server via the tunnel
    /// (e.g. https://vps-ip:8443). Included in the QR code.
    #[serde(default)]
    pub p2p_tunnel_public_url: Option<String>,

    /// Namespace used when registering with / querying the coordinator.
    /// Use different values for dev and production to avoid cross-contamination.
    #[serde(default = "default_p2p_namespace")]
    pub p2p_namespace: String,

    #[serde(default = "default_p2p_data_shards")]
    pub p2p_data_shards: usize,
    #[serde(default = "default_p2p_parity_shards")]
    pub p2p_parity_shards: usize,

    #[serde(default)]
    pub workers: WorkerConfig,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct WorkerConfig {
    #[serde(default = "default_ai_min")]
    pub ai_min_secs: u64,
    #[serde(default = "default_ai_max")]
    pub ai_max_secs: u64,
    
    #[serde(default = "default_duplicate_min")]
    pub duplicate_min_millis: u64,
    #[serde(default = "default_duplicate_max")]
    pub duplicate_max_secs: u64,

    #[serde(default = "default_replication_min")]
    pub replication_min_secs: u64,
    #[serde(default = "default_replication_max")]
    pub replication_max_secs: u64,
    /// How many media files to shard+replicate per worker cycle.
    #[serde(default = "default_replication_batch_size")]
    pub replication_batch_size: i64,

    #[serde(default = "default_audit_min")]
    pub audit_min_secs: u64,
    #[serde(default = "default_audit_max")]
    pub audit_max_secs: u64,

    #[serde(default = "default_rebalance_min")]
    pub rebalance_min_secs: u64,
    #[serde(default = "default_rebalance_max")]
    pub rebalance_max_secs: u64,

    #[serde(default = "default_verification_min")]
    pub verification_min_secs: u64,
    #[serde(default = "default_verification_max")]
    pub verification_max_secs: u64,

    #[serde(default = "default_db_backup_interval")]
    pub db_backup_interval_secs: u64,
    #[serde(default = "default_db_backup_retention")]
    pub db_backup_retention_count: i64,
    #[serde(default = "default_db_backup_enabled")]
    pub db_backup_enabled: bool,
}

impl Default for WorkerConfig {
    fn default() -> Self {
        Self {
            ai_min_secs: default_ai_min(),
            ai_max_secs: default_ai_max(),
            duplicate_min_millis: default_duplicate_min(),
            duplicate_max_secs: default_duplicate_max(),
            replication_min_secs: default_replication_min(),
            replication_max_secs: default_replication_max(),
            replication_batch_size: default_replication_batch_size(),
            audit_min_secs: default_audit_min(),
            audit_max_secs: default_audit_max(),
            rebalance_min_secs: default_rebalance_min(),
            rebalance_max_secs: default_rebalance_max(),
            verification_min_secs: default_verification_min(),
            verification_max_secs: default_verification_max(),
            db_backup_interval_secs: default_db_backup_interval(),
            db_backup_retention_count: default_db_backup_retention(),
            db_backup_enabled: default_db_backup_enabled(),
        }
    }
}

fn default_ai_min() -> u64 { 5 }
fn default_ai_max() -> u64 { 30 }

fn default_duplicate_min() -> u64 { 200 }
fn default_duplicate_max() -> u64 { 300 }

fn default_replication_min() -> u64 { 5 }
fn default_replication_max() -> u64 { 20 }
fn default_replication_batch_size() -> i64 { 50 }

fn default_audit_min() -> u64 { 60 }
fn default_audit_max() -> u64 { 3600 }

fn default_rebalance_min() -> u64 { 5 }
fn default_rebalance_max() -> u64 { 3600 }

fn default_verification_min() -> u64 { 1 }
fn default_verification_max() -> u64 { 10 }

fn default_db_backup_interval() -> u64 { 86400 } // 24 hours
fn default_db_backup_retention() -> i64 { 7 } // keep last 7 snapshots
fn default_db_backup_enabled() -> bool { true }

fn default_p2p_data_shards() -> usize { 3 }
fn default_p2p_parity_shards() -> usize { 2 }

fn default_port() -> u16 {
    8080
}

fn default_geotagging_database_url() -> String {
    "postgres://postgres:postgres@geotagging-db:5432/geotagging_db".to_string()
}

fn default_enable_local_geocoding() -> bool {
    true
}

fn default_enable_external_geocoding_fallback() -> bool {
    true
}

fn default_ai_grpc_url() -> String {
    "http://localhost:50051".to_string()
}

fn default_db_tls() -> bool {
    false
}

fn default_db_pool_max_size() -> usize {
    50
}

fn default_db_pool_min_size() -> usize {
    10
}

fn default_db_pool_timeout_secs() -> u64 {
    30
}

fn default_p2p_data_dir() -> String {
    "data/p2p".to_string()
}

fn default_p2p_discovery_port() -> u16 {
    5066
}

/// Warn (loudly) if a config file that contains the master secret is readable by
/// group or others. Only relevant on Unix; a no-op elsewhere.
#[cfg(unix)]
fn warn_if_config_world_readable(path: &Path) {
    use std::os::unix::fs::PermissionsExt;
    if let Ok(meta) = fs::metadata(path) {
        let mode = meta.permissions().mode() & 0o777;
        if mode & 0o077 != 0 {
            log::warn!(
                "⚠️  SECURITY: {:?} is mode {:o} and holds api_secret_key — readable by group/others. \
                 Run `chmod 600 {:?}` or inject the secret via the API_SECRET_KEY environment variable instead.",
                path, mode, path
            );
        }
    }
}

#[cfg(not(unix))]
fn warn_if_config_world_readable(_path: &Path) {}

fn default_p2p_namespace() -> String {
    "default".to_string()
}

impl Config {
    pub fn from_file<P: AsRef<Path>>(path: P) -> Result<Self, Box<dyn std::error::Error>> {
        let path_ref = path.as_ref();
        let contents = fs::read_to_string(path_ref)?;
        let mut config: Config = serde_yaml::from_str(&contents)?;

        // The API_SECRET_KEY environment variable takes precedence over the config
        // file, so the master secret never has to live in a plaintext file.
        let env_secret = std::env::var("API_SECRET_KEY").ok().filter(|s| !s.is_empty());
        if let Some(secret) = env_secret {
            config.api_secret_key = Some(secret);
        } else {
            // Secret comes from the config file — warn if it's readable by others.
            warn_if_config_world_readable(path_ref);
        }

        // Validate API key at startup
        config.get_api_key().map_err(std::io::Error::other)?;

        // Initialize AI processing settings with defaults
        config.enable_ai_descriptions = Arc::new(AtomicBool::new(true));
        config.enable_embeddings = Arc::new(AtomicBool::new(true));
        config.embedding_parallel_count = Arc::new(AtomicUsize::new(10));
        config.enable_face_detection = Arc::new(AtomicBool::new(true));
        config.face_detection_parallel_count = Arc::new(AtomicUsize::new(10));
        config.enable_orientation_detection = Arc::new(AtomicBool::new(true));
        config.orientation_detection_parallel_count = Arc::new(AtomicUsize::new(10));
        config.enable_media_backup = Arc::new(AtomicBool::new(false));
        Ok(config)
    }

    pub fn get_api_key(&self) -> Result<&str, &'static str> {
        self.api_secret_key.as_deref()
            .filter(|s| !s.is_empty())
            .ok_or("API secret key is not configured — authentication is disabled")
    }

    pub fn get_images_dir(&self) -> &str {
        self.images_dir.as_deref().unwrap_or("uploaded_images")
    }

    pub fn get_videos_dir(&self) -> &str {
        self.videos_dir.as_deref().unwrap_or("uploaded_videos")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::Ordering;

    fn temp_dir(name: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!("reminisce_cfg_{}_{}", name, std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    /// Whether the CI/deploy environment already exports API_SECRET_KEY. Config
    /// intentionally lets env always win over the file, so tests must not assume
    /// the env var is absent (it is set in the deploy environment).
    fn ambient_env_secret() -> bool {
        std::env::var("API_SECRET_KEY").map(|s| !s.is_empty()).unwrap_or(false)
    }

    fn min_yaml(secret: &str) -> String {
        format!("api_secret_key: \"{}\"\n", secret)
    }

    #[test]
    fn from_file_minimal_config_applies_defaults() {
        let dir = temp_dir("minimal");
        let path = dir.join("config.yaml");
        let secret = "unit-test-secret-0123456789abcdef0123456789abcdef";
        std::fs::write(&path, min_yaml(secret)).unwrap();

        let cfg = Config::from_file(&path).expect("minimal config should parse");
        if ambient_env_secret() {
            assert_ne!(cfg.get_api_key().unwrap(), "");
        } else {
            assert_eq!(cfg.get_api_key().unwrap(), secret);
        }
        assert_eq!(cfg.port, 8080);
        assert_eq!(cfg.db_pool_max_size, 50);
        assert_eq!(cfg.db_pool_min_size, 10);
        assert_eq!(cfg.db_pool_timeout_secs, 30);
        assert!(!cfg.db_tls);
        assert_eq!(cfg.ai_grpc_url, "http://localhost:50051");
        assert!(cfg.enable_local_geocoding);
        assert_eq!(cfg.get_images_dir(), "uploaded_images");
        assert_eq!(cfg.get_videos_dir(), "uploaded_videos");
        assert_eq!(cfg.p2p_data_shards, 3);
        assert_eq!(cfg.p2p_parity_shards, 2);
        assert!(cfg.enable_ai_descriptions.load(Ordering::Relaxed));
        assert!(!cfg.enable_media_backup.load(Ordering::Relaxed));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn from_file_respects_explicit_fields() {
        let dir = temp_dir("explicit");
        let path = dir.join("config.yaml");
        let yaml = r#"
api_secret_key: "k-abcdef0123456789abcdef0123456789abcdef0123"
port: 9090
database_url: "postgres://u:p@h:5432/db"
db_pool_max_size: 8
images_dir: "/data/images"
videos_dir: "/data/videos"
p2p_data_shards: 5
p2p_deterministic_identity: true
"#;
        std::fs::write(&path, yaml).unwrap();
        let cfg = Config::from_file(&path).expect("config should parse");
        assert_eq!(cfg.port, 9090);
        if ambient_env_secret() {
            assert_ne!(cfg.get_api_key().unwrap(), "");
        } else {
            assert_eq!(cfg.get_api_key().unwrap(), "k-abcdef0123456789abcdef0123456789abcdef0123");
        }
        assert_eq!(cfg.database_url.as_deref(), Some("postgres://u:p@h:5432/db"));
        assert_eq!(cfg.db_pool_max_size, 8);
        assert_eq!(cfg.get_images_dir(), "/data/images");
        assert_eq!(cfg.get_videos_dir(), "/data/videos");
        assert_eq!(cfg.p2p_data_shards, 5);
        assert!(cfg.p2p_deterministic_identity);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn from_file_missing_file_or_secret_is_an_error() {
        let dir = temp_dir("missing");
        let missing = dir.join("does-not-exist.yaml");
        assert!(Config::from_file(&missing).is_err(), "missing file must error");

        let empty = dir.join("empty.yaml");
        std::fs::write(&empty, "port: 8080\n").unwrap();
        let empty_result = Config::from_file(&empty);
        if ambient_env_secret() {
            assert!(empty_result.is_ok(), "ambient env secret satisfies from_file");
        } else {
            assert!(empty_result.is_err(), "missing api_secret_key must error");
        }

        let blank = dir.join("blank.yaml");
        std::fs::write(&blank, "api_secret_key: \"\"\n").unwrap();
        let blank_result = Config::from_file(&blank);
        if ambient_env_secret() {
            assert!(blank_result.is_ok(), "ambient env secret satisfies from_file");
        } else {
            assert!(blank_result.is_err(), "empty api_secret_key must error");
        }
        let _ = std::fs::remove_dir_all(&dir);
    }

}

