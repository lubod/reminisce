//! Core logic for restoring media files from P2P storage.
//!
//! Fetches shards concurrently from their assigned nodes, tolerates up to 2 missing
//! shards (3/5 Reed-Solomon), then decrypts and reassembles the original file.
//! Handles both single-segment files and large multi-segment files.
//! Called by the HTTP handler (services/p2p_restore.rs) and the CLI binary.

use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use deadpool_postgres::Pool;
use np2p::crypto::NodeIdentity;
use np2p::network::P2PService;
use np2p::network::protocol::Message;
use np2p::storage::{StorageEngine, DATA_SHARDS, TOTAL_SHARDS};
use tokio::io::{AsyncReadExt, AsyncSeekExt, AsyncWriteExt};
use tracing::{info, warn};

/// Parse a CLI storage-node override in `node_id=host:port` form.
pub fn parse_node_override(s: &str) -> Result<(String, SocketAddr), String> {
    let (id, addr) = s.split_once('=').ok_or("expected node_id=host:port")?;
    let id = id.trim();
    if id.is_empty() {
        return Err("node_id must not be empty".to_string());
    }
    let addr = addr.parse::<SocketAddr>().map_err(|e| format!("invalid addr '{}': {}", addr, e))?;
    Ok((id.to_string(), addr))
}

/// Load the home server's P2P identity for CLI restore tools.
/// Prefers `<p2p_data_dir>/node.key`; if absent, derives deterministically from
/// `api_secret` matching the configured `p2p_identity_kdf` (`argon2id` or legacy).
pub fn load_cli_identity(
    p2p_data_dir: &str,
    api_secret: &str,
    kdf: Option<&str>,
) -> Result<NodeIdentity, Box<dyn std::error::Error + Send + Sync>> {
    let path = PathBuf::from(p2p_data_dir).join("node.key");
    if path.exists() {
        let bytes = std::fs::read(&path)
            .map_err(|e| format!("cannot read P2P identity {}: {}", path.display(), e))?;
        return Ok(NodeIdentity::from_secret_bytes(&bytes)?);
    }
    if api_secret.is_empty() {
        return Err(format!(
            "P2P identity key {} not found and --api-secret is empty",
            path.display()
        ).into());
    }
    match kdf {
        Some("argon2id") => {
            eprintln!("node.key not found at {} — deriving P2P identity via argon2id from api_secret", path.display());
            Ok(NodeIdentity::from_secret_hardened(api_secret))
        }
        _ => {
            eprintln!("node.key not found at {} — deriving legacy P2P identity from api_secret", path.display());
            Ok(NodeIdentity::from_secret(api_secret))
        }
    }
}

#[derive(Debug)]
pub struct RestoredFile {
    pub data: Vec<u8>,
    pub filename: String,
    pub media_type: String,
}

#[derive(Debug)]
pub struct RestoredFileMeta {
    pub filename: String,
    pub media_type: String,
    pub size_bytes: u64,
}

struct FileRecord {
    name: String,
    ext: String,
    media_type: String,
    encryption_key: Vec<u8>,
    encrypted_size: i64,
    segment_count: i32,
    segment_enc_sizes: Option<Vec<i64>>,
    data_shards: Option<i32>,
    parity_shards: Option<i32>,
}

/// Restore a file from P2P backup, fetching shards from peers via P2PService.
///
/// `api_secret` is the master secret used to unwrap the per-file ChaCha20 key stored
/// in `p2p_encryption_key` (which is wrapped via `utils::encrypt_key`).
///
/// `owner_user_id` scopes the lookup to one user's media. Pass `None` only for
/// admin callers and the local disaster-recovery CLI; the HTTP handler must always
/// pass `Some(user_id)` for non-admin roles (object-level authorization).
pub async fn restore_file(
    pool: &Pool,
    p2p_service: &Arc<P2PService>,
    file_hash: &str,
    api_secret: &str,
    owner_user_id: Option<&str>,
) -> Result<RestoredFile, Box<dyn std::error::Error + Send + Sync>> {
    let svc = p2p_service.clone();
    let pool_clone = pool.clone();
    restore_file_with_fetcher(pool, file_hash, owner_user_id, move |node_id, shard_hash| {
        let svc = svc.clone();
        let pool = pool_clone.clone();
        async move {
            let hex_hash = hex::encode(shard_hash);
            if let Some(addr) = crate::shard_rebalance_worker::lookup_node_addr(&pool, &svc, &node_id).await {
                if let Ok(data) = crate::p2p_upload::retrieve_shard(&svc, addr, &hex_hash).await {
                    return Some(data);
                }
            }
            let token = svc.identity().create_shard_token(np2p::crypto::ShardOp::Retrieve, &shard_hash);
            match svc.send_message(&node_id, &Message::RetrieveShardRequest { shard_hash, token }).await {
                Ok(Message::RetrieveShardResponse { data, .. }) => data,
                _ => None,
            }
        }
    }, api_secret).await
}

