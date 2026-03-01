use std::path::{Path, PathBuf};
use crate::error::Result;
use tokio::fs;

/// Manages local disk storage for encrypted shards.
/// Shards are stored in subdirectories based on their hash to avoid 
/// having thousands of files in a single folder.
#[derive(Clone)]
pub struct DiskStorage {
    base_path: PathBuf,
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

    /// Stores a shard on disk.
    pub async fn store(&self, shard_hash: [u8; 32], data: &[u8]) -> Result<()> {
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
}
