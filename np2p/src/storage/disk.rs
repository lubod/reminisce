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
        Ok(Self { base_path })
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

        fs::write(path, data).await?;
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
        fs::rename(temp_path, final_path).await?;
        Ok(())
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
}
