use log::error;

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
pub fn encrypt_key(key: &[u8; 32], api_secret: &str) -> Result<Vec<u8>, String> {
    use chacha20poly1305::aead::{Aead, KeyInit};
    use chacha20poly1305::{ChaCha20Poly1305, Nonce};
    use sha2::{Sha256, Digest};
    use rand::RngCore;

    let mut hasher = Sha256::new();
    hasher.update(api_secret.as_bytes());
    let master_key = hasher.finalize();

    let cipher = ChaCha20Poly1305::new_from_slice(&master_key)
        .map_err(|e| format!("Failed to initialize cipher: {}", e))?;
    
    // Generate a random 12-byte nonce
    let mut nonce_bytes = [0u8; 12];
    let mut rng = rand::rng();
    rng.fill_bytes(&mut nonce_bytes);
    let nonce = Nonce::from_slice(&nonce_bytes);

    let ciphertext = cipher.encrypt(nonce, key.as_ref())
        .map_err(|e| format!("Encryption failed: {}", e))?;
    
    // Combine nonce + ciphertext
    let mut result = Vec::with_capacity(12 + ciphertext.len());
    result.extend_from_slice(&nonce_bytes);
    result.extend_from_slice(&ciphertext);
    Ok(result)
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
