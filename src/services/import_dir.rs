use actix_web::{post, get, web, HttpResponse, HttpRequest, Error};
use chrono::Utc;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use std::sync::Mutex;
use std::collections::HashMap;
use tokio::fs;
use blake3::Hasher;
use tokio::io::AsyncReadExt;
use log::{info, warn, error};
use utoipa::ToSchema;
use uuid::Uuid;
use crate::config::Config;
use crate::db::{MainDbPool, GeotaggingDbPool};
use crate::utils;
use crate::services::ingest;

pub type ImportJobStore = Mutex<HashMap<String, ImportJob>>;

#[derive(Clone, Serialize, ToSchema)]
#[serde(rename_all = "lowercase")]
pub enum JobStatus {
    Running,
    Done,
    Failed,
}

#[derive(Clone, Serialize, ToSchema)]
pub struct ImportJob {
    pub status: JobStatus,
    pub scanned: usize,
    pub imported: usize,
    pub failed: usize,
    pub errors: Vec<String>,
}

#[derive(Deserialize, ToSchema)]
pub struct ImportDirectoryRequest {
    pub path: String,
    pub recursive: bool,
    #[serde(default = "default_import_device_id")]
    pub device_id: String,
    /// "none" | "root" (label = root dir name) | "subdir" (label = each file's parent dir name)
    #[serde(default = "default_label_mode")]
    pub label_mode: String,
}

fn default_import_device_id() -> String {
    "server-import".to_string()
}

fn default_label_mode() -> String {
    "none".to_string()
}

#[derive(Serialize, ToSchema)]
pub struct StartImportResponse {
    pub job_id: String,
}

// Kept for OpenAPI schema compatibility
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
        (status = 202, description = "Import job started", body = StartImportResponse),
        (status = 400, description = "Bad request"),
        (status = 401, description = "Unauthorized"),
    )
)]
#[post("/import_directory")]
pub async fn import_directory(
    req: HttpRequest,
    body: web::Json<ImportDirectoryRequest>,
    pool: web::Data<MainDbPool>,
    geotagging_pool: web::Data<GeotaggingDbPool>,
    config: web::Data<Config>,
    job_store: web::Data<ImportJobStore>,
) -> Result<HttpResponse, Error> {
    let claims = match utils::authenticate_request(&req, "import_directory", config.get_api_key()) {
        Ok(claims) => claims,
        Err(resp) => return Ok(resp),
    };

    let user_uuid = utils::parse_user_uuid(&claims.user_id)?;

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
        return Ok(HttpResponse::BadRequest().json(serde_json::json!({
            "error": format!("Path does not exist: {:?}", root_path)
        })));
    }

    let job_id = Uuid::new_v4().to_string();
    {
        let mut store = job_store.lock().unwrap();
        store.insert(job_id.clone(), ImportJob {
            status: JobStatus::Running,
            scanned: 0,
            imported: 0,
            failed: 0,
            errors: vec![],
        });
    }

    let job_id_task = job_id.clone();
    let job_store_task = job_store.clone();
    let pool_task = pool.clone();
    let geotagging_pool_task = geotagging_pool.clone();
    let config_task = config.clone();
    let device_id = body.device_id.clone();
    let recursive = body.recursive;
    let label_mode = body.label_mode.clone();

    tokio::spawn(async move {
        run_import(
            root_path, recursive, device_id, user_uuid, label_mode,
            pool_task, geotagging_pool_task, config_task,
            job_store_task, job_id_task,
        ).await;
    });

    Ok(HttpResponse::Accepted().json(StartImportResponse { job_id }))
}

