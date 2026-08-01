//! Core logic for restoring database snapshots from P2P storage.
//!
//! A full database loss also wipes the `db_backups` manifest table (it lives in
//! the DB being backed up). To make disaster recovery possible, every snapshot
//! also writes a self-contained JSON manifest to disk (`<p2p_data_dir>/db_manifests/<hash>.json`)
//! holding everything needed to restore without the DB: shard placement, node
//! addresses, segment layout, and the (master-key-encrypted) encryption key.
//!
//! Reused by the `disaster_recovery` CLI binary.

use np2p::storage::{StorageEngine, DATA_SHARDS, TOTAL_SHARDS};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

/// One shard's placement within a snapshot.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ManifestShard {
    pub index: usize,
    pub node_id: String,
    pub addr: String,
    pub shard_hash: String,
}

/// Self-contained restore manifest for one DB snapshot (stored on disk, outside the DB).
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct DbBackupManifest {
    pub backup_hash: String,
    pub created_at: String,
    pub size_bytes: i64,
    pub encrypted_size: i64,
    pub data_shards: i32,
    pub parity_shards: i32,
    pub segment_count: i32,
    pub segment_enc_sizes: Option<Vec<i64>>,
    /// Per-snapshot ChaCha20 key, encrypted with the account master key (hex).
    pub encryption_key_hex: String,
    pub shards: Vec<ManifestShard>,
}

/// Directory holding on-disk DB snapshot manifests.
pub fn manifest_dir(p2p_data_dir: &str) -> PathBuf {
    PathBuf::from(p2p_data_dir).join("db_manifests")
}

/// Well-known pinned-object name for the latest DB snapshot manifest on the mesh.
pub const MESH_LATEST_MANIFEST: &str = "reminisce:db-manifest:latest";

/// Pinned-object name for a specific snapshot manifest on the mesh.
pub fn mesh_manifest_name(backup_hash: &str) -> String {
    format!("reminisce:db-manifest:{}", backup_hash)
}

/// Symmetric key for encrypting manifests stored on the P2P mesh, derived from the
/// account master secret. Only the api_secret holder can read mesh manifests.
fn mesh_key(api_secret: &str) -> [u8; 32] {
    blake3::hash(format!("reminisce-db-manifest-mesh:{}", api_secret).as_bytes()).into()
}

/// Encrypt a manifest's JSON bytes for storage on the P2P mesh.
///
/// `nonce_ctx` must be unique per manifest (e.g. `backup_hash`) so that no two
/// manifests share a (key, nonce) pair — the mesh key is constant per deployment.
pub fn encrypt_for_mesh(json: &[u8], api_secret: &str, nonce_ctx: &[u8]) -> Result<Vec<u8>, String> {
    let key = mesh_key(api_secret);
    np2p::storage::encryption::encrypt(json, &key, nonce_ctx).map_err(|e| e.to_string())
}

/// Decrypt a manifest retrieved from the P2P mesh.
pub fn decrypt_from_mesh(data: &[u8], api_secret: &str) -> Result<Vec<u8>, String> {
    let key = mesh_key(api_secret);
    np2p::storage::encryption::decrypt(data, &key).map_err(|e| e.to_string())
}

/// Write a manifest to `<dir>/<backup_hash>.json` (mode 0600 — it contains a key).
pub fn write_manifest(dir: &Path, manifest: &DbBackupManifest) -> Result<PathBuf, String> {
    std::fs::create_dir_all(dir).map_err(|e| format!("create manifest dir failed: {}", e))?;
    let path = dir.join(format!("{}.json", manifest.backup_hash));
    let json = serde_json::to_string_pretty(manifest).map_err(|e| e.to_string())?;

    let mut options = std::fs::OpenOptions::new();
    options.create(true).write(true).truncate(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let mut file = options.open(&path).map_err(|e| format!("open manifest failed: {}", e))?;
    use std::io::Write;
    file.write_all(json.as_bytes()).map_err(|e| format!("write manifest failed: {}", e))?;
    Ok(path)
}

/// Read a single manifest from disk.
pub fn read_manifest(path: &Path) -> Result<DbBackupManifest, String> {
    let data = std::fs::read(path).map_err(|e| format!("read manifest {} failed: {}", path.display(), e))?;
    serde_json::from_slice(&data).map_err(|e| format!("parse manifest {} failed: {}", path.display(), e))
}

/// List all manifests in a directory, newest first (by created_at).
pub fn list_manifests(dir: &Path) -> Vec<DbBackupManifest> {
    let mut out = Vec::new();
    if let Ok(entries) = std::fs::read_dir(dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.extension().and_then(|e| e.to_str()) == Some("json") {
                if let Ok(m) = read_manifest(&path) {
                    out.push(m);
                }
            }
        }
    }
    out.sort_by(|a, b| b.created_at.cmp(&a.created_at));
    out
}

/// Delete the manifest file for a backup (called by retention pruning).
pub fn delete_manifest(dir: &Path, backup_hash: &str) {
    let _ = std::fs::remove_file(dir.join(format!("{}.json", backup_hash)));
}