/// Restore a file from P2P backup directly to `out_path` (0600 on Unix), streaming
/// multi-segment files through disk so peak RAM stays bounded at ~256 MB even for multi-GB videos.
pub async fn restore_file_to_path(
    pool: &Pool,
    p2p_service: &Arc<P2PService>,
    file_hash: &str,
    api_secret: &str,
    owner_user_id: Option<&str>,
    out_path: &Path,
) -> Result<RestoredFileMeta, Box<dyn std::error::Error + Send + Sync>> {
    let client = pool.get().await?;
    let rec = query_file_record(&client, file_hash, owner_user_id, api_secret).await?
        .ok_or_else(|| format!("File {} not found in database", file_hash))?;

    if rec.segment_count <= 1 {
        let restored = restore_file(pool, p2p_service, file_hash, api_secret, owner_user_id).await?;
        write_private_file(out_path, &restored.data).await?;
        return Ok(RestoredFileMeta {
            filename: restored.filename,
            media_type: restored.media_type,
            size_bytes: restored.data.len() as u64,
        });
    }

    let data_shards = rec.data_shards.unwrap_or(DATA_SHARDS as i32).max(1) as usize;
    let parity_shards = rec.parity_shards.unwrap_or((TOTAL_SHARDS - DATA_SHARDS) as i32).max(0) as usize;
    let total_shards = data_shards + parity_shards;
    let enc_sizes = rec.segment_enc_sizes
        .ok_or("p2p_segment_enc_sizes is NULL for multi-segment file")?;

    let shard_rows = client.query(
        "SELECT shard_index, node_id, shard_hash \
         FROM p2p_shards WHERE file_hash = $1 ORDER BY shard_index",
        &[&file_hash],
    ).await?;
    if shard_rows.is_empty() {
        return Err(format!("No shards found for file {}", file_hash).into());
    }

    let scratch_dir = std::env::temp_dir().join(format!(
        "reminisce_restore_{}_{}",
        std::process::id(),
        chrono::Utc::now().timestamp_nanos_opt().unwrap_or(0)
    ));
    tokio::fs::create_dir_all(&scratch_dir).await?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = tokio::fs::set_permissions(&scratch_dir, std::fs::Permissions::from_mode(0o700)).await;
    }

    struct DirCleanup(PathBuf);
    impl Drop for DirCleanup {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }
    let _cleanup = DirCleanup(scratch_dir.clone());

    let mut shard_paths: Vec<Option<PathBuf>> = vec![None; total_shards];
    let mut present = 0usize;

    for row in &shard_rows {
        if present >= data_shards {
            break;
        }
        let shard_index: i32 = row.get(0);
        let node_id: String = row.get(1);
        let shard_hash_hex: String = row.get(2);
        let idx = shard_index as usize;
        if idx >= total_shards || shard_paths[idx].is_some() {
            continue;
        }
        let Some(hash_bytes) = parse_shard_hash(&shard_hash_hex) else {
            continue;
        };

        let part_path = scratch_dir.join(format!("shard_{}.bin", idx));
        let fetched = if let Some(addr) = crate::shard_rebalance_worker::lookup_node_addr(pool, p2p_service, &node_id).await {
            crate::p2p_upload::retrieve_shard_to_file(p2p_service, addr, &shard_hash_hex, &part_path).await.is_ok()
        } else {
            let token = p2p_service.identity().create_shard_token(np2p::crypto::ShardOp::Retrieve, &hash_bytes);
            if let Ok(Message::RetrieveShardResponse { data: Some(data), .. }) =
                p2p_service.send_message(&node_id, &Message::RetrieveShardRequest { shard_hash: hash_bytes, token }).await
            {
                if blake3::hash(&data).to_hex().as_str() == shard_hash_hex {
                    write_private_file(&part_path, &data).await.is_ok()
                } else {
                    false
                }
            } else {
                false
            }
        };
        if fetched {
            info!("Fetched shard {} to disk from node {}", idx, node_id);
            shard_paths[idx] = Some(part_path);
            present += 1;
        }
    }

    if present < data_shards {
        return Err(format!(
            "Only {}/{} shards available for {} — cannot restore (need ≥{})",
            present, total_shards, file_hash, data_shards
        ).into());
    }

    let mut out_file = create_private_file(out_path).await?;
    let mut offset = 0u64;
    let mut size_bytes = 0u64;

    for &enc_size in &enc_sizes {
        let enc_size_usize = enc_size as usize;
        let sub_shard_size = enc_size_usize.div_ceil(data_shards);
        let mut segment_shards: Vec<Option<Vec<u8>>> = vec![None; total_shards];

        for (i, path_opt) in shard_paths.iter().enumerate() {
            if let Some(p) = path_opt {
                let mut f = tokio::fs::File::open(p).await?;
                let file_len = f.metadata().await?.len();
                let start = offset.min(file_len);
                let to_read = ((sub_shard_size as u64).min(file_len - start)) as usize;
                f.seek(std::io::SeekFrom::Start(start)).await?;
                let mut buf = vec![0u8; to_read];
                f.read_exact(&mut buf).await?;
                segment_shards[i] = Some(buf);
            }
        }

        let segment_data = StorageEngine::restore_from_backup(
            segment_shards,
            enc_size_usize,
            &rec.encryption_key,
            data_shards,
            parity_shards,
        )
        .map_err(|e| format!("Segment restore failed at offset {}: {}", offset, e))?;
        out_file.write_all(&segment_data).await?;
        size_bytes += segment_data.len() as u64;
        offset += sub_shard_size as u64;
    }

    out_file.flush().await?;
    Ok(RestoredFileMeta {
        filename: format!("{}.{}", rec.name, rec.ext),
        media_type: rec.media_type,
        size_bytes,
    })
}

