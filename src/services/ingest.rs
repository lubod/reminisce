use std::path::{Path};
use std::fs;
use std::io;

/// Move a file, falling back to copy+delete if rename fails due to cross-device link (EXDEV).
/// This happens in Docker when /tmp and the storage volume are on different mount points.
fn rename_or_copy(from: &Path, to: &Path) -> io::Result<()> {
    match fs::rename(from, to) {
        Ok(()) => Ok(()),
        Err(e) if e.raw_os_error() == Some(18) => {
            // EXDEV: cross-device link — fall back to copy + delete
            fs::copy(from, to)?;
            fs::remove_file(from)?;
            Ok(())
        }
        Err(e) => Err(e),
    }
}
use chrono::{Utc};
use crate::config::Config;
use crate::db::{MainDbPool, GeotaggingDbPool};
use serde::{Serialize, Deserialize};
use uuid::Uuid;
use actix_web::web::Data;

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct IngestResult {
    pub hash: String,
    pub status: String,
    pub name: String,
    pub filename: String,
    pub path: String,
    pub thumbnail: bool,
    pub thumbnail_generated: bool,
}

pub async fn process_image_file(
    temp_path: &Path,
    name: &str,
    hash: &str,
    device_id: &str,
    user_id: &Uuid,
    pool: &Data<MainDbPool>,
    _geotagging_pool: &Data<GeotaggingDbPool>,
    config: &Config,
    move_file: bool
) -> Result<IngestResult, Box<dyn std::error::Error + Send + Sync>> {
    let ext = Path::new(name).extension().and_then(|s| s.to_str()).unwrap_or("jpg");
    let filename = format!("{}.{}", hash, ext);
    
    // 1. Target Path
    let images_dir = config.get_images_dir();
    let sub_dir = &hash[0..2];
    let target_dir = Path::new(images_dir).join(sub_dir);
    if !target_dir.exists() {
        fs::create_dir_all(&target_dir)?;
    }
    let target_path = target_dir.join(&filename);

    // 2. Move or copy
    if move_file {
        rename_or_copy(temp_path, &target_path)?;
    } else {
        fs::copy(temp_path, &target_path)?;
    }

    // 3. Database
    let client = pool.0.get().await?;
    let created_at = Utc::now();
    
    client.execute(
        "INSERT INTO images (deviceid, hash, user_id, name, ext, created_at, added_at)
         VALUES ($1, $2, $3, $4, $5, $6, NOW())
         ON CONFLICT (deviceid, hash) DO UPDATE SET deleted_at = NULL",
        &[&device_id, &hash, user_id, &name, &ext, &created_at]
    ).await?;

    Ok(IngestResult {
        hash: hash.to_string(),
        status: "success".to_string(),
        name: name.to_string(),
        filename,
        path: target_path.to_string_lossy().to_string(),
        thumbnail: false,
        thumbnail_generated: false,
    })
}

pub async fn process_video_file(
    temp_path: &Path,
    name: &str,
    hash: &str,
    device_id: &str,
    user_id: &Uuid,
    pool: &Data<MainDbPool>,
    config: &Config,
    move_file: bool
) -> Result<IngestResult, Box<dyn std::error::Error + Send + Sync>> {
    let ext = Path::new(name).extension().and_then(|s| s.to_str()).unwrap_or("mp4");
    let filename = format!("{}.{}", hash, ext);
    
    let videos_dir = config.get_videos_dir();
    let sub_dir = &hash[0..2];
    let target_dir = Path::new(videos_dir).join(sub_dir);
    if !target_dir.exists() {
        fs::create_dir_all(&target_dir)?;
    }
    let target_path = target_dir.join(&filename);

    if move_file {
        rename_or_copy(temp_path, &target_path)?;
    } else {
        fs::copy(temp_path, &target_path)?;
    }

    let client = pool.0.get().await?;
    client.execute(
        "INSERT INTO videos (deviceid, hash, user_id, name, ext, created_at, added_at)
         VALUES ($1, $2, $3, $4, $5, NOW(), NOW())
         ON CONFLICT (deviceid, hash) DO UPDATE SET deleted_at = NULL",
        &[&device_id, &hash, user_id, &name, &ext]
    ).await?;

    Ok(IngestResult {
        hash: hash.to_string(),
        status: "success".to_string(),
        name: name.to_string(),
        filename,
        path: target_path.to_string_lossy().to_string(),
        thumbnail: false,
        thumbnail_generated: false,
    })
}
