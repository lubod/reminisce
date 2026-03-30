use std::path::{Path};
use std::fs;
use std::io;
use std::io::BufReader;

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
use crate::utils;
use serde::{Serialize, Deserialize};
use serde_json;
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

/// Extract EXIF metadata from an image file.
/// Returns a JSON object mapping tag names to their display-value strings.
/// ASCII-type fields have their surrounding double-quotes stripped
/// (kamadak_exif wraps ASCII values in quotes in display_value()).
fn extract_exif_from_image(path: &Path) -> Option<serde_json::Value> {
    // Only attempt for JPEG/TIFF-based formats that kamadak_exif supports
    let ext = path.extension()?.to_str()?.to_lowercase();
    if !matches!(ext.as_str(), "jpg" | "jpeg" | "tiff" | "tif" | "heic" | "heif") {
        return None;
    }

    let file = fs::File::open(path).ok()?;
    let mut reader = BufReader::new(file);
    let exif = kamadak_exif::Reader::new()
        .read_from_container(&mut reader)
        .ok()?;

    let mut map = serde_json::Map::new();
    for field in exif.fields() {
        // Skip thumbnail IFD (In(1)) to avoid overwriting primary image fields
        if field.ifd_num == kamadak_exif::In(1) {
            continue;
        }
        let tag_name = field.tag.to_string();
        // Ascii fields: display_value() wraps content in double quotes — strip them
        // so we store "HONOR" not '"HONOR"' in the JSON
        let value_str = match &field.value {
            kamadak_exif::Value::Ascii(_) => {
                field.display_value().to_string().trim_matches('"').to_string()
            }
            _ => field.display_value().to_string(),
        };
        map.insert(tag_name, serde_json::Value::String(value_str));
    }

    if map.is_empty() { None } else { Some(serde_json::Value::Object(map)) }
}

pub async fn process_image_file(
    temp_path: &Path,
    name: &str,
    hash: &str,
    device_id: &str,
    user_id: &Uuid,
    pool: &Data<MainDbPool>,
    geotagging_pool: &Data<GeotaggingDbPool>,
    config: &Config,
    move_file: bool,
    client_created_at: Option<chrono::DateTime<Utc>>,
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

    // Store file size
    if let Ok(meta) = fs::metadata(&target_path) {
        let file_size = meta.len().min(i32::MAX as u64) as i32;
        let _ = client.execute(
            "UPDATE images SET file_size_bytes = $1 WHERE deviceid = $2 AND hash = $3",
            &[&file_size, &device_id, &hash],
        ).await;
    }

    // 4. Extract EXIF from the image file and update the DB
    let mut exif_date: Option<chrono::DateTime<Utc>> = None;
    let mut exif_json_opt: Option<serde_json::Value> = None;
    if let Some(exif_json) = extract_exif_from_image(&target_path) {
        let exif_str = exif_json.to_string();

        // Store EXIF only if not already set (client metadata upload takes priority)
        let _ = client.execute(
            "UPDATE images SET exif = $1 WHERE deviceid = $2 AND hash = $3 AND exif IS NULL",
            &[&exif_str, &device_id, &hash],
        ).await;

        // Store orientation separately so thumbnail generation can avoid re-reading the file
        let orientation: Option<i16> = exif_json.get("Orientation")
            .and_then(|v| v.as_str())
            .and_then(|s| s.parse::<i16>().ok());
        if let Some(orient) = orientation {
            let _ = client.execute(
                "UPDATE images SET orientation = $1 WHERE deviceid = $2 AND hash = $3 AND orientation IS NULL",
                &[&orient, &device_id, &hash],
            ).await;
        }

        if let Some(dt_str) = exif_json.get("DateTimeOriginal").and_then(|v| v.as_str()) {
            if let Ok(ndt) = chrono::NaiveDateTime::parse_from_str(dt_str, "%Y-%m-%d %H:%M:%S") {
                exif_date = Some(ndt.and_utc());
            }
        }
        exif_json_opt = Some(exif_json);
    }

    // 5. Pick best date: EXIF > filename > client (apply only if created_at is still upload time)
    let best_date = exif_date
        .or_else(|| utils::parse_date_from_image_name(name))
        .or(client_created_at);

    if let Some(dt) = best_date {
        let _ = client.execute(
            "UPDATE images SET created_at = $1 \
             WHERE deviceid = $2 AND hash = $3 \
             AND created_at > NOW() - INTERVAL '1 minute'",
            &[&dt, &device_id, &hash],
        ).await;
    }

    // 6. Extract GPS and update location + place (only if not already set)
    if let Some(ref exif_json) = exif_json_opt {
        if let Some((lat, lon)) = utils::extract_gps_coordinates(exif_json) {
            let updated = client.execute(
                "UPDATE images SET location = ST_SetSRID(ST_MakePoint($1, $2), 4326) \
                 WHERE deviceid = $3 AND hash = $4 AND location IS NULL",
                &[&lon, &lat, &device_id, &hash],
            ).await.unwrap_or(0);

            if updated > 0 {
                if let Some(place) = utils::reverse_geocode(
                    lat, lon, geotagging_pool,
                    config.enable_local_geocoding,
                    config.enable_external_geocoding_fallback,
                ).await {
                    let _ = client.execute(
                        "UPDATE images SET place = $1 WHERE deviceid = $2 AND hash = $3 AND place IS NULL",
                        &[&place, &device_id, &hash],
                    ).await;
                }
            }
        }
    }

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
    move_file: bool,
    client_created_at: Option<chrono::DateTime<Utc>>,
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

    // Store file size
    if let Ok(meta) = fs::metadata(&target_path) {
        let file_size = meta.len() as i64;
        let _ = client.execute(
            "UPDATE videos SET file_size_bytes = $1 WHERE deviceid = $2 AND hash = $3",
            &[&file_size, &device_id, &hash],
        ).await;
    }

    // Best date: filename > client (apply only if created_at is still upload time)
    let best_date = utils::parse_date_from_video_name(name)
        .or(client_created_at);

    if let Some(dt) = best_date {
        let _ = client.execute(
            "UPDATE videos SET created_at = $1 \
             WHERE deviceid = $2 AND hash = $3 \
             AND created_at > NOW() - INTERVAL '1 minute'",
            &[&dt, &device_id, &hash],
        ).await;
    }

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
