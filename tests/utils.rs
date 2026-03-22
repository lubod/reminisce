use reminisce::config;
use reminisce::Claims;
use reminisce::db::{MainDbPool, GeotaggingDbPool};
use deadpool_postgres::Pool;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicUsize};

pub fn create_test_config() -> config::Config {
    config::Config {
        database_url: Some("".to_string()),
        geotagging_database_url: "postgres://postgres:postgres@localhost:5435/geotagging_db".to_string(),
        api_secret_key: Some("test_secret".to_string()),
        images_dir: Some("uploaded_images_test".to_string()),
        videos_dir: Some("uploaded_videos_test".to_string()),
        enable_local_geocoding: true,
        enable_external_geocoding_fallback: true,
        embedding_service_url: "http://localhost:8081".to_string(),
        face_service_url: "http://localhost:8082".to_string(),
        p2p_daemon_host: Some("127.0.0.1".to_string()),
        p2p_daemon_port: Some(5050),
        enable_media_backup: Arc::new(AtomicBool::new(true)),
        external_ip: None,
        db_pool_max_size: 16,
        db_pool_min_size: 4,
        db_pool_timeout_secs: 30,
        enable_ai_descriptions: Arc::new(AtomicBool::new(true)),
        enable_embeddings: Arc::new(AtomicBool::new(true)),
        embedding_parallel_count: Arc::new(AtomicUsize::new(10)),
        enable_face_detection: Arc::new(AtomicBool::new(true)),
        face_detection_parallel_count: Arc::new(AtomicUsize::new(3)),
        otlp_endpoint: None,
        environment: None,
        relay_url: None,
        relay_api_key: None,
        relay_username: None,
        relay_password: None,
        advertise_addr: None,
        main_server_url: None,
        port: 8080,
        p2p_data_dir: "data/p2p".to_string(),
        p2p_peers: vec![],
        p2p_discovery_port: 5060,
        p2p_coordinator_addr: None,
        p2p_tunnel_local_port: None,
        p2p_tunnel_public_url: None,
        p2p_namespace: "test".to_string(),
    }
}

#[allow(dead_code)]
pub async fn create_test_jwt_token() -> String {
    let shared_secret = "test_secret";

    use jsonwebtoken::{encode, Algorithm, EncodingKey, Header};

    let expiration_time = chrono::Utc::now() + chrono::Duration::days(1);
    let claims = Claims {
        user_id: "550e8400-e29b-41d4-a716-446655440000".to_string(), // Valid UUID for testing
        username: "test-user".to_string(),
        email: "test@example.com".to_string(),
        role: "admin".to_string(),
        exp: expiration_time.timestamp() as usize,
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
