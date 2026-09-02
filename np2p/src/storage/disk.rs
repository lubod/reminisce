use std::path::{Path, PathBuf};
use crate::error::Result;
use tokio::fs;
use tokio::io::AsyncWriteExt;

/// Manages local disk storage for encrypted shards.
/// Shards are stored in subdirectories based on their hash to avoid
/// having thousands of files in a single folder.
#[derive(Clone)]
pub struct DiskStorage {
    base_path: PathBuf,
}

/// Hard cap on the size of a single shard accepted via streaming upload.
/// Bounds disk usage per shard stream (defense against disk-exhaustion DoS).
/// 8 GiB accommodates very large videos (e.g. 11 GB DJI/phone footage produces
/// ~file_size/3 ≈ 3.8 GB shards under 3/5 EC). Shards are streamed to disk, not
/// held in RAM, so a larger bound only limits worst-case per-shard disk usage.
pub const MAX_SHARD_BYTES: u64 = 8 << 30; // 8 GiB

#[cfg(unix)]
fn get_available_space(path: &Path) -> std::io::Result<u64> {
    use std::ffi::CString;
    use std::os::unix::ffi::OsStrExt;

    let c_path = CString::new(path.as_os_str().as_bytes())?;
    unsafe {
        let mut stats: libc::statvfs = std::mem::zeroed();
        if libc::statvfs(c_path.as_ptr(), &mut stats) == 0 {
            Ok(stats.f_frsize * stats.f_bavail)
        } else {
            Err(std::io::Error::last_os_error())
        }
    }
}

#[cfg(not(unix))]
fn get_available_space(path: &Path) -> std::io::Result<u64> {
    // Free-space detection is not implemented on this platform: fail closed instead of
    // reporting unlimited space (which would silently disable the disk-exhaustion guard).
    let _ = path;
    Err(std::io::Error::new(
        std::io::ErrorKind::Unsupported,
        "free-space detection not implemented on this platform",
    ))
}

/// Refuse to write when free space is low or cannot be determined (fail-closed guard).
fn ensure_sufficient_space(base_path: &Path) -> Result<()> {
    match get_available_space(base_path) {
        Ok(avail) if avail < 100 * 1024 * 1024 => Err(crate::error::Np2pError::Storage(format!(
            "Insufficient disk space on node: {} bytes available",
            avail
        ))),
        Err(e) => Err(crate::error::Np2pError::Storage(format!(
            "Cannot determine free space on node: {}",
            e
        ))),
        Ok(_) => Ok(()),
    }
}

impl DiskStorage {
    /// Creates a new DiskStorage instance at the specified path.
    /// Ensures the directory exists.
    pub async fn new<P: AsRef<Path>>(path: P) -> Result<Self> {
        let base_path = path.as_ref().to_path_buf();
        if !base_path.exists() {
            fs::create_dir_all(&base_path).await?;
        }
        let storage = Self { base_path };
        // Purge any stale temp files left behind from aborted uploads on startup (> 1 hour old)
        let _ = storage.cleanup_stale_temp_files(std::time::Duration::from_secs(3600)).await;
        Ok(storage)
    }

    /// Reports free bytes on the storage volume (0 if it cannot be determined).
    /// Used to echo remaining space back in store responses so the home server can
    /// surface peer disk health without node_exporter on storage nodes.
    pub fn available_space(&self) -> u64 {
        get_available_space(&self.base_path).unwrap_or(0)
    }

    /// Returns the path to a shard file based on its hash.
    /// Uses the first 2 characters of the hex hash as a subdirectory.
    fn get_shard_path(&self, shard_hash: &[u8; 32]) -> PathBuf {
        let hash_hex = hex::encode(shard_hash);
        let (prefix, rest) = hash_hex.split_at(2);
        self.base_path.join(prefix).join(rest)
    }

    /// Returns the temp path used during a streaming shard upload.
    /// Keyed by (file_hash || shard_index) so concurrent uploads don't collide.
    pub fn temp_path(&self, temp_id: &[u8; 32]) -> PathBuf {
        let hash_hex = hex::encode(temp_id);
        let (prefix, rest) = hash_hex.split_at(2);
        self.base_path.join(prefix).join(format!("{}.tmp", rest))
    }