async fn run_import(
    root_path: PathBuf,
    recursive: bool,
    device_id: String,
    user_uuid: Uuid,
    label_mode: String,
    pool: web::Data<MainDbPool>,
    geotagging_pool: web::Data<GeotaggingDbPool>,
    config: web::Data<Config>,
    job_store: web::Data<ImportJobStore>,
    job_id: String,
) {
    let mut files_to_process = Vec::new();
    let mut stack = vec![root_path.clone()];

    while let Some(path) = stack.pop() {
        if path.is_dir() {
            if recursive || path == root_path {
                let mut entries = match fs::read_dir(&path).await {
                    Ok(e) => e,
                    Err(e) => { warn!("Failed to read dir {:?}: {}", path, e); continue; }
                };
                while let Ok(Some(entry)) = entries.next_entry().await {
                    let p = entry.path();
                    if p.is_dir() { stack.push(p); } else { files_to_process.push(p); }
                }
            }
        }
    }

    let total_scanned = files_to_process.len();
    let mut imported = 0usize;
    let mut failed = 0usize;
    let mut errors: Vec<String> = Vec::new();

    update_job(&job_store, &job_id, |job| { job.scanned = total_scanned; });

    // label_cache: name → id, shared across all modes to avoid redundant DB hits
    let mut label_cache: std::collections::HashMap<String, i32> = std::collections::HashMap::new();

    for chunk in files_to_process.chunks(100) {
        let mut chunk_hashes: Vec<(PathBuf, String, String, bool, Option<chrono::DateTime<Utc>>)> = Vec::new();

        for path in chunk {
            let extension = path.extension().and_then(|s| s.to_str()).unwrap_or("").to_lowercase();
            let is_image = ["jpg", "jpeg", "png", "webp", "gif"].contains(&extension.as_str());
            let is_video = ["mp4", "mov", "avi", "mkv"].contains(&extension.as_str());
            if !is_image && !is_video { continue; }

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
            let file_mtime: Option<chrono::DateTime<Utc>> = std::fs::metadata(&path)
                .ok()
                .and_then(|m| m.modified().ok())
                .map(chrono::DateTime::from);
            chunk_hashes.push((path.clone(), hash, name, is_image, file_mtime));
        }

        if chunk_hashes.is_empty() { continue; }

        let client = match pool.0.get().await {
            Ok(c) => c,
            Err(e) => { error!("DB pool error: {}", e); continue; }
        };

        let image_hashes: Vec<String> = chunk_hashes.iter()
            .filter(|(_, _, _, is_img, _)| *is_img)
            .map(|(_, h, _, _, _)| h.clone()).collect();
        let video_hashes: Vec<String> = chunk_hashes.iter()
            .filter(|(_, _, _, is_img, _)| !*is_img)
            .map(|(_, h, _, _, _)| h.clone()).collect();

        let mut existing_image_hashes = std::collections::HashSet::new();
        if !image_hashes.is_empty() {
            let mut placeholders = Vec::new();
            for i in 2..=image_hashes.len() + 1 { placeholders.push(format!("${}", i)); }
            let query = format!(
                "SELECT hash FROM images WHERE deviceid = $1 AND hash IN ({}) AND deleted_at IS NULL",
                placeholders.join(", ")
            );
            let mut params: Vec<&(dyn tokio_postgres::types::ToSql + Sync)> = vec![&device_id];
            for h in &image_hashes { params.push(h); }
            if let Ok(rows) = client.query(&query[..], &params[..]).await {
                for row in rows { existing_image_hashes.insert(row.get::<_, String>(0)); }
            }
        }

        let mut existing_video_hashes = std::collections::HashSet::new();
        if !video_hashes.is_empty() {
            let mut placeholders = Vec::new();
            for i in 2..=video_hashes.len() + 1 { placeholders.push(format!("${}", i)); }
            let query = format!(
                "SELECT hash FROM videos WHERE deviceid = $1 AND hash IN ({}) AND deleted_at IS NULL",
                placeholders.join(", ")
            );
            let mut params: Vec<&(dyn tokio_postgres::types::ToSql + Sync)> = vec![&device_id];
            for h in &video_hashes { params.push(h); }
            if let Ok(rows) = client.query(&query[..], &params[..]).await {
                for row in rows { existing_video_hashes.insert(row.get::<_, String>(0)); }
            }
        }

        for (path, hash, name, is_image, file_mtime) in chunk_hashes {
            let already_exists = if is_image {
                existing_image_hashes.contains(&hash)
            } else {
                existing_video_hashes.contains(&hash)
            };

            if !already_exists {
                info!("Importing file: {:?}", path);

                let res = if is_image {
                    ingest::process_image_file(
                        &path, &name, &hash, &device_id, &user_uuid,
                        &pool, &geotagging_pool, &config, false, file_mtime,
                    ).await
                } else {
                    ingest::process_video_file(
                        &path, &name, &hash, &device_id, &user_uuid,
                        &pool, &config, false, file_mtime,
                    ).await
                };

                if let Err(e) = res {
                    warn!("Failed to ingest {:?}: {}", path, e);
                    errors.push(format!("Failed to import {:?}: {}", path, e));
                    failed += 1;
                    continue;
                }
            }

            // Attach labels for both newly imported and already-existing files
            imported += 1;
            let label_names = labels_for_file(&path, &root_path, &label_mode);
            for name in label_names {
                let lid = if let Some(&id) = label_cache.get(&name) {
                    Some(id)
                } else {
                    let id = get_or_create_label(&pool, &user_uuid, &name).await;
                    if let Some(id) = id { label_cache.insert(name, id); }
                    id
                };
                if let Some(lid) = lid {
                    attach_label(&pool, &hash, &device_id, lid, is_image).await;
                }
            }
        }

        // Update progress after each chunk
        let (imp, fail, errs) = (imported, failed, errors.clone());
        update_job(&job_store, &job_id, move |job| {
            job.imported = imp;
            job.failed = fail;
            job.errors = errs.into_iter().take(20).collect();
        });
    }

    update_job(&job_store, &job_id, move |job| {
        job.status = JobStatus::Done;
        job.imported = imported;
        job.failed = failed;
        job.errors = errors.into_iter().take(20).collect();
    });
}