/// Reconstruct and decrypt a DB snapshot from its manifest.
///
/// `fetch(node_id, shard_hash)` returns `Some(data)` on success (BLAKE3-verified
/// before use). Tolerates up to 2 missing shards (3/5 Reed-Solomon).
pub async fn restore_db_snapshot<F, Fut>(
    manifest: &DbBackupManifest,
    api_secret: &str,
    fetch: F,
) -> Result<Vec<u8>, String>
where
    F: Fn(String, [u8; 32]) -> Fut,
    Fut: std::future::Future<Output = Option<Vec<u8>>>,
{
    // Decrypt the per-snapshot key with the account master key.
    let encrypted_key = hex::decode(&manifest.encryption_key_hex)
        .map_err(|e| format!("invalid encryption_key_hex: {}", e))?;
    let key = crate::utils::decrypt_key(&encrypted_key, api_secret)?;

    let mut full_shards: Vec<Option<Vec<u8>>> = vec![None; TOTAL_SHARDS];
    for shard in &manifest.shards {
        let idx = shard.index;
        if idx >= TOTAL_SHARDS {
            continue;
        }
        let hash_bytes = match crate::p2p_restore::parse_shard_hash(&shard.shard_hash) {
            Some(b) => b,
            None => continue,
        };
        if let Some(data) = fetch(shard.node_id.clone(), hash_bytes).await {
            if blake3::hash(&data).to_hex().to_string() == shard.shard_hash {
                full_shards[idx] = Some(data);
            }
        }
    }

    let present = full_shards.iter().filter(|s| s.is_some()).count();
    if present < DATA_SHARDS {
        return Err(format!(
            "Only {}/{} shards available for snapshot {} — cannot restore (need ≥{})",
            present, TOTAL_SHARDS, manifest.backup_hash, DATA_SHARDS
        ));
    }

    let data_shards = manifest.data_shards.max(1) as usize;
    let parity_shards = manifest.parity_shards.max(0) as usize;

    if manifest.segment_count <= 1 {
        StorageEngine::restore_from_backup(
            full_shards,
            manifest.encrypted_size as usize,
            &key,
            data_shards,
            parity_shards,
        )
        .map_err(|e| format!("Snapshot restore failed: {}", e))
    } else {
        let enc_sizes = manifest
            .segment_enc_sizes
            .as_ref()
            .ok_or("segment_enc_sizes missing for multi-segment snapshot")?;
        crate::p2p_restore::restore_segmented(full_shards, enc_sizes, &key)
            .map_err(|e| e.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use np2p::storage::{PARITY_SHARDS, StorageEngine};
    use std::collections::HashMap;

    const API_SECRET: &str = "test_master_secret";

    fn manifest_for(
        key: &[u8; 32],
        size_bytes: i64,
        encrypted_size: i64,
        segment_count: i32,
        segment_enc_sizes: Option<Vec<i64>>,
        full_shards: &[Vec<u8>],
    ) -> DbBackupManifest {
        let encrypted_key = crate::utils::encrypt_key(key, API_SECRET).unwrap();
        DbBackupManifest {
            backup_hash: "deadbeef".to_string(),
            created_at: "2026-01-01T00:00:00Z".to_string(),
            size_bytes,
            encrypted_size,
            data_shards: DATA_SHARDS as i32,
            parity_shards: PARITY_SHARDS as i32,
            segment_count,
            segment_enc_sizes,
            encryption_key_hex: hex::encode(encrypted_key),
            shards: full_shards.iter().enumerate().map(|(i, s)| ManifestShard {
                index: i,
                node_id: format!("node{}", i),
                addr: "127.0.0.1:5050".to_string(),
                shard_hash: blake3::hash(s).to_hex().to_string(),
            }).collect(),
        }
    }

    /// Build a fetcher that serves shard data keyed by its BLAKE3 hash.
    fn fetcher_from(shards: &[Vec<u8>]) -> impl Fn(String, [u8; 32]) -> std::future::Ready<Option<Vec<u8>>> {
        let map: HashMap<[u8; 32], Vec<u8>> = shards.iter()
            .map(|s| (blake3::hash(s).into(), s.clone()))
            .collect();
        move |_node_id, hash| std::future::ready(map.get(&hash).cloned())
    }

    #[tokio::test]
    async fn test_restore_single_segment_roundtrip() {
        let key = [0x42u8; 32];
        let data = b"fake pg_dump custom-format contents";
        let (shards, enc_size) = StorageEngine::process_for_backup(data, &key, &key, DATA_SHARDS, PARITY_SHARDS).unwrap();
        let manifest = manifest_for(&key, data.len() as i64, enc_size as i64, 1, None, &shards);

        let restored = restore_db_snapshot(&manifest, API_SECRET, fetcher_from(&shards)).await.unwrap();
        assert_eq!(restored, data);
    }

    #[tokio::test]
    async fn test_restore_tolerates_two_missing_shards() {
        let key = [0x77u8; 32];
        let data = b"snapshot data with some shards lost";
        let (shards, enc_size) = StorageEngine::process_for_backup(data, &key, &key, DATA_SHARDS, PARITY_SHARDS).unwrap();
        let manifest = manifest_for(&key, data.len() as i64, enc_size as i64, 1, None, &shards);

        // Only 3 of 5 shards available (the RS threshold).
        let partial: Vec<Vec<u8>> = shards[0..3].to_vec();
        let restored = restore_db_snapshot(&manifest, API_SECRET, fetcher_from(&partial)).await.unwrap();
        assert_eq!(restored, data);
    }

    #[tokio::test]
    async fn test_restore_fails_below_threshold() {
        let key = [0x88u8; 32];
        let data = b"some data";
        let (shards, enc_size) = StorageEngine::process_for_backup(data, &key, &key, DATA_SHARDS, PARITY_SHARDS).unwrap();
        let manifest = manifest_for(&key, data.len() as i64, enc_size as i64, 1, None, &shards);

        // Only 2 shards available — below the 3/5 threshold.
        let partial: Vec<Vec<u8>> = shards[0..2].to_vec();
        let result = restore_db_snapshot(&manifest, API_SECRET, fetcher_from(&partial)).await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_restore_multi_segment_roundtrip() {
        let key = [0x99u8; 32];
        let seg0 = vec![1u8; 1000];
        let seg1 = vec![2u8; 700];

        // Encrypt + shard each segment independently (nonce context = key || seg_idx).
        let mut full: Vec<Vec<u8>> = vec![Vec::new(); TOTAL_SHARDS];
        let mut enc_sizes = Vec::new();
        for (seg_idx, seg) in [seg0.clone(), seg1.clone()].iter().enumerate() {
            let nonce_ctx: Vec<u8> = key.iter().chain((seg_idx as u32).to_le_bytes().iter()).cloned().collect();
            let (sub_shards, enc_size) = StorageEngine::process_for_backup(seg, &key, &nonce_ctx, DATA_SHARDS, PARITY_SHARDS).unwrap();
            let sub_size = enc_size.div_ceil(DATA_SHARDS);
            enc_sizes.push(enc_size as i64);
            for (i, shard) in sub_shards.iter().enumerate() {
                let mut chunk = shard[..sub_size.min(shard.len())].to_vec();
                chunk.resize(sub_size, 0);
                full[i].extend_from_slice(&chunk);
            }
        }

        let total_plain = (seg0.len() + seg1.len()) as i64;
        let manifest = manifest_for(&key, total_plain, 0, 2, Some(enc_sizes), &full);

        let restored = restore_db_snapshot(&manifest, API_SECRET, fetcher_from(&full)).await.unwrap();
        let mut expected = seg0.clone();
        expected.extend_from_slice(&seg1);
        assert_eq!(restored, expected);
    }

    #[test]
    fn test_manifest_write_list_read_roundtrip() {
        let dir = std::env::temp_dir().join(format!("reminisce_manifest_test_{}", uuid::Uuid::new_v4()));
        let key = [0xABu8; 32];
        let data = b"roundtrip";
        let (shards, enc_size) = StorageEngine::process_for_backup(data, &key, &key, DATA_SHARDS, PARITY_SHARDS).unwrap();
        let manifest = manifest_for(&key, data.len() as i64, enc_size as i64, 1, None, &shards);

        // Write → list → read.
        let path = write_manifest(&dir, &manifest).unwrap();
        assert!(path.exists());

        let listed = list_manifests(&dir);
        assert_eq!(listed.len(), 1);
        assert_eq!(listed[0].backup_hash, manifest.backup_hash);
        assert_eq!(listed[0].shards.len(), TOTAL_SHARDS);

        let read = read_manifest(&path).unwrap();
        assert_eq!(read.encryption_key_hex, manifest.encryption_key_hex);

        // delete_manifest removes the file.
        delete_manifest(&dir, &manifest.backup_hash);
        assert!(!path.exists());

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_mesh_encryption_roundtrip_and_unique_nonce() {
        let json = br#"{"backup_hash":"abc","shards":[]}"#;
        let encrypted = encrypt_for_mesh(json, API_SECRET, b"backup-a").unwrap();
        assert_ne!(encrypted, json);
        let decrypted = decrypt_from_mesh(&encrypted, API_SECRET).unwrap();
        assert_eq!(decrypted, json);

        // A different secret cannot decrypt it.
        assert!(decrypt_from_mesh(&encrypted, "wrong_secret").is_err());

        // Two manifests with the same key but different nonce contexts must not
        // produce identical ciphertexts (no nonce reuse).
        let encrypted2 = encrypt_for_mesh(json, API_SECRET, b"backup-b").unwrap();
        assert_ne!(encrypted, encrypted2);
    }

    #[test]
    fn test_mesh_manifest_names() {
        assert_eq!(MESH_LATEST_MANIFEST, "reminisce:db-manifest:latest");
        assert_eq!(mesh_manifest_name("deadbeef"), "reminisce:db-manifest:deadbeef");
    }
}
