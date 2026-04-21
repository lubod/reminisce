//! Core logic for restoring media files from P2P storage.
//!
//! Fetches shards concurrently from their assigned nodes, tolerates up to 2 missing
//! shards (3/5 Reed-Solomon), then decrypts and reassembles the original file.
//! Handles both single-segment files and large multi-segment files.
//! Called by the HTTP handler (services/p2p_restore.rs) and the CLI binary.

use std::sync::Arc;
use deadpool_postgres::Pool;
use np2p::network::P2PService;
use np2p::network::protocol::Message;
use np2p::storage::{StorageEngine, DATA_SHARDS, TOTAL_SHARDS};
use tracing::{info, warn};

#[derive(Debug)]
pub struct RestoredFile {
    pub data: Vec<u8>,
    pub filename: String,
    pub media_type: String,
}

struct FileRecord {
    name: String,
    ext: String,
    media_type: String,
    encryption_key: Vec<u8>,
    encrypted_size: i32,
    segment_count: i32,
    segment_enc_sizes: Option<Vec<i64>>,
}

/// Restore a file from P2P backup, fetching shards from peers via P2PService.
pub async fn restore_file(
    pool: &Pool,
    p2p_service: &Arc<P2PService>,
    file_hash: &str,
) -> Result<RestoredFile, Box<dyn std::error::Error + Send + Sync>> {
    let svc = p2p_service.clone();
    restore_file_with_fetcher(pool, file_hash, move |node_id, shard_hash| {
        let svc = svc.clone();
        async move {
            match svc.send_message(&node_id, &Message::RetrieveShardRequest { shard_hash }).await {
                Ok(Message::RetrieveShardResponse { data, .. }) => data,
                _ => None,
            }
        }
    }).await
}

/// Core restore logic with an injectable shard fetcher.
///
/// `fetch(node_id, shard_hash)` returns `Some(data)` on success, `None` if unavailable.
/// Results are BLAKE3-verified before use; a mismatch is treated as unavailable.
pub async fn restore_file_with_fetcher<F, Fut>(
    pool: &Pool,
    file_hash: &str,
    fetch: F,
) -> Result<RestoredFile, Box<dyn std::error::Error + Send + Sync>>
where
    F: Fn(String, [u8; 32]) -> Fut,
    Fut: std::future::Future<Output = Option<Vec<u8>>>,
{
    let client = pool.get().await?;

    let rec = query_file_record(&client, file_hash).await?
        .ok_or_else(|| format!("File {} not found in database", file_hash))?;

    let shard_rows = client.query(
        "SELECT shard_index, node_id, shard_hash \
         FROM p2p_shards WHERE file_hash = $1 ORDER BY shard_index",
        &[&file_hash],
    ).await?;

    if shard_rows.is_empty() {
        return Err(format!("No shards found for file {}", file_hash).into());
    }

    let mut full_shards: Vec<Option<Vec<u8>>> = vec![None; TOTAL_SHARDS];

    for row in &shard_rows {
        let shard_index: i32 = row.get(0);
        let node_id: String = row.get(1);
        let shard_hash_hex: String = row.get(2);

        let idx = shard_index as usize;
        if idx >= TOTAL_SHARDS {
            warn!("Shard index {} out of range, skipping", idx);
            continue;
        }

        let hash_bytes = match parse_shard_hash(&shard_hash_hex) {
            Some(b) => b,
            None => {
                warn!("Invalid shard hash hex for shard {}: {}", idx, shard_hash_hex);
                continue;
            }
        };

        match fetch(node_id.clone(), hash_bytes).await {
            Some(data) => {
                let actual = blake3::hash(&data).to_hex().to_string();
                if actual == shard_hash_hex {
                    info!("Fetched shard {} from node {}", idx, node_id);
                    full_shards[idx] = Some(data);
                } else {
                    warn!("Hash mismatch for shard {} from node {} — discarding", idx, node_id);
                }
            }
            None => {
                warn!("Shard {} unavailable from node {}", idx, node_id);
            }
        }
    }

    let present = full_shards.iter().filter(|s| s.is_some()).count();
    if present < DATA_SHARDS {
        return Err(format!(
            "Only {}/{} shards available for {} — cannot restore (need ≥{})",
            present, TOTAL_SHARDS, file_hash, DATA_SHARDS
        ).into());
    }

    let plaintext = if rec.segment_count <= 1 {
        StorageEngine::restore_from_backup(full_shards, rec.encrypted_size as usize, &rec.encryption_key)
            .map_err(|e| format!("Restore failed: {}", e))?
    } else {
        let enc_sizes = rec.segment_enc_sizes
            .ok_or("p2p_segment_enc_sizes is NULL for multi-segment file")?;
        restore_segmented(full_shards, &enc_sizes, &rec.encryption_key)?
    };

    Ok(RestoredFile {
        data: plaintext,
        filename: format!("{}.{}", rec.name, rec.ext),
        media_type: rec.media_type,
    })
}

