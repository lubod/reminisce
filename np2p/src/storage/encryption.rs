//! ChaCha20Poly1305 encryption for P2P shard data.
//!
//! Uses a deterministic 12-byte nonce derived from blake3(key || nonce_context)
//! so that re-encrypting the same file+key always produces identical ciphertext.
//! This is required for shard repair: a replacement shard must be byte-for-byte
//! compatible with the surviving shards from the original encryption pass.

use chacha20poly1305::{
    aead::{Aead, KeyInit},
    ChaCha20Poly1305, Key, Nonce,
};
use crate::error::{Np2pError, Result};

pub const KEY_SIZE: usize = 32;
pub const NONCE_SIZE: usize = 12;

/// Encrypts data using ChaCha20-Poly1305 with a deterministic nonce derived from
/// blake3(key || nonce_context). The nonce is prepended to the resulting ciphertext.
///
/// The `nonce_context` is a caller-chosen domain separator (e.g. file id +
/// segment index or the content hash). The final nonce is additionally bound to
/// the ACTUAL plaintext content (`blake3(data)`), so two distinct payloads can
/// never reuse a nonce even when callers pass the same context under the same
/// key — nonce reuse on ChaCha20-Poly1305 is catastrophic (ECB-style plaintext
/// XOR leak + forgery). Identical content + key + context still derives the same
/// nonce and reproduces byte-identical ciphertext, preserving shard-repair
/// compatibility.
pub fn encrypt(data: &[u8], key_bytes: &[u8], nonce_context: &[u8]) -> Result<Vec<u8>> {
    if key_bytes.len() != KEY_SIZE {
        return Err(Np2pError::Crypto(format!("Invalid key size: expected {}, got {}", KEY_SIZE, key_bytes.len())));
    }

    let key = Key::from_slice(key_bytes);
    let cipher = ChaCha20Poly1305::new(key);
    // Bind the nonce to the content itself: different content -> different nonce,
    // even under an identical (key, nonce_context) pair.
    let content_hash = blake3::hash(data);
    let mut nonce_input = Vec::with_capacity(key_bytes.len() + nonce_context.len() + 32);
    nonce_input.extend_from_slice(key_bytes);
    nonce_input.extend_from_slice(nonce_context);
    nonce_input.extend_from_slice(content_hash.as_bytes());
    let nonce_hash = blake3::hash(&nonce_input);
    let nonce_bytes: [u8; NONCE_SIZE] = nonce_hash.as_bytes()[..NONCE_SIZE].try_into().unwrap();
    let nonce = Nonce::from_slice(&nonce_bytes);

    // Encrypt data
    let ciphertext = cipher
        .encrypt(nonce, data)
        .map_err(|e| Np2pError::Crypto(format!("Encryption failed: {}", e)))?;

    // Prepend nonce to ciphertext: [Nonce (12b)][Ciphertext (Nb)]
    let mut result = Vec::with_capacity(NONCE_SIZE + ciphertext.len());
    result.extend_from_slice(&nonce_bytes);
    result.extend_from_slice(&ciphertext);

    Ok(result)
}

/// Decrypts data that was encrypted with the `encrypt` function.
/// Expects the nonce to be prepended to the ciphertext.
pub fn decrypt(encrypted_data: &[u8], key_bytes: &[u8]) -> Result<Vec<u8>> {
    if key_bytes.len() != KEY_SIZE {
        return Err(Np2pError::Crypto(format!("Invalid key size: expected {}, got {}", KEY_SIZE, key_bytes.len())));
    }

    if encrypted_data.len() < NONCE_SIZE {
        return Err(Np2pError::Crypto("Encrypted data too short to contain a nonce".to_string()));
    }

    let key = Key::from_slice(key_bytes);
    let cipher = ChaCha20Poly1305::new(key);

    // Split nonce and ciphertext
    let (nonce_bytes, ciphertext) = encrypted_data.split_at(NONCE_SIZE);
    let nonce = Nonce::from_slice(nonce_bytes);

    // Decrypt data
    let plaintext = cipher
        .decrypt(nonce, ciphertext)
        .map_err(|e| Np2pError::Crypto(format!("Decryption failed (likely invalid key or corrupted data): {}", e)))?;

    Ok(plaintext)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_encrypt_decrypt_roundtrip() {
        let key = [0u8; 32];
        let data = b"Hello, np2p distributed storage!";
        let ctx = b"test-context";
        let encrypted = encrypt(data, &key, ctx).expect("Encryption failed");
        assert!(encrypted.len() > data.len());
        let decrypted = decrypt(&encrypted, &key).expect("Decryption failed");
        assert_eq!(data, decrypted.as_slice());
    }

    #[test]
    fn test_deterministic_nonce() {
        let key = [0u8; 32];
        let data = b"Hello, np2p distributed storage!";
        let ctx = b"file-hash-context";
        let enc1 = encrypt(data, &key, ctx).unwrap();
        let enc2 = encrypt(data, &key, ctx).unwrap();
        assert_eq!(enc1, enc2, "Same inputs must produce identical ciphertext");
    }

    #[test]
    fn test_decrypt_wrong_key() {
        let key1 = [1u8; 32];
        let key2 = [2u8; 32];
        let data = b"Sensitive data";
        let encrypted = encrypt(data, &key1, &key1).unwrap();
        let result = decrypt(&encrypted, &key2);
        assert!(result.is_err());
    }

    #[test]
    fn test_distinct_content_never_reuses_nonce_same_key_and_context() {
        // Regression: the nonce is bound to the content, so two different payloads
        // encrypted under the SAME key and SAME caller context must use different
        // nonces (nonce reuse on ChaCha20-Poly1305 leaks the keystream). The e2e
        // test historically passed the key as the context; that must be harmless.
        let key = [0x42u8; 32];
        let shared_ctx = key; // the exact misuse pattern from the old e2e test
        let data_a = b"first distinct payload";
        let data_b = b"second, different payload";

        let enc_a = encrypt(data_a, &key, &shared_ctx).unwrap();
        let enc_b = encrypt(data_b, &key, &shared_ctx).unwrap();

        assert_ne!(&enc_a[..NONCE_SIZE], &enc_b[..NONCE_SIZE],
            "different content must yield a different nonce under identical key+context");
        assert_ne!(enc_a, enc_b);
        assert_eq!(decrypt(&enc_a, &key).unwrap(), data_a);
        assert_eq!(decrypt(&enc_b, &key).unwrap(), data_b);
    }

    #[test]
    fn test_encrypt_rejects_wrong_key_size() {
        let data = b"payload";
        let ctx = b"ctx";
        assert!(encrypt(data, &[0u8; 31], ctx).is_err(), "31-byte key must be rejected");
        assert!(encrypt(data, &[0u8; 33], ctx).is_err(), "33-byte key must be rejected");
    }

    #[test]
    fn test_identical_content_reproduces_identical_ciphertext() {
        // Shard-repair compatibility: re-encrypting identical content must be
        // byte-identical so a repaired shard matches the surviving ones.
        let key = [0x24u8; 32];
        let data = vec![0xABu8; 4096];
        let ctx = b"segment-3";
        let e1 = encrypt(&data, &key, ctx).unwrap();
        let e2 = encrypt(&data, &key, ctx).unwrap();
        assert_eq!(e1, e2);
    }
}
