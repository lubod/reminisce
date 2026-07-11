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
    // AI service configuration
    #[serde(default = "default_embedding_service_url", alias = "clip_service_url", alias = "ai_service_url")]
    pub embedding_service_url: String,
    #[serde(default = "default_face_service_url")]
    pub face_service_url: String,
    // P2P daemon connection
    #[serde(default)]
    pub p2p_daemon_host: Option<String>,
    #[serde(default)]
    pub p2p_daemon_port: Option<u16>,
    #[serde(skip)]
    pub enable_media_backup: Arc<AtomicBool>,
    #[serde(default)]
    pub external_ip: Option<String>,
    #[serde(default)]
    pub allowed_import_dirs: Option<Vec<String>>,

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

    // Observability configuration
    #[serde(default)]
    pub otlp_endpoint: Option<String>,
    #[serde(default)]
    pub environment: Option<String>,

    // Relay configuration for home-server discovery
    #[serde(default)]
    pub relay_url: Option<String>,
    #[serde(default)]
    pub relay_api_key: Option<String>,
    #[serde(default)]
    pub relay_username: Option<String>,
    #[serde(default)]
    pub relay_password: Option<String>,
    #[serde(default)]
    pub advertise_addr: Option<String>,
    #[serde(default)]
    pub main_server_url: Option<String>,
    #[serde(default = "default_port")]
    pub port: u16,

    #[serde(default = "default_p2p_data_dir")]
    pub p2p_data_dir: String,

    // P2P Storage — dynamic discovery
    /// UDP port to listen on for LAN broadcast announcements from storage nodes.
    #[serde(default = "default_p2p_discovery_port")]
    pub p2p_discovery_port: u16,
    /// Coordinator QUIC address for cross-network peer discovery (e.g. 1.2.3.4:5055).
    #[serde(default)]
    pub p2p_coordinator_addr: Option<String>,
    /// Legacy static peer list — kept for backward compatibility, prefer discovery.
    #[serde(default)]
    pub p2p_peers: Vec<String>,

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
            audit_min_secs: default_audit_min(),
            audit_max_secs: default_audit_max(),
            rebalance_min_secs: default_rebalance_min(),
            rebalance_max_secs: default_rebalance_max(),
            verification_min_secs: default_verification_min(),
            verification_max_secs: default_verification_max(),
        }
    }
}

fn default_ai_min() -> u64 { 5 }
fn default_ai_max() -> u64 { 30 }

fn default_duplicate_min() -> u64 { 200 }
fn default_duplicate_max() -> u64 { 300 }

fn default_replication_min() -> u64 { 10 }
fn default_replication_max() -> u64 { 60 }

fn default_audit_min() -> u64 { 60 }
fn default_audit_max() -> u64 { 3600 }

fn default_rebalance_min() -> u64 { 120 }
fn default_rebalance_max() -> u64 { 3600 }

fn default_verification_min() -> u64 { 1 }
fn default_verification_max() -> u64 { 10 }

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

fn default_embedding_service_url() -> String {
    "http://localhost:8081".to_string()
}

fn default_face_service_url() -> String {
    "http://localhost:8081".to_string()  // Consolidated with embedding service
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

fn default_p2p_namespace() -> String {
    "default".to_string()
}

impl Config {
    pub fn from_file<P: AsRef<Path>>(path: P) -> Result<Self, Box<dyn std::error::Error>> {
        let contents = fs::read_to_string(path)?;
        let mut config: Config = serde_yaml::from_str(&contents)?;

        // Validate API key at startup
        config.get_api_key().map_err(|e| std::io::Error::new(std::io::ErrorKind::Other, e))?;

        // Initialize AI processing settings with defaults
        config.enable_ai_descriptions = Arc::new(AtomicBool::new(true));
        config.enable_embeddings = Arc::new(AtomicBool::new(true));
        config.embedding_parallel_count = Arc::new(AtomicUsize::new(10));
        config.enable_face_detection = Arc::new(AtomicBool::new(true));
        config.face_detection_parallel_count = Arc::new(AtomicUsize::new(10));
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