/// Returns the label name(s) to apply to a file based on label_mode.
///
/// - "root"       → [root dir name]                           e.g. "photos"
/// - "subdir"     → [immediate parent dir name]               e.g. "beach"
/// - "path"       → [relative path root→parent as one label]  e.g. "2023/vacation/beach"
/// - "components" → [one label per path component]            e.g. ["2023","vacation","beach"]
fn labels_for_file(path: &PathBuf, root_path: &PathBuf, label_mode: &str) -> Vec<String> {
    match label_mode {
        "root" => root_path
            .file_name()
            .map(|n| vec![n.to_string_lossy().to_string()])
            .unwrap_or_default(),

        "subdir" => path
            .parent()
            .and_then(|p| p.file_name())
            .map(|n| vec![n.to_string_lossy().to_string()])
            .unwrap_or_default(),

        "path" => {
            let rel = path.parent()
                .and_then(|p| p.strip_prefix(root_path).ok())
                .map(|p| p.to_string_lossy().replace('\\', "/"))
                .unwrap_or_default();
            if rel.is_empty() { vec![] } else { vec![rel] }
        }

        "components" => path
            .parent()
            .and_then(|p| p.strip_prefix(root_path).ok())
            .map(|rel| {
                rel.components()
                    .filter_map(|c| match c {
                        std::path::Component::Normal(s) => Some(s.to_string_lossy().to_string()),
                        _ => None,
                    })
                    .collect()
            })
            .unwrap_or_default(),

        _ => vec![],
    }
}

async fn get_or_create_label(pool: &web::Data<MainDbPool>, user_id: &Uuid, name: &str) -> Option<i32> {
    match pool.0.get().await {
        Ok(client) => {
            client.query_one(
                "INSERT INTO labels (user_id, name, color)
                 VALUES ($1, $2, '#3B82F6')
                 ON CONFLICT (user_id, name) DO UPDATE SET name = EXCLUDED.name
                 RETURNING id",
                &[user_id, &name],
            ).await.ok().map(|row| row.get(0))
        }
        Err(e) => { error!("DB pool error creating label: {}", e); None }
    }
}

async fn attach_label(pool: &web::Data<MainDbPool>, hash: &str, device_id: &str, label_id: i32, is_image: bool) {
    let table = if is_image { "image_labels" } else { "video_labels" };
    let hash_col = if is_image { "image_hash" } else { "video_hash" };
    let dev_col = if is_image { "image_deviceid" } else { "video_deviceid" };
    let query = format!(
        "INSERT INTO {} ({}, {}, label_id) VALUES ($1, $2, $3) ON CONFLICT DO NOTHING",
        table, hash_col, dev_col
    );
    match pool.0.get().await {
        Ok(client) => { let _ = client.execute(&query, &[&hash, &device_id, &label_id]).await; }
        Err(e) => error!("DB pool error attaching label: {}", e),
    }
}

fn update_job<F: FnOnce(&mut ImportJob)>(store: &web::Data<ImportJobStore>, job_id: &str, f: F) {
    if let Ok(mut map) = store.lock() {
        if let Some(job) = map.get_mut(job_id) {
            f(job);
        }
    }
}

#[utoipa::path(
    get,
    path = "/import_directory/status/{job_id}",
    responses(
        (status = 200, description = "Job status", body = ImportJob),
        (status = 404, description = "Job not found"),
        (status = 401, description = "Unauthorized"),
    )
)]
#[get("/import_directory/status/{job_id}")]
pub async fn get_import_status(
    req: HttpRequest,
    path: web::Path<String>,
    config: web::Data<Config>,
    job_store: web::Data<ImportJobStore>,
) -> Result<HttpResponse, Error> {
    match utils::authenticate_request(&req, "get_import_status", config.get_api_key()) {
        Ok(_) => {}
        Err(resp) => return Ok(resp),
    };

    let job_id = path.into_inner();
    let store = job_store.lock().unwrap();
    match store.get(&job_id) {
        Some(job) => Ok(HttpResponse::Ok().json(job)),
        None => Ok(HttpResponse::NotFound().json(serde_json::json!({"error": "Job not found"}))),
    }
}