async fn create_private_file(path: &Path) -> std::io::Result<tokio::fs::File> {
    if let Some(parent) = path.parent() {
        tokio::fs::create_dir_all(parent).await?;
    }
    let mut opts = tokio::fs::OpenOptions::new();
    opts.write(true).create(true).truncate(true);
    #[cfg(unix)]
    opts.mode(0o600);
    opts.open(path).await
}

async fn write_private_file(path: &Path, data: &[u8]) -> std::io::Result<()> {
    let mut f = create_private_file(path).await?;
    f.write_all(data).await?;
    f.flush().await
}

/// Core restore logic with an injectable shard fetcher.
///
/// `fetch(node_id, shard_hash)` returns `Some(data)` on success, `None` if unavailable.
/// Results are BLAKE3-verified before use; a mismatch is treated as unavailable.
pub async fn restore_file_with_fetcher<F, Fut>(
    pool: &Pool,
    file_hash: &str,
    owner_user_id: Option<&str>,
    fetch: F,
    api_secret: &str,
) -> Result<RestoredFile, Box<dyn std::error::Error + Send + Sync>>
where
    F: Fn(String, [u8; 32]) -> Fut,
    Fut: std::future::Future<Output = Option<Vec<u8>>>,
{
    let client = pool.get().await?;

    let rec = query_file_record(&client, file_hash, owner_user_id, api_secret).await?
        .ok_or_else(|| format!("File {} not found in database", file_hash))?;

    let shard_rows = client.query(
        "SELECT shard_index, node_id, shard_hash \
         FROM p2p_shards WHERE file_hash = $1 ORDER BY shard_index",
        &[&file_hash],
    ).await?;

    if shard_rows.is_empty() {
        return Err(format!("No shards found for file {}", file_hash).into());
    }

    let data_shards = rec.data_shards.unwrap_or(DATA_SHARDS as i32).max(1) as usize;
    let parity_shards = rec.parity_shards.unwrap_or((TOTAL_SHARDS - DATA_SHARDS) as i32).max(0) as usize;
    let total_shards = data_shards + parity_shards;
    let mut full_shards: Vec<Option<Vec<u8>>> = vec![None; total_shards];
    let mut present = 0usize;

    for row in &shard_rows {
        if present >= data_shards {
            break;
        }
        let shard_index: i32 = row.get(0);
        let node_id: String = row.get(1);
        let shard_hash_hex: String = row.get(2);

        let idx = shard_index as usize;
        if idx >= total_shards {
            warn!("Shard index {} out of range, skipping", idx);
            continue;
        }
        if full_shards[idx].is_some() {
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
                    present += 1;
                } else {
                    warn!("Hash mismatch for shard {} from node {} — discarding", idx, node_id);
                }
            }
            None => {
                warn!("Shard {} unavailable from node {}", idx, node_id);
            }
        }
    }

    if present < data_shards {
        return Err(format!(
            "Only {}/{} shards available for {} — cannot restore (need ≥{})",
            present, total_shards, file_hash, data_shards
        ).into());
    }

    let plaintext = if rec.segment_count <= 1 {
        StorageEngine::restore_from_backup(full_shards, rec.encrypted_size as usize, &rec.encryption_key, data_shards, parity_shards)
            .map_err(|e| format!("Restore failed: {}", e))?
    } else {
        let enc_sizes = rec.segment_enc_sizes
            .ok_or("p2p_segment_enc_sizes is NULL for multi-segment file")?;
        restore_segmented(full_shards, &enc_sizes, &rec.encryption_key, data_shards, parity_shards)?
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
    owner_user_id: Option<&str>,
    api_secret: &str,
) -> Result<Option<FileRecord>, Box<dyn std::error::Error + Send + Sync>> {
    let owner_uuid = owner_user_id.and_then(|uid| uuid::Uuid::parse_str(uid).ok());
    let row = match owner_uuid {
        Some(uid) => client.query_opt(
            "SELECT name, ext, p2p_encryption_key, p2p_encrypted_size::BIGINT, \
             p2p_segment_count, p2p_segment_enc_sizes, p2p_data_shards, p2p_parity_shards \
             FROM images WHERE hash = $1 AND deleted_at IS NULL AND user_id = $2",
            &[&file_hash, &uid],
        ).await?,
        None => client.query_opt(
            "SELECT name, ext, p2p_encryption_key, p2p_encrypted_size::BIGINT, \
             p2p_segment_count, p2p_segment_enc_sizes, p2p_data_shards, p2p_parity_shards \
             FROM images WHERE hash = $1 AND deleted_at IS NULL",
            &[&file_hash],
        ).await?,
    };

    if let Some(r) = row {
        let key = r.get::<_, Option<Vec<u8>>>(2)
            .map(|k| crate::utils::decrypt_key(&k, api_secret))
            .transpose()?
            .ok_or("p2p_encryption_key is NULL")?;
        return Ok(Some(FileRecord {
            name: r.get(0),
            ext: r.get(1),
            media_type: "image".to_string(),
            encryption_key: key,
            encrypted_size: r.get(3),
            segment_count: r.get(4),
            segment_enc_sizes: r.get(5),
            data_shards: r.get(6),
            parity_shards: r.get(7),
        }));
    }

    let row = match owner_uuid {
        Some(uid) => client.query_opt(
            "SELECT name, ext, p2p_encryption_key, p2p_encrypted_size::BIGINT, \
             p2p_segment_count, p2p_segment_enc_sizes, p2p_data_shards, p2p_parity_shards \
             FROM videos WHERE hash = $1 AND deleted_at IS NULL AND user_id = $2",
            &[&file_hash, &uid],
        ).await?,
        None => client.query_opt(
            "SELECT name, ext, p2p_encryption_key, p2p_encrypted_size::BIGINT, \
             p2p_segment_count, p2p_segment_enc_sizes, p2p_data_shards, p2p_parity_shards \
             FROM videos WHERE hash = $1 AND deleted_at IS NULL",
            &[&file_hash],
        ).await?,
    };

    if let Some(r) = row {
        let key = r.get::<_, Option<Vec<u8>>>(2)
            .map(|k| crate::utils::decrypt_key(&k, api_secret))
            .transpose()?
            .ok_or("p2p_encryption_key is NULL")?;
        return Ok(Some(FileRecord {
            name: r.get(0),
            ext: r.get(1),
            media_type: "video".to_string(),
            encryption_key: key,
            encrypted_size: r.get(3),
            segment_count: r.get(4),
            segment_enc_sizes: r.get(5),
            data_shards: r.get(6),
            parity_shards: r.get(7),
        }));
    }

    Ok(None)
}

