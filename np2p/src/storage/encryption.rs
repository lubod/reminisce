use chacha20poly1305::{
    aead::{Aead, KeyInit},
    ChaCha20Poly1305, Key, Nonce,
};
use crate::error::{Np2pError, Result};

pub const KEY_SIZE: usize = 32;
pub const NONCE_SIZE: usize = 12;

/// Encrypts data using ChaCha20-Poly1305 with a random nonce.
/// The nonce is prepended to the resulting ciphertext.
pub fn encrypt(data: &[u8], key_bytes: &[u8]) -> Result<Vec<u8>> {
    if key_bytes.len() != KEY_SIZE {
        return Err(Np2pError::Crypto(format!("Invalid key size: expected {}, got {}", KEY_SIZE, key_bytes.len())));
    }

    let key = Key::from_slice(key_bytes);
    let cipher = ChaCha20Poly1305::new(key);
    
    // Generate a random nonce
    let mut nonce_bytes = [0u8; NONCE_SIZE];
    use rand::RngCore;
    rand::thread_rng().fill_bytes(&mut nonce_bytes);
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
        let key = [0u8; 32]; // Example key
        let data = b"Hello, np2p distributed storage!";
        
        let encrypted = encrypt(data, &key).expect("Encryption failed");
        assert!(encrypted.len() > data.len());
        
        let decrypted = decrypt(&encrypted, &key).expect("Decryption failed");
        assert_eq!(data, decrypted.as_slice());
    }

    #[test]
    fn test_decrypt_wrong_key() {
        let key1 = [1u8; 32];
        let key2 = [2u8; 32];
        let data = b"Sensitive data";
        
        let encrypted = encrypt(data, &key1).unwrap();
        let result = decrypt(&encrypted, &key2);
        
        assert!(result.is_err());
    }
}
