pub mod encryption;
pub mod erasure;
pub mod disk;

pub use encryption::{encrypt, decrypt, KEY_SIZE};
pub use erasure::{shard, reconstruct, TOTAL_SHARDS, DATA_SHARDS, PARITY_SHARDS};
pub use disk::DiskStorage;

use crate::error::Result;

/// High-level engine for distributed storage operations.
/// Combines encryption and erasure coding.
pub struct StorageEngine;

impl StorageEngine {
    /// Prepares a file for distributed backup.
    /// 1. Encrypts the data with the provided key and deterministic nonce derived from .
    /// 2. Splits the encrypted data into 5 shards (3 data + 2 parity).
    /// Returns the 5 shards and the size of the encrypted blob (needed for reconstruction).
    ///
    ///  must be unique per (file, key, segment). Pass  for single-segment
    /// files (key is randomly generated once per file). For multi-segment files pass
    ///  to avoid nonce reuse across segments.
    pub fn process_for_backup(data: &[u8], key: &[u8], nonce_context: &[u8]) -> Result<(Vec<Vec<u8>>, usize)> {
        // 1. Encrypt
        let encrypted = encryption::encrypt(data, key, nonce_context)?;
        let encrypted_size = encrypted.len();

        // 2. Shard
        let shards = erasure::shard(&encrypted)?;

        Ok((shards, encrypted_size))
    }

    /// Reconstructs and decrypts a file from shards.
    /// 1. Reconstructs the encrypted blob from at least 3 shards.
    /// 2. Decrypts the blob with the provided key.
    pub fn restore_from_backup(shards: Vec<Option<Vec<u8>>>, encrypted_size: usize, key: &[u8]) -> Result<Vec<u8>> {
        // 1. Reconstruct encrypted blob
        let encrypted = erasure::reconstruct(shards, encrypted_size)?;

        // 2. Decrypt
        let plaintext = encryption::decrypt(&encrypted, key)?;

        Ok(plaintext)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_storage_engine_roundtrip() {
        let key = [0xAAu8; 32];
        let original_data = b"Distributed backup test with encryption and EC 3/5.";
        
        // Backup
        let (shards, enc_size) = StorageEngine::process_for_backup(original_data, &key, &key).unwrap();
        assert_eq!(shards.len(), 5);

        // Simulate losing 2 storage nodes
        let mut partial_shards: Vec<Option<Vec<u8>>> = shards.into_iter().map(Some).collect();
        partial_shards[0] = None;
        partial_shards[4] = None;

        // Restore
        let restored = StorageEngine::restore_from_backup(partial_shards, enc_size, &key).unwrap();
        assert_eq!(original_data.to_vec(), restored);
    }
}
