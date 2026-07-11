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
    /// 1. Encrypts the data with the provided key.
    /// 2. Splits the encrypted data into shards (data + parity).
    /// Returns the shards and the size of the encrypted blob (needed for reconstruction).
    pub fn process_for_backup(data: &[u8], key: &[u8], nonce_context: &[u8], data_shards: usize, parity_shards: usize) -> Result<(Vec<Vec<u8>>, usize)> {
        // 1. Encrypt
        let encrypted = encryption::encrypt(data, key, nonce_context)?;
        let encrypted_size = encrypted.len();

        // 2. Shard
        let shards = erasure::shard(&encrypted, data_shards, parity_shards)?;

        Ok((shards, encrypted_size))
    }

    /// Reconstructs and decrypts a file from shards.
    /// 1. Reconstructs the encrypted blob from at least data_shards.
    /// 2. Decrypts the blob with the provided key.
    pub fn restore_from_backup(
        shards: Vec<Option<Vec<u8>>>,
        encrypted_size: usize,
        key: &[u8],
        data_shards: usize,
        parity_shards: usize,
    ) -> Result<Vec<u8>> {
        // 1. Reconstruct encrypted blob
        let encrypted = erasure::reconstruct(shards, encrypted_size, data_shards, parity_shards)?;

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
        let (shards, enc_size) = StorageEngine::process_for_backup(original_data, &key, &key, 3, 2).unwrap();
        assert_eq!(shards.len(), 5);

        // Simulate losing 2 storage nodes
        let mut partial_shards: Vec<Option<Vec<u8>>> = shards.into_iter().map(Some).collect();
        partial_shards[0] = None;
        partial_shards[4] = None;

        // Restore
        let restored = StorageEngine::restore_from_backup(partial_shards, enc_size, &key, 3, 2).unwrap();
        assert_eq!(original_data.to_vec(), restored);
    }
}