pub(crate) fn restore_segmented(
    full_shards: Vec<Option<Vec<u8>>>,
    segment_enc_sizes: &[i64],
    key: &[u8],
    data_shards: usize,
    parity_shards: usize,
) -> Result<Vec<u8>, Box<dyn std::error::Error + Send + Sync>> {
    let mut plaintext = Vec::new();
    let mut offset = 0usize;

    for &enc_size in segment_enc_sizes {
        let enc_size = enc_size as usize;
        let sub_shard_size = enc_size.div_ceil(data_shards);

        let segment_shards: Vec<Option<Vec<u8>>> = full_shards.iter().map(|opt| {
            opt.as_ref().map(|s| {
                let end = (offset + sub_shard_size).min(s.len());
                s[offset..end].to_vec()
            })
        }).collect();

        let segment_data = StorageEngine::restore_from_backup(segment_shards, enc_size, key, data_shards, parity_shards)
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
        StorageEngine::process_for_backup(data, key, key, DATA_SHARDS, TOTAL_SHARDS - DATA_SHARDS).expect("process_for_backup failed")
    }

    /// Builds the concatenated full-shard representation the Pi stores,
    /// given per-segment (shards, enc_size) tuples.
    fn concat_shards(segments: &[(Vec<Vec<u8>>, usize)]) -> Vec<Option<Vec<u8>>> {
        let mut full: Vec<Vec<u8>> = vec![Vec::new(); TOTAL_SHARDS];
        for (shards, enc_size) in segments {
            let sub_size = enc_size.div_ceil(DATA_SHARDS);
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

        let result = restore_segmented(full_shards, &[enc_size as i64], &key, DATA_SHARDS, TOTAL_SHARDS - DATA_SHARDS).unwrap();
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

        let result = restore_segmented(full_shards, &enc_sizes, &key, DATA_SHARDS, TOTAL_SHARDS - DATA_SHARDS).unwrap();

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

        let result = restore_segmented(full_shards, &enc_sizes, &key, DATA_SHARDS, TOTAL_SHARDS - DATA_SHARDS).unwrap();

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

        let result = restore_segmented(full_shards, &[enc_size as i64], &key, DATA_SHARDS, TOTAL_SHARDS - DATA_SHARDS).unwrap();
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

        let result = restore_segmented(full_shards, &[enc_size as i64], &key, DATA_SHARDS, TOTAL_SHARDS - DATA_SHARDS);
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

        let result = restore_segmented(full_shards, &enc_sizes, &key, DATA_SHARDS, TOTAL_SHARDS - DATA_SHARDS).unwrap();

        let mut expected = seg0.clone();
        expected.extend_from_slice(&seg1);
        assert_eq!(result, expected);
    }

    #[test]
    fn test_restore_segmented_custom_rs_params() {
        let key = [0x77u8; 32];
        let original = vec![0x5Au8; 512];
        let (shards, enc_size) = StorageEngine::process_for_backup(&original, &key, &key, 2, 1).unwrap();
        let mut full_shards: Vec<Option<Vec<u8>>> = shards.into_iter().map(Some).collect();
        full_shards[1] = None; // drop one shard out of (2+1)

        let result = restore_segmented(full_shards, &[enc_size as i64], &key, 2, 1).unwrap();
        assert_eq!(result, original);
    }

    // restore_file_with_fetcher tests that need a real DB live in
    // tests/p2p_restore_test.rs (uses setup_test_database_with_instance).
}
