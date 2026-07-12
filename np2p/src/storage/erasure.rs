//! Reed-Solomon 3/5 erasure coding wrapper.
//!
//! Encodes data into DATA_SHARDS=3 data shards + PARITY_SHARDS=2 parity shards.
//! Any 3 of the 5 shards are sufficient to recover the original data.
//! Uses the galois_8 field via the reed-solomon-erasure crate.

use reed_solomon_erasure::galois_8::ReedSolomon;
use crate::error::{Np2pError, Result};

pub const DATA_SHARDS: usize = 3;
pub const PARITY_SHARDS: usize = 2;
pub const TOTAL_SHARDS: usize = DATA_SHARDS + PARITY_SHARDS;

/// Splits data into shards using Reed-Solomon erasure coding.
/// Returns a vector of shards. 
/// Each shard will be the same size (padded if necessary).
pub fn shard(data: &[u8], data_shards: usize, parity_shards: usize) -> Result<Vec<Vec<u8>>> {
    if data.is_empty() {
        return Err(Np2pError::ErasureCoding("Cannot shard empty data".to_string()));
    }
    if data_shards == 0 {
        return Err(Np2pError::ErasureCoding("Data shards count must be positive".to_string()));
    }

    let r = ReedSolomon::new(data_shards, parity_shards)
        .map_err(|e| Np2pError::ErasureCoding(format!("Failed to initialize ReedSolomon: {}", e)))?;

    // 1. Calculate shard size (must be multiple of DATA_SHARDS if we want perfect fit, 
    // but ReedSolomon handles padding if we provide chunks).
    // The simplest way: pad data to be a multiple of DATA_SHARDS.
    let shard_size = data.len().div_ceil(data_shards);
    let total_data_size = shard_size * data_shards;

    let mut padded_data = data.to_vec();
    padded_data.resize(total_data_size, 0u8);

    // 2. Split into data shards
    let mut shards: Vec<Vec<u8>> = padded_data
        .chunks_exact(shard_size)
        .map(|chunk| chunk.to_vec())
        .collect();

    // 3. Add empty parity shards
    for _ in 0..parity_shards {
        shards.push(vec![0u8; shard_size]);
    }

    // 4. Encode parity
    r.encode(&mut shards)
        .map_err(|e| Np2pError::ErasureCoding(format!("Encoding failed: {}", e)))?;

    Ok(shards)
}

/// Reconstructs the original data from a subset of shards.
/// `shards` is a vector of Option<Vec<u8>>. Missing shards are None.
/// `original_size` is needed to truncate the padding.
pub fn reconstruct(
    mut shards: Vec<Option<Vec<u8>>>,
    original_size: usize,
    data_shards: usize,
    parity_shards: usize,
) -> Result<Vec<u8>> {
    let total_shards = data_shards + parity_shards;
    if shards.len() != total_shards {
        return Err(Np2pError::ErasureCoding(format!("Expected {} shards, got {}", total_shards, shards.len())));
    }

    let present_count = shards.iter().filter(|s| s.is_some()).count();
    if present_count < data_shards {
        return Err(Np2pError::ErasureCoding(format!("Not enough shards for reconstruction: need {}, got {}", data_shards, present_count)));
    }

    let r = ReedSolomon::new(data_shards, parity_shards)
        .map_err(|e| Np2pError::ErasureCoding(format!("Failed to initialize ReedSolomon: {}", e)))?;

    // Reconstruct missing shards
    r.reconstruct(&mut shards)
        .map_err(|e| Np2pError::ErasureCoding(format!("Reconstruction failed: {}", e)))?;

    // Combine data shards
    let mut result = Vec::with_capacity(original_size);
    for shard_data in shards.iter().take(data_shards).flatten() {
        result.extend_from_slice(shard_data);
    }

    // Truncate to original size
    result.truncate(original_size);
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_shard_reconstruct_roundtrip() {
        let data = b"This is a test of the emergency broadcast system. It is only a test.";
        let original_size = data.len();

        let shards = shard(data, DATA_SHARDS, PARITY_SHARDS).expect("Sharding failed");
        assert_eq!(shards.len(), 5);

        // Simulate losing 2 shards (the last two)
        let mut partial_shards: Vec<Option<Vec<u8>>> = shards.into_iter().map(Some).collect();
        partial_shards[3] = None;
        partial_shards[4] = None;

        let reconstructed = reconstruct(partial_shards, original_size, DATA_SHARDS, PARITY_SHARDS).expect("Reconstruction failed");
        assert_eq!(data.to_vec(), reconstructed);
    }

    #[test]
    fn test_shard_reconstruct_lose_data_shards() {
        let data = vec![1u8; 1000];
        let original_size = data.len();

        let shards = shard(&data, DATA_SHARDS, PARITY_SHARDS).expect("Sharding failed");

        // Simulate losing 2 data shards
        let mut partial_shards: Vec<Option<Vec<u8>>> = shards.into_iter().map(Some).collect();
        partial_shards[0] = None;
        partial_shards[1] = None;

        let reconstructed = reconstruct(partial_shards, original_size, DATA_SHARDS, PARITY_SHARDS).expect("Reconstruction failed");
        assert_eq!(data, reconstructed);
    }

    #[test]
    fn test_reconstruct_too_many_lost() {
        let data = b"Small data";
        let shards = shard(data, DATA_SHARDS, PARITY_SHARDS).unwrap();

        // Simulate losing 3 shards (only 2 left, need 3)
        let mut partial_shards: Vec<Option<Vec<u8>>> = shards.into_iter().map(Some).collect();
        partial_shards[0] = None;
        partial_shards[1] = None;
        partial_shards[2] = None;

        let result = reconstruct(partial_shards, data.len(), DATA_SHARDS, PARITY_SHARDS);
        assert!(result.is_err());
    }
}