    /// Returns a unique sibling temp path for `final_path` (same directory, so
    /// the subsequent rename stays atomic on the same filesystem). Uniqueness
    /// comes from pid + wall-clock nanos + a process-local counter, so two
    /// writers (or two processes) can never interleave into one temp file.
    fn unique_temp_sibling(final_path: &Path) -> PathBuf {
        use std::sync::atomic::{AtomicU64, Ordering};
        static SEQ: AtomicU64 = AtomicU64::new(0);
        let seq = SEQ.fetch_add(1, Ordering::Relaxed);
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0);
        let mut name = final_path
            .file_name()
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_else(|| "shard".to_string());
        name.push_str(&format!(".{}.{}.{}.tmp", std::process::id(), nanos, seq));
        final_path.with_file_name(name)
    }

    /// Best-effort fsync of the parent directory of `path` so the rename that
    /// moved `path` into place is itself durable across power loss. Errors are
    /// ignored (e.g. platforms where directories cannot be opened as files).
    async fn sync_parent_dir(path: &Path) {
        #[cfg(unix)]
        if let Some(parent) = path.parent() {
            if let Ok(dir) = tokio::fs::File::open(parent).await {
                let _ = dir.sync_all().await;
            }
        }
        #[cfg(not(unix))]
        let _ = path;
    }

    /// Stores a shard on disk.
    pub async fn store(&self, shard_hash: [u8; 32], data: &[u8]) -> Result<()> {
        // Assert at least 100MB of free space is available (fails closed off-Unix).
        ensure_sufficient_space(&self.base_path)?;

        let path = self.get_shard_path(&shard_hash);

        // Ensure parent directory exists
        if let Some(parent) = path.parent() {
            if !parent.exists() {
                fs::create_dir_all(parent).await?;
            }
        }

        // Durable write: temp file in the same dir → flush → fsync → atomic rename
        // over the content-addressed final path. A crash mid-write can therefore
        // never leave a truncated/partial shard at the final path.
        let tmp_path = Self::unique_temp_sibling(&path);
        let result = async {
            let mut file = fs::File::create(&tmp_path).await?;
            file.write_all(data).await?;
            file.flush().await?;
            file.sync_all().await?;
            drop(file);
            fs::rename(&tmp_path, &path).await
        }
        .await;
        if let Err(e) = result {
            // Never leak the temp file on failure.
            let _ = fs::remove_file(&tmp_path).await;
            return Err(e.into());
        }

        // Persist the directory entry (best-effort).
        Self::sync_parent_dir(&path).await;
        Ok(())
    }

    /// Appends a chunk to the in-progress temp file for a streaming shard upload.
    pub async fn store_stream_chunk(&self, temp_path: &Path, data: &[u8]) -> Result<()> {
        // Reject once the shard stream exceeds the hard cap (bounds disk use).
        if data.len() as u64 > MAX_SHARD_BYTES {
            return Err(crate::error::Np2pError::Storage(format!(
                "Shard chunk exceeds MAX_SHARD_BYTES ({} bytes)", MAX_SHARD_BYTES
            )));
        }
        // Refuse to grow the temp file when disk space is low (same policy as `store`,
        // fails closed off-Unix).
        ensure_sufficient_space(&self.base_path)?;

        if let Some(parent) = temp_path.parent() {
            if !parent.exists() {
                fs::create_dir_all(parent).await?;
            }
        }
        let mut file = tokio::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(temp_path)
            .await?;
        file.write_all(data).await?;
        Ok(())
    }

    /// Moves the verified temp file to its content-addressed final path.
    /// The caller must verify the BLAKE3 hash before calling this.
    pub async fn finalize_stream_temp(&self, temp_path: &Path, shard_hash: [u8; 32]) -> Result<()> {
        let final_path = self.get_shard_path(&shard_hash);
        if let Some(parent) = final_path.parent() {
            if !parent.exists() {
                fs::create_dir_all(parent).await?;
            }
        }
        // Flush the streamed (append-written) chunks to stable storage BEFORE the
        // rename, so the final path never names a file whose bytes are not durable.
        let file = tokio::fs::File::open(temp_path).await?;
        file.sync_all().await?;
        drop(file);
        fs::rename(temp_path, &final_path).await?;
        // Persist the directory entry (best-effort).
        Self::sync_parent_dir(&final_path).await;
        Ok(())
    }

    /// Opens a shard file for streaming read, returning the File handle and its size in bytes.
    pub async fn open_shard_file(&self, shard_hash: [u8; 32]) -> Result<Option<(tokio::fs::File, u64)>> {
        let path = self.get_shard_path(&shard_hash);
        if !path.exists() {
            return Ok(None);
        }
        let metadata = tokio::fs::metadata(&path).await?;
        let file = tokio::fs::File::open(&path).await?;
        Ok(Some((file, metadata.len())))
    }

    /// Purges orphaned temp files (*.tmp) older than max_age (e.g. from aborted uploads or daemon restarts).
    pub async fn cleanup_stale_temp_files(&self, max_age: std::time::Duration) -> Result<u64> {
        let mut count = 0u64;
        let now = std::time::SystemTime::now();
        let mut entries = match fs::read_dir(&self.base_path).await {
            Ok(e) => e,
            Err(_) => return Ok(0),
        };
        while let Ok(Some(prefix_entry)) = entries.next_entry().await {
            let prefix_path = prefix_entry.path();
            if !prefix_path.is_dir() {
                continue;
            }
            let mut sub_entries = match fs::read_dir(&prefix_path).await {
                Ok(e) => e,
                Err(_) => continue,
            };
            while let Ok(Some(file_entry)) = sub_entries.next_entry().await {
                let file_path = file_entry.path();
                if let Some(file_name) = file_path.file_name().and_then(|n| n.to_str()) {
                    if file_name.ends_with(".tmp") {
                        if let Ok(metadata) = file_entry.metadata().await {
                            if let Ok(mtime) = metadata.modified() {
                                if let Ok(age) = now.duration_since(mtime) {
                                    if age >= max_age {
                                        if fs::remove_file(&file_path).await.is_ok() {
                                            count += 1;
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }
        if count > 0 {
            tracing::info!("Cleaned up {} stale temp shard file(s) from {}", count, self.base_path.display());
        }
        Ok(count)
    }

    /// Retrieves a shard from disk.
    pub async fn get(&self, shard_hash: [u8; 32]) -> Result<Option<Vec<u8>>> {
        let path = self.get_shard_path(&shard_hash);
        if !path.exists() {
            return Ok(None);
        }

        let data = fs::read(path).await?;
        Ok(Some(data))
    }

    /// Checks if a shard exists on disk.
    pub fn exists(&self, shard_hash: [u8; 32]) -> bool {
        self.get_shard_path(&shard_hash).exists()
    }

    /// Deletes a shard from disk.
    pub async fn delete(&self, shard_hash: [u8; 32]) -> Result<()> {
        let path = self.get_shard_path(&shard_hash);
        if path.exists() {
            fs::remove_file(path).await?;
        }
        Ok(())
    }

    /// Lists stored shard hashes, optionally filtered by a 2-hex-character prefix.
    pub async fn list_shards(&self, prefix: Option<&str>) -> Result<Vec<[u8; 32]>> {
        let mut hashes = Vec::new();
        let prefixes: Vec<String> = match prefix {
            Some(p) => vec![p.to_string()],
            None => (0..=255).map(|i| format!("{:02x}", i)).collect(),
        };

        for p in prefixes {
            let dir = self.base_path.join(&p);
            if !dir.exists() {
                continue;
            }
            let mut entries = match fs::read_dir(&dir).await {
                Ok(e) => e,
                Err(_) => continue,
            };
            while let Ok(Some(entry)) = entries.next_entry().await {
                let name = entry.file_name();
                let name_str = name.to_string_lossy();
                if name_str.ends_with(".tmp") || name_str.len() != 62 {
                    continue;
                }
                let full_hex = format!("{}{}", p, name_str);
                if let Ok(hash_bytes) = hex::decode(&full_hex) {
                    if let Ok(arr) = hash_bytes.try_into() {
                        hashes.push(arr);
                    }
                }
            }
        }
        hashes.sort();
        Ok(hashes)
    }

    /// Path for a name-addressed pinned object, keyed by blake3(name).
    fn pinned_path(&self, name: &str) -> PathBuf {
        let hash_hex = hex::encode(blake3::hash(name.as_bytes()).as_bytes());
        self.base_path.join("pinned").join(hash_hex)
    }

    /// Stores a pinned object by name (overwrites any existing value).
    /// Used for small critical metadata that must survive a home-server disk loss.
    pub async fn store_pinned(&self, name: &str, data: &[u8]) -> Result<()> {
        let path = self.pinned_path(name);
        if let Some(parent) = path.parent() {
            if !parent.exists() {
                fs::create_dir_all(parent).await?;
            }
        }
        fs::write(path, data).await?;
        Ok(())
    }

    /// Retrieves a pinned object by name.
    pub async fn get_pinned(&self, name: &str) -> Result<Option<Vec<u8>>> {
        let path = self.pinned_path(name);
        if !path.exists() {
            return Ok(None);
        }
        Ok(Some(fs::read(path).await?))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[tokio::test]
    async fn test_disk_storage_roundtrip() {
        let tmp = tempdir().unwrap();
        let storage = DiskStorage::new(tmp.path()).await.unwrap();

        let hash = [0xABu8; 32];
        let data = b"Some encrypted shard data";

        // Store
        storage.store(hash, data).await.unwrap();
        assert!(storage.exists(hash));

        // Get
        let retrieved = storage.get(hash).await.unwrap().expect("Shard missing");
        assert_eq!(retrieved, data);

        // Delete
        storage.delete(hash).await.unwrap();
        assert!(!storage.exists(hash));
    }

    #[tokio::test]
    async fn test_get_nonexistent() {
        let tmp = tempdir().unwrap();
        let storage = DiskStorage::new(tmp.path()).await.unwrap();
        let hash = [0xCDu8; 32];

        let result = storage.get(hash).await.unwrap();
        assert!(result.is_none());
    }

    #[tokio::test]
    async fn test_pinned_object_roundtrip_and_overwrite() {
        let tmp = tempdir().unwrap();
        let storage = DiskStorage::new(tmp.path()).await.unwrap();

        // Missing initially.
        assert!(storage.get_pinned("manifest:latest").await.unwrap().is_none());

        // Store + retrieve.
        storage.store_pinned("manifest:latest", b"v1").await.unwrap();
        assert_eq!(storage.get_pinned("manifest:latest").await.unwrap().unwrap(), b"v1");

        // Overwrite (the 'latest' pointer moves forward).
        storage.store_pinned("manifest:latest", b"v2").await.unwrap();
        assert_eq!(storage.get_pinned("manifest:latest").await.unwrap().unwrap(), b"v2");

        // Different names are independent.
        storage.store_pinned("manifest:abc", b"other").await.unwrap();
        assert_eq!(storage.get_pinned("manifest:abc").await.unwrap().unwrap(), b"other");
        assert_eq!(storage.get_pinned("manifest:latest").await.unwrap().unwrap(), b"v2");
    }

    #[tokio::test]
    async fn test_cleanup_stale_temp_files_and_open_shard() {
        let tmp = tempdir().unwrap();
        let storage = DiskStorage::new(tmp.path()).await.unwrap();

        let hash = [0x42u8; 32];
        let data = b"test shard content for streaming read";
        storage.store(hash, data).await.unwrap();

        // Verify open_shard_file
        let (mut file, size) = storage.open_shard_file(hash).await.unwrap().expect("shard exists");
        assert_eq!(size, data.len() as u64);
        use tokio::io::AsyncReadExt;
        let mut read_buf = Vec::new();
        file.read_to_end(&mut read_buf).await.unwrap();
        assert_eq!(read_buf, data);

        // Verify nonexistent open_shard_file
        assert!(storage.open_shard_file([0x99u8; 32]).await.unwrap().is_none());

        // Create a temp file inside a prefix directory
        let temp_path = storage.temp_path(&[0x11u8; 32]);
        if let Some(p) = temp_path.parent() {
            tokio::fs::create_dir_all(p).await.unwrap();
        }
        tokio::fs::write(&temp_path, b"temp data").await.unwrap();
        assert!(temp_path.exists());

        // Calling with max_age of 0 duration purges the file immediately
        let cleaned = storage.cleanup_stale_temp_files(std::time::Duration::from_secs(0)).await.unwrap();
        assert_eq!(cleaned, 1);
        assert!(!temp_path.exists());
    }
}