async fn query_file_record(
    client: &deadpool_postgres::Object,
    file_hash: &str,
) -> Result<Option<FileRecord>, Box<dyn std::error::Error + Send + Sync>> {
    let row = client.query_opt(
        "SELECT name, ext, p2p_encryption_key, p2p_encrypted_size, \
         p2p_segment_count, p2p_segment_enc_sizes \
         FROM images WHERE hash = $1 AND deleted_at IS NULL",
        &[&file_hash],
    ).await?;

    if let Some(r) = row {
        let key: Option<Vec<u8>> = r.get(2);
        return Ok(Some(FileRecord {
            name: r.get(0),
            ext: r.get(1),
            media_type: "image".to_string(),
            encryption_key: key.ok_or("p2p_encryption_key is NULL")?,
            encrypted_size: r.get(3),
            segment_count: r.get(4),
            segment_enc_sizes: r.get(5),
        }));
    }

    let row = client.query_opt(
        "SELECT name, ext, p2p_encryption_key, p2p_encrypted_size, \
         p2p_segment_count, p2p_segment_enc_sizes \
         FROM videos WHERE hash = $1 AND deleted_at IS NULL",
        &[&file_hash],
    ).await?;

    if let Some(r) = row {
        let key: Option<Vec<u8>> = r.get(2);
        return Ok(Some(FileRecord {
            name: r.get(0),
            ext: r.get(1),
            media_type: "video".to_string(),
            encryption_key: key.ok_or("p2p_encryption_key is NULL")?,
            encrypted_size: r.get(3),
            segment_count: r.get(4),
            segment_enc_sizes: r.get(5),
        }));
    }

    Ok(None)
}

pub(crate) fn restore_segmented(
    full_shards: Vec<Option<Vec<u8>>>,
    segment_enc_sizes: &[i64],
    key: &[u8],
) -> Result<Vec<u8>, Box<dyn std::error::Error + Send + Sync>> {
    let mut plaintext = Vec::new();
    let mut offset = 0usize;

    for &enc_size in segment_enc_sizes {
        let enc_size = enc_size as usize;
        let sub_shard_size = (enc_size + DATA_SHARDS - 1) / DATA_SHARDS;

        let segment_shards: Vec<Option<Vec<u8>>> = full_shards.iter().map(|opt| {
            opt.as_ref().map(|s| {
                let end = (offset + sub_shard_size).min(s.len());
                s[offset..end].to_vec()
            })
        }).collect();

        let segment_data = StorageEngine::restore_from_backup(segment_shards, enc_size, key)
            .map_err(|e| format!("Segment restore failed at offset {}: {}", offset, e))?;
        plaintext.extend_from_slice(&segment_data);
        offset += sub_shard_size;
    }

    Ok(plaintext)
}

pub(crate) fn parse_shard_hash(hex_str: &str) -> Option<[u8; 32]> {
    let bytes = hex::decode(hex_str).ok()?;
    if bytes.len() != 32 {
        return None;
    }
    let mut arr = [0u8; 32];
    arr.copy_from_slice(&bytes);
    Some(arr)
}

#[cfg(test)]
mod tests {
    use super::*;
    use np2p::storage::StorageEngine;

    // --- Helper: produce shards and concatenated full_shards from segments ---

    fn make_shards_for_segment(data: &[u8], key: &[u8]) -> (Vec<Vec<u8>>, usize) {
        StorageEngine::process_for_backup(data, key, key).expect("process_for_backup failed")
    }

    /// Builds the concatenated full-shard representation the Pi stores,
    /// given per-segment (shards, enc_size) tuples.
    fn concat_shards(segments: &[(Vec<Vec<u8>>, usize)]) -> Vec<Option<Vec<u8>>> {
        let mut full: Vec<Vec<u8>> = vec![Vec::new(); TOTAL_SHARDS];
        for (shards, enc_size) in segments {
            let sub_size = (enc_size + DATA_SHARDS - 1) / DATA_SHARDS;
            for (i, shard) in shards.iter().enumerate() {
                // Pad to exact sub_size (process_for_backup pads to multiple of DATA_SHARDS)
                let mut chunk = shard[..sub_size.min(shard.len())].to_vec();
                chunk.resize(sub_size, 0);
                full[i].extend_from_slice(&chunk);
            }
        }
        full.into_iter().map(Some).collect()
    }

    // -------------------------------------------------------------------------
    // parse_shard_hash
    // -------------------------------------------------------------------------

    #[test]
    fn test_parse_shard_hash_valid() {
        let arr = [0xABu8; 32];
        let hex = hex::encode(arr);
        assert_eq!(parse_shard_hash(&hex), Some(arr));
    }

