use reminisce::config;
use reminisce::Claims;
use reminisce::db::{MainDbPool, GeotaggingDbPool};
use deadpool_postgres::Pool;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicUsize};

// Used by some integration test crates but not all — appears dead in crates that don't call it.
#[allow(dead_code)]
pub fn create_test_config() -> config::Config {
    config::Config {
        database_url: Some("".to_string()),
        geotagging_database_url: "postgres://postgres:postgres@localhost:5435/geotagging_db".to_string(),
        api_secret_key: Some("test_secret_key_which_is_at_least_32_bytes_long".to_string()),
        images_dir: Some("uploaded_images_test".to_string()),
        videos_dir: Some("uploaded_videos_test".to_string()),
        enable_local_geocoding: true,
        enable_external_geocoding_fallback: true,
        ai_grpc_url: "http://localhost:50051".to_string(),
        enable_media_backup: Arc::new(AtomicBool::new(true)),
        db_tls: false,
        db_pool_max_size: 16,
        db_pool_min_size: 4,
        db_pool_timeout_secs: 30,
        enable_ai_descriptions: Arc::new(AtomicBool::new(true)),
        enable_embeddings: Arc::new(AtomicBool::new(true)),
        embedding_parallel_count: Arc::new(AtomicUsize::new(10)),
        enable_face_detection: Arc::new(AtomicBool::new(true)),
        face_detection_parallel_count: Arc::new(AtomicUsize::new(3)),
        enable_orientation_detection: Arc::new(AtomicBool::new(true)),
        orientation_detection_parallel_count: Arc::new(AtomicUsize::new(3)),
        log_dir: None,
        ai_http_url: None,
        environment: None,
        port: 8080,
        p2p_data_dir: "data/p2p".to_string(),
        p2p_deterministic_identity: false,
        p2p_discovery_port: 5066,
        p2p_coordinator_addr: None,
        p2p_coordinator_node_id: None,
        p2p_tunnel_local_port: None,
        p2p_tunnel_public_url: None,
        p2p_namespace: "test".to_string(),
        allowed_import_dirs: Some(vec![std::env::temp_dir().to_string_lossy().to_string()]),
        cors_allowed_origins: Vec::new(),
        rate_limit_trusted_proxies: Vec::new(),
        p2p_allowed_node_ids: Vec::new(),
        p2p_identity_kdf: None,
        p2p_data_shards: 3,
        p2p_parity_shards: 2,
        workers: config::WorkerConfig::default(),
    }
}

#[allow(dead_code)]
pub async fn create_test_jwt_token() -> String {
    create_test_jwt_token_with_scope(None).await
}

/// Creates a signed test JWT with an optional `scope` claim (e.g. "media_read").
#[allow(dead_code)]
pub async fn create_test_jwt_token_with_scope(scope: Option<&str>) -> String {
    let shared_secret = "test_secret_key_which_is_at_least_32_bytes_long";

    use jsonwebtoken::{encode, Algorithm, EncodingKey, Header};

    let expiration_time = chrono::Utc::now() + chrono::Duration::days(1);
    let claims = Claims {
        user_id: "550e8400-e29b-41d4-a716-446655440000".to_string(), // Valid UUID for testing
        username: "test-user".to_string(),
        email: "test@example.com".to_string(),
        role: "admin".to_string(),
        exp: expiration_time.timestamp() as usize,
        scope: scope.map(|s| s.to_string()),
    };
    encode(
        &Header::new(Algorithm::HS512),
        &claims,
        &EncodingKey::from_secret(shared_secret.as_ref()),
    )
    .expect("Failed to generate JWT token for test")
}

/// Wraps a database pool in MainDbPool for use in tests
#[allow(dead_code)]
pub fn wrap_main_pool(pool: Pool) -> MainDbPool {
    MainDbPool(pool)
}

/// Creates a GeotaggingDbPool that connects to the geotagging dev database
/// The geotagging database runs in Docker on port 5435 (see docker-compose-dev.yml)
#[allow(dead_code)]
pub async fn create_geotagging_pool() -> GeotaggingDbPool {
    let geotagging_url = std::env::var("TEST_GEOTAGGING_DATABASE_URL")
        .unwrap_or_else(|_| "postgres://postgres:postgres@localhost:5435/geotagging_db".to_string());

    let pool = reminisce::db::create_pool(&geotagging_url)
        .expect("Failed to create geotagging database pool for tests");

    GeotaggingDbPool(pool)
}

/// Creates a mock GeotaggingDbPool that uses the same pool as the main database
/// Only use this for tests that don't need geotagging functionality
#[allow(dead_code)]
pub fn create_mock_geotagging_pool(pool: Pool) -> GeotaggingDbPool {
    GeotaggingDbPool(pool)
}
