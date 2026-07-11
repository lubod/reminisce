use log::error;
use std::path::PathBuf;

pub use crate::system_utils::{
    WorkerConcurrencyLimits, get_load_average, get_gpu_load, get_cpu_count,
    adjust_batch_size, calculate_worker_concurrency, calculate_parallel_batch_size,
    run_worker_loop,
};
pub use crate::auth_utils::{parse_user_uuid, ensure_user_exists, authenticate_request};
pub use crate::geo_utils::{extract_gps_coordinates, reverse_geocode};
pub use crate::media_utils::{
    ExistenceCheckResult, get_subdirectory_path, check_if_exists,
    determine_image_type, determine_video_type,
    list_thumbnails, total_thumbnails,
    parse_date_from_image_name, parse_date_from_video_name,
    cleanup_temp_files, cleanup_temp_files_spawn,
};

/// Get a DB client from the pool, returning 500 on failure.
pub async fn get_db_client(pool: &deadpool_postgres::Pool) -> Result<deadpool_postgres::Client, actix_web::Error> {
    pool.get().await.map_err(|e| {
        error!("Failed to get DB client: {}", e);
        actix_web::error::ErrorInternalServerError("Database connection failed")
    })
}

/// Helper to dump DB to a file
pub fn perform_db_dump(config: &crate::config::Config) -> Result<PathBuf, String> {
    let database_url = config.database_url.as_ref().ok_or("Database URL not configured")?;

    let password = url::Url::parse(database_url)
        .ok()
        .and_then(|url| url.password().map(|p| p.to_string()))
        .unwrap_or_else(|| "postgres".to_string());

    let output_path = PathBuf::from(format!("db_dump_temp_{}.sql", chrono::Utc::now().timestamp()));

    let file = std::fs::File::create(&output_path).map_err(|e| e.to_string())?;

    let mut command = std::process::Command::new("pg_dump");
    command
        .arg("--format=plain")
        .env("PGPASSWORD", password)
        .arg(database_url)
        .stdout(file);

    match command.status() {
        Ok(status) if status.success() => Ok(output_path),
        Ok(_) => Err("pg_dump failed".to_string()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Err("pg_dump missing".to_string()),
        Err(e) => Err(e.to_string()),
    }
}

/// Parse a peer address string into a SocketAddr.
/// Handles both "ip:port" and bare "ip" (defaults to port 5050).
pub fn parse_peer_addr(peer: &str) -> Result<std::net::SocketAddr, String> {
    if let Ok(addr) = peer.parse::<std::net::SocketAddr>() {
        return Ok(addr);
    }
    format!("{}:5050", peer)
        .parse::<std::net::SocketAddr>()
        .map_err(|e| format!("Invalid peer address '{}': {}", peer, e))
}

/// Assert that the given table name is one of the strictly whitelisted database tables.
/// This prevents SQL injection patterns when database tables must be dynamically interpolated.
pub fn validate_table_name(table: &str) -> Result<(), &'static str> {
    let allowed = [
        "images", "videos", "starred_images", "starred_videos", 
        "p2p_shards", "users", "persons", "faces", "image_labels", "video_labels"
    ];
    if !allowed.contains(&table) {
        Err("CRITICAL SECURITY: Invalid database table name dynamic query interpolation attempted")
    } else {
        Ok(())
    }
}

/// Encrypt a 32-byte key with a master key derived from the API secret key.
pub fn encrypt_key(key: &[u8; 32], api_secret: &str) -> Vec<u8> {
    use chacha20poly1305::aead::{Aead, KeyInit};
    use chacha20poly1305::{ChaCha20Poly1305, Nonce};
    use sha2::{Sha256, Digest};
    use rand::RngCore;

    let mut hasher = Sha256::new();
    hasher.update(api_secret.as_bytes());
    let master_key = hasher.finalize();

    let cipher = ChaCha20Poly1305::new_from_slice(&master_key).unwrap();
    
    // Generate a random 12-byte nonce
    let mut nonce_bytes = [0u8; 12];
    let mut rng = rand::rng();
    rng.fill_bytes(&mut nonce_bytes);
    let nonce = Nonce::from_slice(&nonce_bytes);

    let ciphertext = cipher.encrypt(nonce, key.as_ref()).expect("Encryption failed");
    
    // Combine nonce + ciphertext
    let mut result = Vec::with_capacity(12 + ciphertext.len());
    result.extend_from_slice(&nonce_bytes);
    result.extend_from_slice(&ciphertext);
    result
}

/// Decrypt a key using the API secret key.
/// Falls back to returning the key as-is if it is not encrypted (e.g. legacy data).
pub fn decrypt_key(encrypted_key: &[u8], api_secret: &str) -> Result<Vec<u8>, String> {
    use chacha20poly1305::aead::{Aead, KeyInit};
    use chacha20poly1305::{ChaCha20Poly1305, Nonce};
    use sha2::{Sha256, Digest};

    if encrypted_key.len() == 32 {
        // Legacy plaintext key
        return Ok(encrypted_key.to_vec());
    }

    if encrypted_key.len() < 12 {
        return Err("Encrypted key is too short".to_string());
    }

    let mut hasher = Sha256::new();
    hasher.update(api_secret.as_bytes());
    let master_key = hasher.finalize();

    let cipher = ChaCha20Poly1305::new_from_slice(&master_key)
        .map_err(|e| format!("Failed to initialize cipher: {}", e))?;

    let nonce_bytes = &encrypted_key[0..12];
    let ciphertext = &encrypted_key[12..];
    let nonce = Nonce::from_slice(nonce_bytes);

    let decrypted = cipher.decrypt(nonce, ciphertext)
        .map_err(|e| format!("Decryption failed: {}", e))?;
    Ok(decrypted)
}

/// Global lock to coordinate rebalance and audit background tasks to avoid race conditions.
pub static P2P_WORKER_LOCK: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

static HTTP_CLIENT: std::sync::OnceLock<reqwest::Client> = std::sync::OnceLock::new();

/// Get a globally shared reqwest::Client to enable HTTP connection pooling.
pub fn get_http_client() -> &'static reqwest::Client {
    HTTP_CLIENT.get_or_init(|| {
        reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(60))
            .build()
            .expect("Failed to initialize global HTTP client")
    })
}
