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

/// Master key used by key-envelope **v1** to wrap per-file encryption keys.
///
/// v0 wrapped keys under SHA256(api_secret): a single unsalted fast hash, so
/// anyone with DB read access could brute-force the master secret offline at
/// hash speed. v1 derives the wrapping key with Argon2id (memory-hard), making
/// each guess cost ~50-100ms. The KDF output is cached per process so bulk
/// wrap/unwrap loops (replication, audit, restore) stay fast after first use.
fn keywrap_master_key_v1(api_secret: &str) -> [u8; 32] {
    use argon2::{Algorithm, Argon2, Params, Version};
    use std::sync::Mutex;

    static CACHE: Mutex<Option<([u8; 32], [u8; 32])>> = Mutex::new(None);
    const SALT: &[u8] = b"reminisce-keywrap-v1";
    let params = Params::new(64 * 1024, 3, 1, Some(32)).expect("static KDF params are valid");
    let argon = Argon2::new(Algorithm::Argon2id, Version::V0x13, params);

    let cache_key: [u8; 32] = blake3::hash(api_secret.as_bytes()).into();
    let mut cache = CACHE.lock().unwrap_or_else(|e| e.into_inner());
    if let Some((seen, derived)) = cache.as_ref() {
        if *seen == cache_key {
            return *derived;
        }
    }
    let mut out = [0u8; 32];
    argon
        .hash_password_into(api_secret.as_bytes(), SALT, &mut out)
        .expect("argon2id derivation with static params cannot fail");
    *cache = Some((cache_key, out));
    out
}

/// Encrypt a 32-byte key with a master key derived from the API secret key.
///
/// Emits key-envelope v1: `0x01 || nonce(12) || AEAD(ciphertext)`. The legacy
/// (v0) format remains readable via `decrypt_key` for existing data.
pub fn encrypt_key(key: &[u8; 32], api_secret: &str) -> Result<Vec<u8>, String> {
    use chacha20poly1305::aead::{Aead, KeyInit};
    use chacha20poly1305::{ChaCha20Poly1305, Nonce};
    use rand::RngCore;

    let master_key = keywrap_master_key_v1(api_secret);
    let cipher = ChaCha20Poly1305::new_from_slice(&master_key)
        .map_err(|e| format!("Failed to initialize cipher: {}", e))?;

    // Generate a random 12-byte nonce
    let mut nonce_bytes = [0u8; 12];
    let mut rng = rand::rng();
    rng.fill_bytes(&mut nonce_bytes);
    let nonce = Nonce::from_slice(&nonce_bytes);

    let ciphertext = cipher.encrypt(nonce, key.as_ref())
        .map_err(|e| format!("Encryption failed: {}", e))?;

    // Envelope: version byte || nonce || ciphertext
    let mut result = Vec::with_capacity(1 + 12 + ciphertext.len());
    result.push(0x01);
    result.extend_from_slice(&nonce_bytes);
    result.extend_from_slice(&ciphertext);
    Ok(result)
}

/// Decrypt a key using the API secret key.
///
/// Understands all three storage formats:
/// - v1 envelope `0x01 || nonce || ct` wrapped under the Argon2id master key,
/// - legacy v0 `nonce || ct` wrapped under SHA256(secret),
/// - raw 32-byte plaintext keys from early deployments.
pub fn decrypt_key(encrypted_key: &[u8], api_secret: &str) -> Result<Vec<u8>, String> {
    use chacha20poly1305::aead::{Aead, KeyInit};
    use chacha20poly1305::{ChaCha20Poly1305, Nonce};
    use sha2::{Sha256, Digest};

    if encrypted_key.len() == 32 {
        // Legacy plaintext key
        return Ok(encrypted_key.to_vec());
    }

    // v1 envelope: version byte must be 0x01 and there must be room for
    // nonce(12) + Poly1305 tag(16). AEAD authentication makes the check safe —
    // on any failure we fall back to the legacy path below.
    if encrypted_key.len() >= 29 && encrypted_key[0] == 0x01 {
        let master_key = keywrap_master_key_v1(api_secret);
        if let Ok(cipher) = ChaCha20Poly1305::new_from_slice(&master_key) {
            let nonce = Nonce::from_slice(&encrypted_key[1..13]);
            if let Ok(decrypted) = cipher.decrypt(nonce, &encrypted_key[13..]) {
                return Ok(decrypted);
            }
        }
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

#[cfg(test)]
mod key_envelope_tests {
    use super::*;

    const SECRET: &str = "unit-test-master-secret";

    #[test]
    fn encrypt_decrypt_roundtrip_v1() {
        let mut key = [0u8; 32];
        rand::RngCore::fill_bytes(&mut rand::rng(), &mut key);
        let wrapped = encrypt_key(&key, SECRET).expect("wrap");
        assert_eq!(wrapped[0], 0x01, "new wraps must use the v1 envelope");
        let unwrapped = decrypt_key(&wrapped, SECRET).expect("unwrap");
        assert_eq!(unwrapped, key.to_vec());
    }

    #[test]
    fn v1_wraps_reject_wrong_secret() {
        let key = [0x42u8; 32];
        let wrapped = encrypt_key(&key, SECRET).expect("wrap");
        assert!(decrypt_key(&wrapped, "different-secret").is_err());
    }

    #[test]
    fn legacy_v0_wrapped_keys_still_decrypt() {
        // Reconstruct the legacy v0 envelope exactly as the pre-v1 code wrote it:
        // nonce(12) || ChaCha20Poly1305(SHA256(secret)).encrypt(key)
        use chacha20poly1305::aead::{Aead, KeyInit};
        use chacha20poly1305::{ChaCha20Poly1305, Nonce};
        use sha2::{Digest, Sha256};

        let key = [0x77u8; 32];
        let mut hasher = Sha256::new();
        hasher.update(SECRET.as_bytes());
        let master = hasher.finalize();
        let cipher = ChaCha20Poly1305::new_from_slice(&master).unwrap();
        let nonce = Nonce::from_slice(&[0xAAu8; 12]);
        let ct = cipher.encrypt(nonce, key.as_ref()).unwrap();

        let mut legacy = Vec::new();
        legacy.extend_from_slice(&[0xAAu8; 12]);
        legacy.extend_from_slice(&ct);

        // 0xAA first byte means the v1 branch cannot match; legacy path must win.
        let unwrapped = decrypt_key(&legacy, SECRET).expect("legacy unwrap must keep working");
        assert_eq!(unwrapped, key.to_vec());
    }

    #[test]
    fn legacy_plaintext_32byte_keys_still_decrypt() {
        let raw = vec![0x55u8; 32];
        assert_eq!(decrypt_key(&raw, SECRET).unwrap(), raw);
    }
}