    #[test]
    fn test_parse_shard_hash_wrong_length() {
        // 31 bytes → 62 hex chars
        let hex = hex::encode([0xABu8; 31]);
        assert_eq!(parse_shard_hash(&hex), None);
    }

    #[test]
    fn test_parse_shard_hash_invalid_hex() {
        assert_eq!(parse_shard_hash("not-hex-at-all"), None);
    }

    #[test]
    fn test_parse_shard_hash_empty() {
        assert_eq!(parse_shard_hash(""), None);
    }

    // -------------------------------------------------------------------------
    // restore_segmented — roundtrip
    // -------------------------------------------------------------------------

    #[test]
    fn test_restore_segmented_single_segment_roundtrip() {
        let key = [0x11u8; 32];
        let original = b"hello single segment";
        let (shards, enc_size) = make_shards_for_segment(original, &key);
        let full_shards: Vec<Option<Vec<u8>>> = shards.into_iter().map(Some).collect();

        let result = restore_segmented(full_shards, &[enc_size as i64], &key).unwrap();
        assert_eq!(result, original);
    }

    #[test]
    fn test_restore_segmented_three_segments_roundtrip() {
        let key = [0x22u8; 32];
        let seg0 = vec![1u8; 1000];
        let seg1 = vec![2u8; 2000];
        let seg2 = vec![3u8; 500];

        let s0 = make_shards_for_segment(&seg0, &key);
        let s1 = make_shards_for_segment(&seg1, &key);
        let s2 = make_shards_for_segment(&seg2, &key);

        let enc_sizes = vec![s0.1 as i64, s1.1 as i64, s2.1 as i64];
        let full_shards = concat_shards(&[s0, s1, s2]);

        let result = restore_segmented(full_shards, &enc_sizes, &key).unwrap();

        let mut expected = seg0.clone();
        expected.extend_from_slice(&seg1);
        expected.extend_from_slice(&seg2);
        assert_eq!(result, expected);
    }

    #[test]
    fn test_restore_segmented_uneven_sizes() {
        // Segments whose encrypted sizes are NOT multiples of DATA_SHARDS (padding edge case)
        let key = [0x33u8; 32];
        let seg0 = vec![0xAAu8; 101]; // enc size will not be divisible by 3
        let seg1 = vec![0xBBu8; 7];

        let s0 = make_shards_for_segment(&seg0, &key);
        let s1 = make_shards_for_segment(&seg1, &key);

        let enc_sizes = vec![s0.1 as i64, s1.1 as i64];
        let full_shards = concat_shards(&[s0, s1]);

        let result = restore_segmented(full_shards, &enc_sizes, &key).unwrap();

        let mut expected = seg0.clone();
        expected.extend_from_slice(&seg1);
        assert_eq!(result, expected);
    }

    #[test]
    fn test_restore_segmented_two_shards_missing() {
        // 3/5 shards present — just at the RS threshold
        let key = [0x44u8; 32];
        let original = vec![0xCCu8; 300];
        let (shards, enc_size) = make_shards_for_segment(&original, &key);

        let mut full_shards: Vec<Option<Vec<u8>>> = shards.into_iter().map(Some).collect();
        full_shards[0] = None; // lose shard 0
        full_shards[4] = None; // lose shard 4 (parity)

        let result = restore_segmented(full_shards, &[enc_size as i64], &key).unwrap();
        assert_eq!(result, original);
    }

    #[test]
    fn test_restore_segmented_too_few_shards() {
        let key = [0x55u8; 32];
        let original = vec![0xDDu8; 200];
        let (shards, enc_size) = make_shards_for_segment(&original, &key);

        let mut full_shards: Vec<Option<Vec<u8>>> = shards.into_iter().map(Some).collect();
        full_shards[0] = None;
        full_shards[1] = None;
        full_shards[2] = None; // only 2 left — below threshold

        let result = restore_segmented(full_shards, &[enc_size as i64], &key);
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("Not enough shards"));
    }

    #[test]
    fn test_restore_segmented_multi_segment_one_missing() {
        // 4/5 shards for a 2-segment file
        let key = [0x66u8; 32];
        let seg0 = vec![0xAAu8; 400];
        let seg1 = vec![0xBBu8; 600];

        let s0 = make_shards_for_segment(&seg0, &key);
        let s1 = make_shards_for_segment(&seg1, &key);

        let enc_sizes = vec![s0.1 as i64, s1.1 as i64];
        let mut full_shards = concat_shards(&[s0, s1]);
        full_shards[3] = None; // drop one parity shard

        let result = restore_segmented(full_shards, &enc_sizes, &key).unwrap();

        let mut expected = seg0.clone();
        expected.extend_from_slice(&seg1);
        assert_eq!(result, expected);
    }

    // restore_file_with_fetcher tests that need a real DB live in
    // tests/p2p_restore_test.rs (uses setup_test_database_with_instance).
}
