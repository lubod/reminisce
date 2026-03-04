use actix_web::{post, web, HttpResponse, HttpRequest, Error};
use chrono::Utc;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use tokio::fs;
use blake3::Hasher;
use tokio::io::AsyncReadExt;
use log::{info, warn, error};
use utoipa::ToSchema;
use crate::config::Config;
use crate::db::{MainDbPool, GeotaggingDbPool};
use crate::utils;
use crate::services::ingest;

#[derive(Deserialize, ToSchema)]
pub struct ImportDirectoryRequest {
    pub path: String,
    pub recursive: bool,
    #[serde(default = "default_import_device_id")]
    pub device_id: String,
}

fn default_import_device_id() -> String {
    "server-import".to_string()
}

#[derive(Serialize, ToSchema)]
pub struct ImportDirectoryResponse {
    pub scanned: usize,
    pub imported: usize,
    pub failed: usize,
    pub errors: Vec<String>,
}

#[utoipa::path(
    post,
    path = "/import_directory",
    request_body = ImportDirectoryRequest,
    responses(
        (status = 200, description = "Import complete", body = ImportDirectoryResponse),
        (status = 400, description = "Bad request"),
        (status = 401, description = "Unauthorized"),
        (status = 500, description = "Internal server error")
    )
)]
#[post("/import_directory")]
pub async fn import_directory(
    req: HttpRequest,
    body: web::Json<ImportDirectoryRequest>,
    pool: web::Data<MainDbPool>,
    geotagging_pool: web::Data<GeotaggingDbPool>,
    config: web::Data<Config>,
) -> Result<HttpResponse, Error> {
    let claims = match utils::authenticate_request(&req, "import_directory", config.get_api_key()) {
        Ok(claims) => claims,
        Err(resp) => return Ok(resp),
    };

    info!("Received import request for path: {}", body.path);

    let path_string = if body.path.starts_with("~") {
        if let Ok(home) = std::env::var("HOME") {
            body.path.replacen("~", &home, 1)
        } else {
            body.path.clone()
        }
    } else {
        body.path.clone()
    };

    let root_path = PathBuf::from(&path_string);
    if !root_path.exists() {
         warn!("Import path does not exist: {:?}", root_path);
         return Ok(HttpResponse::BadRequest().json(serde_json::json!({"error": format!("Path does not exist: {:?}", root_path)})));
    }

    let mut files_to_process = Vec::new();
    let mut stack = vec![root_path.clone()];

    // Simple recursive collection
    while let Some(path) = stack.pop() {
        if path.is_dir() {
            if body.recursive || path == root_path {
                let mut entries = match fs::read_dir(&path).await {
                    Ok(e) => e,
                    Err(e) => {
                        warn!("Failed to read dir {:?}: {}", path, e);
                        continue;
                    }
                };
                while let Ok(Some(entry)) = entries.next_entry().await {
                    let p = entry.path();
                    if p.is_dir() {
                        stack.push(p);
                    } else {
                        files_to_process.push(p);
                    }
                }
            }
        }
    }

    let total_scanned = files_to_process.len();
    let mut imported = 0;
    let mut failed = 0;
    let mut errors = Vec::new();

    let user_uuid = utils::parse_user_uuid(&claims.user_id)?;

    let chunks = files_to_process.chunks(100);

    for chunk in chunks {
        let mut chunk_hashes: Vec<(PathBuf, String, String, bool, Option<chrono::DateTime<Utc>>)> = Vec::new();

        for path in chunk {
            let extension = path.extension().and_then(|s| s.to_str()).unwrap_or("").to_lowercase();
            let is_image = ["jpg", "jpeg", "png", "webp", "gif"].contains(&extension.as_str());
            let is_video = ["mp4", "mov", "avi", "mkv"].contains(&extension.as_str());
            
            if !is_image && !is_video {
                continue;
            }

            let mut file = match fs::File::open(&path).await {
                Ok(f) => f,
                Err(e) => {
                    errors.push(format!("Failed to open {:?}: {}", path, e));
                    failed += 1;
                    continue;
                }
            };
            
            let mut hasher = Hasher::new();
            let mut buffer = [0u8; 8192];
            loop {
                 let n = match file.read(&mut buffer).await {
                     Ok(n) if n == 0 => break,
                     Ok(n) => n,
                     Err(_) => break,
                 };
                 hasher.update(&buffer[..n]);
            }
            let hash = hasher.finalize().to_hex().to_string();
            let name = path.file_name().unwrap_or_default().to_string_lossy().to_string();
            // File mtime as a date fallback (used when no EXIF and no parseable filename)
            let file_mtime: Option<chrono::DateTime<Utc>> = std::fs::metadata(&path)
                .ok()
                .and_then(|m| m.modified().ok())
                .map(chrono::DateTime::from);
            chunk_hashes.push((path.clone(), hash, name, is_image, file_mtime));
        }

        if chunk_hashes.is_empty() {
            continue;
        }

        // Batch check existing hashes for this device
        let client = utils::get_db_client(&pool.0).await?;

        let image_hashes: Vec<String> = chunk_hashes.iter().filter(|(_, _, _, is_img, _)| *is_img).map(|(_, h, _, _, _)| h.clone()).collect();
        let video_hashes: Vec<String> = chunk_hashes.iter().filter(|(_, _, _, is_img, _)| !*is_img).map(|(_, h, _, _, _)| h.clone()).collect();

        let mut existing_image_hashes = std::collections::HashSet::new();
        if !image_hashes.is_empty() {
            let mut placeholders = Vec::new();
            for i in 2..=image_hashes.len() + 1 { placeholders.push(format!("${}", i)); }
            let query = format!("SELECT hash FROM images WHERE deviceid = $1 AND hash IN ({}) AND deleted_at IS NULL", placeholders.join(", "));
            let mut params: Vec<&(dyn tokio_postgres::types::ToSql + Sync)> = vec![&body.device_id];
            for h in &image_hashes { params.push(h); }
            let rows = client.query(&query[..], &params[..]).await.map_err(|e| {
                error!("Failed to query existing images: {}", e);
                actix_web::error::ErrorInternalServerError("Database query error")
            })?;
            for row in rows { existing_image_hashes.insert(row.get::<_, String>(0)); }
        }

        let mut existing_video_hashes = std::collections::HashSet::new();
        if !video_hashes.is_empty() {
            let mut placeholders = Vec::new();
            for i in 2..=video_hashes.len() + 1 { placeholders.push(format!("${}", i)); }
            let query = format!("SELECT hash FROM videos WHERE deviceid = $1 AND hash IN ({}) AND deleted_at IS NULL", placeholders.join(", "));
            let mut params: Vec<&(dyn tokio_postgres::types::ToSql + Sync)> = vec![&body.device_id];
            for h in &video_hashes { params.push(h); }
            let rows = client.query(&query[..], &params[..]).await.map_err(|e| {
                error!("Failed to query existing videos: {}", e);
                actix_web::error::ErrorInternalServerError("Database query error")
            })?;
            for row in rows { existing_video_hashes.insert(row.get::<_, String>(0)); }
        }

        for (path, hash, name, is_image, file_mtime) in chunk_hashes {
            let already_exists = if is_image { existing_image_hashes.contains(&hash) } else { existing_video_hashes.contains(&hash) };

            if already_exists {
                imported += 1;
                continue;
            }

            info!("Importing file: {:?}", path);

            let res = if is_image {
                 ingest::process_image_file(
                     &path, &name, &hash, &body.device_id, &user_uuid,
                     &pool, &geotagging_pool, &config, false, file_mtime,
                 ).await
            } else {
                 ingest::process_video_file(
                     &path, &name, &hash, &body.device_id, &user_uuid,
                     &pool, &config, false, file_mtime,
                 ).await
            };

            match res {
                Ok(_) => imported += 1,
                Err(e) => {
                    warn!("Failed to ingest {:?}: {}", path, e);
                    errors.push(format!("Failed to import {:?}: {}", path, e));
                    failed += 1;
                }
            }
        }
    }

    Ok(HttpResponse::Ok().json(ImportDirectoryResponse {
        scanned: total_scanned,
        imported,
        failed,
        errors: errors.into_iter().take(20).collect(),
    }))
}
