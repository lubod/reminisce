use actix_files;
use actix_web::{ get, post, web, HttpRequest, HttpResponse };
use base64::{Engine as _, engine::general_purpose};
use log::{ error, info, warn };
use serde::{Serialize, Deserialize};
use serde_json;
use utoipa::{ToSchema, IntoParams};
use std::collections::HashSet;
use std::path::Path;

use crate::config::Config;
use crate::utils;
use crate::db::{MainDbPool, GeotaggingDbPool};
use crate::services::ingest;

#[utoipa::path(
    get,
    path = "/image/{image_hash}",
    responses(
        (status = 200, description = "Image found"),
        (status = 401, description = "Unauthorized"),
        (status = 404, description = "Image not found"),
        (status = 500, description = "Internal server error")
    )
)]
#[get("/image/{image_hash}")]
pub async fn get_image(
    req: HttpRequest,
    path: web::Path<String>,
    pool: web::Data<MainDbPool>,
    config: web::Data<Config>,
) -> Result<HttpResponse, actix_web::Error> {
    let claims = match utils::authenticate_request(&req, "get_image", config.get_api_key()).await {
        Ok(claims) => claims,
        Err(response) => return Ok(response),
    };

    let hash_to_find = path.into_inner();
    let client = utils::get_db_client(&pool.0).await?;
    let user_uuid = utils::parse_user_uuid(&claims.user_id)?;

    let row = client
        .query_opt(
            "SELECT name, place, ext, orientation, (exif IS NULL) AS no_exif FROM images WHERE user_id = $1 AND hash = $2 AND deleted_at IS NULL LIMIT 1",
            &[&user_uuid, &hash_to_find]
        ).await
        .map_err(|e| {
            error!("Failed to query image from database: {}", e);
            actix_web::error::ErrorInternalServerError("Failed to retrieve image info")
        })?;

    if let Some(row) = row {
        let original_name: String = row.get(0);
        let place: Option<String> = row.get(1);
        let extension: String = row.get(2);
        let orientation: Option<i16> = row.get(3);
        let no_exif: bool = row.get(4);

        let image_path = match crate::media_utils::safe_resolve_content_path(
            config.get_images_dir(),
            &hash_to_find,
            &extension,
        ) {
            Ok(p) => p,
            Err(e) => {
                error!(
                    "Unsafe or missing image path for hash '{}': {}",
                    hash_to_find, e
                );
                return Ok(HttpResponse::NotFound().body("Image not found"));
            }
        };

        // Guess the MIME type from the file extension for the Content-Type header.
        let mime_type = mime_guess::from_path(&image_path).first_or_octet_stream();

        match tokio::fs::read(&image_path).await {
            Ok(data) => {
                // Orientation correction on serve:
                // - JPEG: guarantee the file carries the DB-verified orientation
                //   tag (inject minimal EXIF, splice into existing EXIF, or
                //   rewrite a stale tag). Browsers then render it upright.
                //   Covers photos whose files never had an Orientation tag but
                //   whose orientation was fixed by AI detection later.
                // - PNG (no EXIF container): rotate pixels losslessly instead.
                let data = match orientation.filter(|o| (2..=8).contains(o)) {
                    Some(o) => {
                        let ext_lc = extension.to_lowercase();
                        if ext_lc == "jpg" || ext_lc == "jpeg" {
                            crate::media_utils::ensure_exif_orientation(&data, o as u16)
                        } else if no_exif && ext_lc == "png" {
                            crate::media_utils::rotate_png_bytes(&data, o as u16).unwrap_or(data)
                        } else {
                            data
                        }
                    }
                    None => data,
                };
                info!("Serving image: {:?}", image_path);
                // Only allow-listed media extensions may render inline from our
                // origin; anything else (e.g. a name ending in .html/.svg that
                // slipped into the DB historically) is forced to download as
                // inert octet-stream so it cannot host active content.
                let ext_lc = extension.to_lowercase();
                let inline_safe = crate::services::ingest::IMAGE_EXTS.contains(&ext_lc.as_str());
                let disposition = if inline_safe {
                    format!("inline; filename=\"{}\"", original_name)
                } else {
                    "attachment".to_string()
                };
                let mut response = HttpResponse::Ok();
                if inline_safe {
                    response.content_type(mime_type.as_ref());
                } else {
                    response.content_type("application/octet-stream");
                }
                response.insert_header(("Content-Disposition", disposition));

                // Add place as a custom header if available
                if let Some(place_value) = place {
                    response.insert_header(("X-Image-Place", place_value));
                }

                Ok(response.body(data))
            }
            Err(e) => {
                error!(
                    "Local image file not found. Hash: '{}', Path: {:?}, Error: {}",
                    hash_to_find,
                    image_path,
                    e
                );
                Err(actix_web::error::ErrorInternalServerError("Could not read image file."))
            }
        }
    } else {
        warn!("Image not found for hash: '{}'", hash_to_find);
        Ok(
            HttpResponse::NotFound().json(
                serde_json::json!({"status": "error", "message": "Image not found."})
            )
        )
    }
}

#[utoipa::path(
    get,
    path = "/video/{video_hash}",
    responses(
        (status = 200, description = "Video found"),
        (status = 401, description = "Unauthorized"),
        (status = 404, description = "Video not found"),
        (status = 500, description = "Internal server error")
    )
)]
#[get("/video/{video_hash}")]
pub async fn get_video(
    req: HttpRequest,
    path: web::Path<String>,
    pool: web::Data<MainDbPool>,
    config: web::Data<Config>,
) -> Result<HttpResponse, actix_web::Error> {
    let claims = match utils::authenticate_request(&req, "get_video", config.get_api_key()).await {
        Ok(claims) => claims,
        Err(response) => return Ok(response),
    };

    let hash_to_find = path.into_inner();
    let client = utils::get_db_client(&pool.0).await?;
    let user_uuid = utils::parse_user_uuid(&claims.user_id)?;

    let row = client
        .query_opt(
            "SELECT name, ext FROM videos WHERE user_id = $1 AND hash = $2 AND deleted_at IS NULL LIMIT 1",
            &[&user_uuid, &hash_to_find]
        ).await
        .map_err(|e| {
            error!("Failed to query video from database: {}", e);
            actix_web::error::ErrorInternalServerError("Failed to retrieve video info")
        })?;

    if let Some(row) = row {
        let original_name: String = row.get(0);
        let extension: String = row.get(1);

        let video_path = match crate::media_utils::safe_resolve_content_path(
            config.get_videos_dir(),
            &hash_to_find,
            &extension,
        ) {
            Ok(p) => p,
            Err(e) => {
                error!(
                    "Unsafe or missing video path for hash '{}': {}",
                    hash_to_find, e
                );
                return Ok(HttpResponse::NotFound().body("Video not found"));
            }
        };

        // Guess the MIME type from the file extension for the Content-Type header.
        let mime_type = mime_guess::from_path(&video_path).first_or_octet_stream();

        match actix_files::NamedFile::open(&video_path) {
            Ok(file) => {
                info!("Serving video: {:?}", video_path);
                // Same inline-safe defense as images: only allow-listed video
                // extensions may render inline from our origin; anything else is
                // forced to download as inert octet-stream.
                const INLINE_SAFE_VIDEO_EXTS: &[&str] = &["mp4", "mov", "webm", "m4v"];
                let ext_lc = extension.to_lowercase();
                if INLINE_SAFE_VIDEO_EXTS.contains(&ext_lc.as_str()) {
                    Ok(file
                        .set_content_type(mime_type)
                        .set_content_disposition(actix_web::http::header::ContentDisposition {
                            disposition: actix_web::http::header::DispositionType::Inline,
                            parameters: vec![actix_web::http::header::DispositionParam::Filename(original_name.clone())],
                        })
                        .into_response(&req))
                } else {
                    Ok(file
                        .set_content_type(mime_guess::mime::APPLICATION_OCTET_STREAM)
                        .set_content_disposition(actix_web::http::header::ContentDisposition {
                            disposition: actix_web::http::header::DispositionType::Attachment,
                            parameters: vec![actix_web::http::header::DispositionParam::Filename(original_name.clone())],
                        })
                        .into_response(&req))
                }
            }
            Err(e) => {
                error!(
                    "Local video file not found. Hash: '{}', Path: {:?}, Error: {}",
                    hash_to_find,
                    video_path,
                    e
                );
                Err(actix_web::error::ErrorInternalServerError("Could not open video file for streaming."))
            }
        }
    } else {
        warn!("Video not found for hash: '{}'", hash_to_find);
        Ok(
            HttpResponse::NotFound().json(
                serde_json::json!({"status": "error", "message": "Video not found."})
            )
        )
    }
}

/// Derive a display orientation label from stored dimensions + EXIF-style
/// orientation. Orientations 5-8 swap the sensor width/height, so the
/// *effective* dimensions decide Landscape vs Portrait vs Square.
/// Effective display dimensions after applying the EXIF-style rotation.
/// Orientations 5-8 swap the sensor's width/height.
fn effective_dimensions(
    width: Option<i32>,
    height: Option<i32>,
    orientation: Option<i16>,
) -> Option<(i64, i64)> {
    let (w, h) = match (width, height) {
        (Some(w), Some(h)) if w > 0 && h > 0 => (w as i64, h as i64),
        _ => return None,
    };
    let swaps = matches!(orientation, Some(5..=8));
    Some(if swaps { (h, w) } else { (w, h) })
}

/// "W × H" of the DISPLAYED image (post-rotation), e.g. `"3000 × 4000"`.
pub fn resolution_label(
    width: Option<i32>,
    height: Option<i32>,
    orientation: Option<i16>,
) -> Option<String> {
    effective_dimensions(width, height, orientation)
        .map(|(ew, eh)| format!("{ew} × {eh}"))
}

pub fn orientation_label(
    width: Option<i32>,
    height: Option<i32>,
    orientation: Option<i16>,
) -> Option<String> {
    let (ew, eh) = effective_dimensions(width, height, orientation)?;
    const TOL: i64 = 5; // percent tolerance around square
    if (ew - eh).abs() * 100 <= eh * TOL {
        Some("Square".to_string())
    } else if ew > eh {
        Some("Landscape".to_string())
    } else {
        Some("Portrait".to_string())
    }
}

#[derive(Serialize, ToSchema)]
#[schema(example = json!({
    "hash": "somehash",
    "name": "IMG_20231222_101010.jpg",
    "description": "A beautiful sunset over the mountains",
    "place": "Paris, France",
    "created_at": "2025-01-01T12:00:00Z",
    "exif": "{...}",
    "starred": false,
    "device_id": "pixel-7",
    "file_size_bytes": 4567890,
    "width": 4032,
    "height": 3024,
    "orientation": 6,
    "orientation_label": "Portrait",
    "resolution_label": "3024 × 4032",
    "media_type": "image"
}))]
pub struct ImageMetadata {
    pub hash: String,
    pub name: String,
    pub description: Option<String>,
    pub place: Option<String>,
    pub created_at: String,
    pub exif: Option<String>,
    pub starred: bool,
    pub device_id: Option<String>,
    pub file_size_bytes: Option<i64>,
    pub width: Option<i32>,
    pub height: Option<i32>,
    /// Raw EXIF-style orientation value (1-8). NULL when unknown.
    pub orientation: Option<i16>,
    /// Display orientation derived from effective dimensions after the
    /// stored orientation is applied ("Landscape" / "Portrait" / "Square").
    #[serde(skip_serializing_if = "Option::is_none")]
    pub orientation_label: Option<String>,
    /// Displayed resolution "W × H" after rotation is applied.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub resolution_label: Option<String>,
    pub media_type: Option<String>,
}

#[utoipa::path(
    get,
    path = "/image/{image_hash}/metadata",
    responses(
        (status = 200, description = "Image metadata found", body = ImageMetadata),
        (status = 401, description = "Unauthorized"),
        (status = 404, description = "Image not found"),
        (status = 500, description = "Internal server error")
    )
)]
#[get("/image/{image_hash}/metadata")]
pub async fn get_image_metadata(
    req: HttpRequest,
    path: web::Path<String>,
    pool: web::Data<MainDbPool>,
    config: web::Data<Config>
) -> Result<HttpResponse, actix_web::Error> {
    let claims = match utils::authenticate_request(&req, "get_image_metadata", config.get_api_key()).await {
        Ok(claims) => claims,
        Err(response) => return Ok(response),
    };

    let hash_to_find = path.into_inner();
    let user_uuid = utils::parse_user_uuid(&claims.user_id)?;
    let client = utils::get_db_client(&pool.0).await?;

    let img_row = client
        .query_opt(
            "SELECT i.hash, i.name, i.description, i.place, i.created_at, i.exif, 
             CASE WHEN s.hash IS NOT NULL THEN true ELSE false END as starred, 
             i.deviceid, i.file_size_bytes, i.width, i.height, i.orientation 
             FROM images i 
             LEFT JOIN starred_images s ON i.hash = s.hash AND s.user_id = $1 
             WHERE i.user_id = $1 AND i.hash = $2 AND i.deleted_at IS NULL LIMIT 1",
            &[&user_uuid, &hash_to_find]
        ).await
        .map_err(|e| {
            error!("Failed to query image metadata from database: {:?}", e);
            actix_web::error::ErrorInternalServerError("Failed to retrieve image metadata")
        })?;

    if let Some(row) = img_row {
        let file_size: Option<i32> = row.get(8);
        let metadata = ImageMetadata {
            hash: row.get(0),
            name: row.get(1),
            description: row.get(2),
            place: row.get(3),
            created_at: row.get::<_, chrono::DateTime<chrono::Utc>>(4).to_rfc3339(),
            exif: row.get(5),
            starred: row.get(6),
            device_id: row.get(7),
            file_size_bytes: file_size.map(|v| v as i64),
            width: row.get(9),
            height: row.get(10),
            orientation: row.get(11),
            orientation_label: orientation_label(row.get(9), row.get(10), row.get(11)),
            resolution_label: resolution_label(row.get(9), row.get(10), row.get(11)),
            media_type: Some("image".to_string()),
        };

        info!("Serving metadata for image: {}", hash_to_find);
        return Ok(HttpResponse::Ok().json(metadata));
    }

    let vid_row = client
        .query_opt(
            "SELECT v.hash, v.name, v.description, NULL::text as place, v.created_at, NULL::text as exif, 
             CASE WHEN s.hash IS NOT NULL THEN true ELSE false END as starred, 
             v.deviceid, v.file_size_bytes, NULL::integer as width, NULL::integer as height, NULL::smallint as orientation 
             FROM videos v 
             LEFT JOIN starred_videos s ON v.hash = s.hash AND s.user_id = $1 
             WHERE v.user_id = $1 AND v.hash = $2 AND v.deleted_at IS NULL LIMIT 1",
            &[&user_uuid, &hash_to_find]
        ).await
        .map_err(|e| {
            error!("Failed to query video metadata from database: {:?}", e);
            actix_web::error::ErrorInternalServerError("Failed to retrieve video metadata")
        })?;

    if let Some(row) = vid_row {
        let file_size: Option<i64> = row.get(8);
        let metadata = ImageMetadata {
            hash: row.get(0),
            name: row.get(1),
            description: row.get(2),
            place: row.get(3),
            created_at: row.get::<_, chrono::DateTime<chrono::Utc>>(4).to_rfc3339(),
            exif: row.get(5),
            starred: row.get(6),
            device_id: row.get(7),
            file_size_bytes: file_size,
            width: row.get(9),
            height: row.get(10),
            orientation: None,
            orientation_label: None,
            resolution_label: None,
            media_type: Some("video".to_string()),
        };

        info!("Serving metadata for video: {}", hash_to_find);
        return Ok(HttpResponse::Ok().json(metadata));
    }

    warn!("Media not found for hash: '{}'", hash_to_find);
    Ok(
        HttpResponse::NotFound().json(
            serde_json::json!({"status": "error", "message": "Media not found."})
        )
    )
}

#[derive(Serialize, ToSchema)]
#[schema(example = json!({
    "hash": "somehash",
    "starred": true
}))]
pub struct StarResponse {
    pub hash: String,
    pub starred: bool,
}

/// Shared implementation for toggling star status on images or videos.
async fn toggle_media_star_inner(
    pool: &deadpool_postgres::Pool,
    media_table: &str,
    starred_table: &str,
    hash: &str,
    user_uuid: &uuid::Uuid,
) -> Result<HttpResponse, actix_web::Error> {
    crate::utils::validate_table_name(media_table).map_err(actix_web::error::ErrorBadRequest)?;
    crate::utils::validate_table_name(starred_table).map_err(actix_web::error::ErrorBadRequest)?;
    let mut client = utils::get_db_client(pool).await?;

    let transaction = client.transaction().await.map_err(|e| {
        error!("Failed to start transaction: {}", e);
        actix_web::error::ErrorInternalServerError("Database error")
    })?;

    // Verify the media exists and user has access
    let exists = transaction
        .query_opt(
            &format!("SELECT 1 FROM {} WHERE user_id = $1 AND hash = $2 AND deleted_at IS NULL LIMIT 1", media_table),
            &[user_uuid, &hash]
        )
        .await
        .map_err(|e| {
            error!("Failed to check {} existence: {}", media_table, e);
            actix_web::error::ErrorInternalServerError("Database error")
        })?
        .is_some();

    if !exists {
        warn!("{} not found or access denied for hash: '{}'", media_table, hash);
        return Ok(HttpResponse::NotFound().json(
            serde_json::json!({"status": "error", "message": format!("{} not found.", media_table.trim_end_matches('s'))})
        ));
    }

    // Check current starred status
    let is_starred = transaction
        .query_opt(
            &format!("SELECT 1 FROM {} WHERE user_id = $1 AND hash = $2", starred_table),
            &[user_uuid, &hash]
        )
        .await
        .map_err(|e| {
            error!("Failed to check starred status: {}", e);
            actix_web::error::ErrorInternalServerError("Database error")
        })?
        .is_some();

    // Toggle
    let new_starred_status = if is_starred {
        transaction
            .execute(
                &format!("DELETE FROM {} WHERE user_id = $1 AND hash = $2", starred_table),
                &[user_uuid, &hash]
            )
            .await
            .map_err(|e| {
                error!("Failed to unstar: {}", e);
                actix_web::error::ErrorInternalServerError("Failed to update star status")
            })?;
        false
    } else {
        transaction
            .execute(
                &format!("INSERT INTO {} (user_id, hash) VALUES ($1, $2)", starred_table),
                &[user_uuid, &hash]
            )
            .await
            .map_err(|e| {
                error!("Failed to star: {}", e);
                actix_web::error::ErrorInternalServerError("Failed to update star status")
            })?;
        true
    };

    transaction.commit().await.map_err(|e| {
        error!("Failed to commit transaction: {}", e);
        actix_web::error::ErrorInternalServerError("Failed to save star status")
    })?;

    info!("{} {} starred status set to: {}", media_table, hash, new_starred_status);

    Ok(HttpResponse::Ok().json(StarResponse {
        hash: hash.to_string(),
        starred: new_starred_status,
    }))
}

#[utoipa::path(
    post,
    path = "/image/{image_hash}/star",
    responses(
        (status = 200, description = "Star status toggled successfully", body = StarResponse),
        (status = 401, description = "Unauthorized"),
        (status = 404, description = "Image not found"),
        (status = 500, description = "Internal server error")
    )
)]
#[actix_web::post("/image/{image_hash}/star")]
pub async fn toggle_image_star(
    req: HttpRequest,
    path: web::Path<String>,
    pool: web::Data<MainDbPool>,
    config: web::Data<Config>
) -> Result<HttpResponse, actix_web::Error> {
    let claims = match utils::authenticate_request(&req, "toggle_image_star", config.get_api_key()).await {
        Ok(claims) => claims,
        Err(response) => return Ok(response),
    };
    let user_uuid = utils::parse_user_uuid(&claims.user_id)?;
    let hash = path.into_inner();
    toggle_media_star_inner(&pool.0, "images", "starred_images", &hash, &user_uuid).await
}

#[utoipa::path(
    post,
    path = "/video/{video_hash}/star",
    params(
        ("video_hash" = String, Path, description = "Video hash to toggle star status")
    ),
    responses(
        (status = 200, description = "Star status toggled successfully", body = StarResponse),
        (status = 404, description = "Video not found"),
        (status = 500, description = "Internal server error")
    )
)]
#[actix_web::post("/video/{video_hash}/star")]
pub async fn toggle_video_star(
    req: HttpRequest,
    path: web::Path<String>,
    pool: web::Data<MainDbPool>,
    config: web::Data<Config>
) -> Result<HttpResponse, actix_web::Error> {
    let claims = match utils::authenticate_request(&req, "toggle_video_star", config.get_api_key()).await {
        Ok(claims) => claims,
        Err(response) => return Ok(response),
    };
    let user_uuid = utils::parse_user_uuid(&claims.user_id)?;
    let hash = path.into_inner();
    toggle_media_star_inner(&pool.0, "videos", "starred_videos", &hash, &user_uuid).await
}

#[derive(Serialize, ToSchema)]
#[schema(example = json!({
    "device_ids": ["device-123", "device-456"]
}))]
pub struct DeviceIdsResponse {
    pub device_ids: Vec<String>,
}

#[utoipa::path(
    get,
    path = "/api/device_ids",
    responses(
        (status = 200, description = "List of device IDs", body = DeviceIdsResponse),
        (status = 401, description = "Unauthorized"),
        (status = 500, description = "Internal server error")
    )
)]
#[get("/device_ids")]
pub async fn get_device_ids(
    req: HttpRequest,
    pool: web::Data<MainDbPool>,
    config: web::Data<Config>
) -> Result<HttpResponse, actix_web::Error> {
    let claims = match utils::authenticate_request(&req, "get_device_ids", config.get_api_key()).await {
        Ok(claims) => claims,
        Err(response) => return Ok(response),
    };

    let client = utils::get_db_client(&pool.0).await?;
    let user_uuid = utils::parse_user_uuid(&claims.user_id)?;
    // Build queries: always filter by user_id
    let mut device_set = HashSet::new();

    for table in &["images", "videos"] {
        crate::utils::validate_table_name(table).map_err(actix_web::error::ErrorBadRequest)?;
        let query = format!("SELECT DISTINCT deviceid FROM {} WHERE user_id = $1 AND deviceid IS NOT NULL AND deleted_at IS NULL", table);
        let params = vec![&user_uuid as &(dyn tokio_postgres::types::ToSql + Sync)];

        let rows = client.query(&query, &params).await.map_err(|e| {
            error!("Failed to query {} device IDs: {}", table, e);
            actix_web::error::ErrorInternalServerError("Failed to retrieve device IDs")
        })?;

        for row in rows {
            let device_id: String = row.get(0);
            device_set.insert(device_id);
        }
    }

    let mut sorted_ids: Vec<String> = device_set.into_iter().collect();
    sorted_ids.sort();

    info!("Returning {} device IDs for user role: {}", sorted_ids.len(), claims.role);
    Ok(HttpResponse::Ok().json(DeviceIdsResponse { device_ids: sorted_ids }))
}

#[derive(Serialize, ToSchema)]
#[schema(example = json!({
    "hash": "somehash",
    "name": "IMG_20231222_101010.jpg",
    "created_at": "2025-01-01T12:00:00Z",
    "place": "Paris, France"
}))]
pub struct RandomImageResponse {
    pub hash: String,
    pub name: String,
    pub created_at: String,
    pub place: Option<String>,
}

#[derive(Deserialize, ToSchema, IntoParams)]
pub struct RandomImageQuery {
    #[serde(default)]
    pub starred_only: bool,
    /// Comma-separated label IDs to filter by (OR semantics)
    #[serde(default)]
    pub label_ids: Option<String>,
}

#[utoipa::path(
    get,
    path = "/image/random",
    params(RandomImageQuery),
    responses(
        (status = 200, description = "Random image found", body = RandomImageResponse),
        (status = 401, description = "Unauthorized"),
        (status = 404, description = "No images found"),
        (status = 500, description = "Internal server error")
    )
)]
#[get("/image/random")]
pub async fn get_random_image(
    req: HttpRequest,
    query: web::Query<RandomImageQuery>,
    pool: web::Data<MainDbPool>,
    config: web::Data<Config>
) -> Result<HttpResponse, actix_web::Error> {
    let claims = match utils::authenticate_request(&req, "get_random_image", config.get_api_key()).await {
        Ok(claims) => claims,
        Err(response) => return Ok(response),
    };

    let user_uuid = utils::parse_user_uuid(&claims.user_id)?;
    let client = utils::get_db_client(&pool.0).await?;

    let label_ids_vec: Vec<i32> = query.label_ids.as_deref()
        .unwrap_or("")
        .split(',')
        .filter_map(|s| s.trim().parse::<i32>().ok())
        .collect();

    let mut sql = "SELECT i.hash, i.name, i.created_at, i.place FROM images i".to_string();
    let mut conditions = vec!["i.deleted_at IS NULL".to_string()];
    let mut params: Vec<&(dyn tokio_postgres::types::ToSql + Sync)> = Vec::new();

    if query.starred_only {
        sql.push_str(" INNER JOIN starred_images s ON i.hash = s.hash AND s.user_id = $1");
        params.push(&user_uuid as &(dyn tokio_postgres::types::ToSql + Sync));
    }

    if !label_ids_vec.is_empty() {
        sql.push_str(&format!(
            " INNER JOIN image_labels il ON i.hash = il.image_hash AND il.label_id = ANY(${})",
            params.len() + 1
        ));
        params.push(&label_ids_vec as &(dyn tokio_postgres::types::ToSql + Sync));
    }

    conditions.push(format!("i.user_id = ${}", params.len() + 1));
    params.push(&user_uuid as &(dyn tokio_postgres::types::ToSql + Sync));

    if !conditions.is_empty() {
        sql.push_str(" WHERE ");
        sql.push_str(&conditions.join(" AND "));
    }

    sql.push_str(" ORDER BY RANDOM() LIMIT 1");

    let row = client.query_opt(&sql, &params).await.map_err(|e| {
        error!("Failed to fetch random image: {}", e);
        actix_web::error::ErrorInternalServerError("Database error")
    })?;

    match row {
        Some(row) => {
            let hash: String = row.get(0);
            let name: String = row.get(1);
            let created_at: chrono::DateTime<chrono::Utc> = row.get(2);
            let place: Option<String> = row.get(3);

            Ok(HttpResponse::Ok().json(RandomImageResponse {
                hash,
                name,
                created_at: created_at.to_rfc3339(),
                place,
            }))
        }
        None => Ok(HttpResponse::NotFound().json(serde_json::json!({"error": "No images found"}))),
    }
}

#[derive(Serialize, ToSchema)]
pub struct TrashItem {
    pub hash: String,
    pub name: String,
    pub created_at: String,
    pub ext: String,
    #[serde(rename = "type")]
    pub media_kind: String,
    pub deviceid: Option<String>,
    pub deleted_at: String,
    pub media_type: String,
}

#[utoipa::path(
    get,
    path = "/trash",
    responses(
        (status = 200, description = "List of deleted media"),
        (status = 401, description = "Unauthorized"),
        (status = 500, description = "Internal server error")
    )
)]
#[get("/trash")]
pub async fn get_trash(
    req: HttpRequest,
    pool: web::Data<MainDbPool>,
    config: web::Data<Config>,
) -> Result<HttpResponse, actix_web::Error> {
    let claims = match utils::authenticate_request(&req, "get_trash", config.get_api_key()).await {
        Ok(claims) => claims,
        Err(response) => return Ok(response),
    };

    let user_uuid = utils::parse_user_uuid(&claims.user_id)?;
    let client = utils::get_db_client(&pool.0).await?;

    let rows = client
        .query(
            "SELECT hash, name, created_at, ext, COALESCE(type, ''), deviceid, deleted_at, 'image' as media_type \
             FROM images WHERE user_id = $1 AND deleted_at IS NOT NULL \
             UNION ALL \
             SELECT hash, name, created_at, ext, COALESCE(type, ''), deviceid, deleted_at, 'video' as media_type \
             FROM videos WHERE user_id = $1 AND deleted_at IS NOT NULL \
             ORDER BY deleted_at DESC \
             LIMIT 200",
            &[&user_uuid]
        ).await
        .map_err(|e| {
            error!("Failed to query trash: {}", e);
            actix_web::error::ErrorInternalServerError("Database error")
        })?;

    let items: Vec<TrashItem> = rows.iter().map(|row| TrashItem {
        hash: row.get(0),
        name: row.get(1),
        created_at: row.get::<_, chrono::DateTime<chrono::Utc>>(2).to_rfc3339(),
        ext: row.get(3),
        media_kind: row.get(4),
        deviceid: row.get(5),
        deleted_at: row.get::<_, chrono::DateTime<chrono::Utc>>(6).to_rfc3339(),
        media_type: row.get(7),
    }).collect();

    info!("Returning {} trash items", items.len());
    Ok(HttpResponse::Ok().json(items))
}

/// Shared implementation for soft-restoring images or videos.
async fn soft_restore_media(
    pool: &deadpool_postgres::Pool,
    table: &str,
    hash: &str,
    user_id: &uuid::Uuid,
) -> Result<HttpResponse, actix_web::Error> {
    crate::utils::validate_table_name(table).map_err(actix_web::error::ErrorBadRequest)?;
    let client = utils::get_db_client(pool).await?;

    let query = if table == "images" {
        "UPDATE images SET deleted_at = NULL, duplicates_checked_at = NULL WHERE hash = $1 AND user_id = $2 AND deleted_at IS NOT NULL"
    } else {
        "UPDATE videos SET deleted_at = NULL WHERE hash = $1 AND user_id = $2 AND deleted_at IS NOT NULL"
    };

    let result = client
        .execute(query, &[&hash, user_id])
        .await
        .map_err(|e| {
            error!("Failed to restore {}: {}", table, e);
            actix_web::error::ErrorInternalServerError("Database error")
        })?;

    if result == 0 {
        let media_type = table.trim_end_matches('s');
        return Ok(HttpResponse::NotFound().json(serde_json::json!({
            "status": "error",
            "message": format!("{} not found or not deleted.", media_type.chars().next().unwrap().to_uppercase().to_string() + &media_type[1..])
        })));
    }

    info!("{} restored: {}", table, hash);
    Ok(HttpResponse::Ok().json(serde_json::json!({"status": "success", "hash": hash})))
}

#[utoipa::path(
    post,
    path = "/image/{image_hash}/restore",
    responses(
        (status = 200, description = "Image restored"),
        (status = 401, description = "Unauthorized"),
        (status = 404, description = "Image not found or not deleted"),
        (status = 500, description = "Internal server error")
    )
)]
#[post("/image/{image_hash}/restore")]
pub async fn restore_image(
    req: HttpRequest,
    path: web::Path<String>,
    pool: web::Data<MainDbPool>,
    config: web::Data<Config>,
) -> Result<HttpResponse, actix_web::Error> {
    let claims = match utils::authenticate_request(&req, "restore_image", config.get_api_key()).await {
        Ok(claims) => claims,
        Err(response) => return Ok(response),
    };

    let user_uuid = utils::parse_user_uuid(&claims.user_id)?;
    let hash = path.into_inner();
    soft_restore_media(&pool.0, "images", &hash, &user_uuid).await
}

#[utoipa::path(
    post,
    path = "/video/{video_hash}/restore",
    responses(
        (status = 200, description = "Video restored"),
        (status = 401, description = "Unauthorized"),
        (status = 404, description = "Video not found or not deleted"),
        (status = 500, description = "Internal server error")
    )
)]
#[post("/video/{video_hash}/restore")]
pub async fn restore_video(
    req: HttpRequest,
    path: web::Path<String>,
    pool: web::Data<MainDbPool>,
    config: web::Data<Config>,
) -> Result<HttpResponse, actix_web::Error> {
    let claims = match utils::authenticate_request(&req, "restore_video", config.get_api_key()).await {
        Ok(claims) => claims,
        Err(response) => return Ok(response),
    };

    let user_uuid = utils::parse_user_uuid(&claims.user_id)?;
    let hash = path.into_inner();
    soft_restore_media(&pool.0, "videos", &hash, &user_uuid).await
}

/// Shared implementation for soft-deleting images or videos.
async fn soft_delete_media(
    pool: &deadpool_postgres::Pool,
    table: &str,
    hash: &str,
    user_id: &uuid::Uuid,
) -> Result<HttpResponse, actix_web::Error> {
    crate::utils::validate_table_name(table).map_err(actix_web::error::ErrorBadRequest)?;
    let client = utils::get_db_client(pool).await?;

    let result = client
        .execute(
            &format!("UPDATE {} SET deleted_at = NOW() WHERE hash = $1 AND user_id = $2 AND deleted_at IS NULL", table),
            &[&hash, user_id]
        ).await
        .map_err(|e| {
            error!("Failed to soft delete {}: {}", table, e);
            actix_web::error::ErrorInternalServerError("Database error")
        })?;

    if result == 0 {
        let media_type = table.trim_end_matches('s');
        return Ok(HttpResponse::NotFound().json(serde_json::json!({
            "status": "error",
            "message": format!("{} not found or already deleted.", media_type.chars().next().unwrap().to_uppercase().to_string() + &media_type[1..])
        })));
    }

    if table == "images" {
        let _ = client
            .execute(
                "DELETE FROM image_duplicate_pairs WHERE user_id = $1 AND (hash_a = $2 OR hash_b = $2)",
                &[user_id, &hash],
            )
            .await;
    }

    info!("{} marked as deleted: {}", table, hash);
    Ok(HttpResponse::Ok().json(serde_json::json!({"status": "success", "hash": hash})))
}

#[utoipa::path(
    post,
    path = "/image/{image_hash}/delete",
    responses(
        (status = 200, description = "Image marked as deleted"),
        (status = 401, description = "Unauthorized"),
        (status = 404, description = "Image not found"),
        (status = 500, description = "Internal server error")
    )
)]
#[post("/image/{image_hash}/delete")]
pub async fn delete_image(
    req: HttpRequest,
    path: web::Path<String>,
    pool: web::Data<MainDbPool>,
    config: web::Data<Config>
) -> Result<HttpResponse, actix_web::Error> {
    let claims = match utils::authenticate_request(&req, "delete_image", config.get_api_key()).await {
        Ok(claims) => claims,
        Err(response) => return Ok(response),
    };

    let user_uuid = utils::parse_user_uuid(&claims.user_id)?;
    let hash = path.into_inner();
    soft_delete_media(&pool.0, "images", &hash, &user_uuid).await
}

#[utoipa::path(
    post,
    path = "/video/{video_hash}/delete",
    responses(
        (status = 200, description = "Video marked as deleted"),
        (status = 401, description = "Unauthorized"),
        (status = 404, description = "Video not found"),
        (status = 500, description = "Internal server error")
    )
)]
#[post("/video/{video_hash}/delete")]
pub async fn delete_video(
    req: HttpRequest,
    path: web::Path<String>,
    pool: web::Data<MainDbPool>,
    config: web::Data<Config>
) -> Result<HttpResponse, actix_web::Error> {
    let claims = match utils::authenticate_request(&req, "delete_video", config.get_api_key()).await {
        Ok(claims) => claims,
        Err(response) => return Ok(response),
    };

    let user_uuid = utils::parse_user_uuid(&claims.user_id)?;
    let hash = path.into_inner();
    soft_delete_media(&pool.0, "videos", &hash, &user_uuid).await
}

// ── Image enhancement ─────────────────────────────────────────────────────────

#[derive(Deserialize, IntoParams)]
pub struct EnhanceQuery {
    /// Enhancement mode: auto (default), exposure, restore, all
    mode: Option<String>,
}

#[utoipa::path(
    post,
    path = "/image/{hash}/enhance",
    params(EnhanceQuery),
    responses(
        (status = 200, description = "Enhanced JPEG image", content_type = "image/jpeg"),
        (status = 401, description = "Unauthorized"),
        (status = 404, description = "Image not found"),
        (status = 503, description = "AI service unavailable"),
    )
)]
#[post("/image/{hash}/enhance")]
pub async fn enhance_image(
    req: HttpRequest,
    path: web::Path<String>,
    query: web::Query<EnhanceQuery>,
    pool: web::Data<MainDbPool>,
    config: web::Data<Config>,
) -> Result<HttpResponse, actix_web::Error> {
    let claims = match utils::authenticate_request(&req, "enhance_image", config.get_api_key()).await {
        Ok(c) => c,
        Err(r) => return Ok(r),
    };

    let hash = path.into_inner();
    let client = utils::get_db_client(&pool.0).await?;
    let user_uuid = utils::parse_user_uuid(&claims.user_id)?;

    let row = client.query_opt(
        "SELECT ext FROM images WHERE user_id = $1 AND hash = $2 AND deleted_at IS NULL LIMIT 1",
        &[&user_uuid, &hash],
    ).await.map_err(|e| {
        error!("DB error in enhance_image: {}", e);
        actix_web::error::ErrorInternalServerError("Database error")
    })?;

    let ext: String = match row {
        Some(r) => r.get(0),
        None => return Ok(HttpResponse::NotFound().json(
            serde_json::json!({"error": "Image not found"})
        )),
    };

    let image_path = match crate::media_utils::safe_resolve_content_path(
        config.get_images_dir(),
        &hash,
        &ext,
    ) {
        Ok(p) => p,
        Err(e) => {
            error!("Unsafe or missing image path for enhance hash '{}': {}", hash, e);
            return Ok(HttpResponse::NotFound().json(
                serde_json::json!({"error": "Image not found"})
            ));
        }
    };

    let image_data = tokio::fs::read(&image_path).await.map_err(|e| {
        error!("Failed to read image {:?}: {}", image_path, e);
        actix_web::error::ErrorInternalServerError("Failed to read image file")
    })?;

    let mode = query.mode.clone().unwrap_or_else(|| "auto".to_string());

    let ai_client = crate::ai_client::AiClient::shared(&config);
    let enhance_resp = ai_client.enhance_image(image_data.to_vec(), mode).await.map_err(|e| {
        error!("AI service enhance failed: {}", e);
        actix_web::error::ErrorServiceUnavailable("AI service unavailable")
    })?;

    info!("Enhanced image {}: ops={:?}", hash, enhance_resp.operations);
    Ok(HttpResponse::Ok()
        .content_type("image/jpeg")
        .insert_header(("X-Enhance-Operations", enhance_resp.operations.join(",")))
        .body(enhance_resp.image_data))
}

// ── Save enhanced image to library ───────────────────────────────────────────

#[derive(Deserialize, ToSchema)]
pub struct SaveEnhancedRequest {
    /// Base64-encoded JPEG of the enhanced image (from the /enhance endpoint)
    pub image: String,
}

#[utoipa::path(
    post,
    path = "/image/{hash}/save-enhanced",
    responses(
        (status = 201, description = "Enhanced image saved to library"),
        (status = 400, description = "Invalid image data"),
        (status = 401, description = "Unauthorized"),
        (status = 404, description = "Original image not found"),
    )
)]
#[post("/image/{hash}/save-enhanced")]
pub async fn save_enhanced_image(
    req: HttpRequest,
    path: web::Path<String>,
    body: web::Json<SaveEnhancedRequest>,
    pool: web::Data<MainDbPool>,
    geotagging_pool: web::Data<GeotaggingDbPool>,
    config: web::Data<Config>,
) -> Result<HttpResponse, actix_web::Error> {
    let claims = match utils::authenticate_request(&req, "save_enhanced_image", config.get_api_key()).await {
        Ok(c) => c,
        Err(r) => return Ok(r),
    };

    let original_hash = path.into_inner();
    let client = utils::get_db_client(&pool.0).await?;
    let user_uuid = utils::parse_user_uuid(&claims.user_id)?;

    // Fetch the original image name and date so we can preserve them
    let row = client.query_opt(
        "SELECT name, created_at FROM images WHERE user_id = $1 AND hash = $2 AND deleted_at IS NULL LIMIT 1",
        &[&user_uuid, &original_hash],
    ).await.map_err(|e| {
        error!("DB error in save_enhanced_image: {}", e);
        actix_web::error::ErrorInternalServerError("Database error")
    })?;

    let (original_name, original_created_at): (String, chrono::DateTime<chrono::Utc>) = match row {
        Some(r) => (r.get(0), r.get(1)),
        None => return Ok(HttpResponse::NotFound().json(
            serde_json::json!({"error": "Original image not found"})
        )),
    };

    // Decode the base64 JPEG sent by the browser
    let image_bytes = general_purpose::STANDARD.decode(&body.image).map_err(|_| {
        actix_web::error::ErrorBadRequest("Invalid base64 image data")
    })?;

    // Hash the bytes (blake3) — this becomes the new image's identifier
    let new_hash = blake3::hash(&image_bytes).to_hex().to_string();

    // Derive name: strip original extension, append _enhanced.jpg
    let base_stem = Path::new(&original_name)
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("enhanced");
    let enhanced_name = format!("{}_enhanced.jpg", base_stem);

    // Write bytes to a temp file for ingest
    let temp_dir = Path::new(config.get_images_dir()).join(".tmp");
    tokio::fs::create_dir_all(&temp_dir).await.map_err(|_| {
        actix_web::error::ErrorInternalServerError("Failed to create temp dir")
    })?;
    let temp_path = temp_dir.join(format!("{}.tmp", uuid::Uuid::new_v4()));
    tokio::fs::write(&temp_path, &image_bytes).await.map_err(|e| {
        error!("Failed to write temp file: {}", e);
        actix_web::error::ErrorInternalServerError("Failed to write temp file")
    })?;

    // Run through the normal ingest pipeline (moves file, inserts DB row, extracts EXIF, geo)
    match ingest::process_image_file(
        &temp_path,
        &enhanced_name,
        &new_hash,
        "web-enhanced",
        &user_uuid,
        &pool,
        &geotagging_pool,
        &config,
        true,                        // move (not copy) the temp file
        Some(original_created_at),   // preserve the original photo's date
    ).await {
        Ok(result) => {
            info!("Saved enhanced image: original={} new={}", original_hash, result.hash);
            Ok(HttpResponse::Created().json(serde_json::json!({
                "status": "success",
                "hash": result.hash,
                "name": result.name,
            })))
        }
        Err(e) => {
            error!("Failed to ingest enhanced image: {}", e);
            Ok(HttpResponse::InternalServerError().json(
                serde_json::json!({"error": format!("Failed to save: {}", e)})
            ))
        }
    }
}

#[cfg(test)]
mod orientation_label_tests {
    use super::orientation_label;

    #[test]
    fn labels_basic_orientations() {
        assert_eq!(orientation_label(Some(4000), Some(3000), Some(1)), Some("Landscape".to_string()));
        assert_eq!(orientation_label(Some(3000), Some(4000), Some(1)), Some("Portrait".to_string()));
        assert_eq!(orientation_label(Some(2000), Some(2000), None), Some("Square".to_string()));
    }

    #[test]
    fn swapped_orientation_flips_effective_dimensions() {
        // 4000x3000 sensor rotated 90° (orientation 6) displays as portrait.
        assert_eq!(orientation_label(Some(4000), Some(3000), Some(6)), Some("Portrait".to_string()));
        assert_eq!(orientation_label(Some(3000), Some(4000), Some(8)), Some("Landscape".to_string()));
    }

    #[test]
    fn near_square_within_tolerance_is_square() {
        assert_eq!(orientation_label(Some(1000), Some(970), Some(1)), Some("Square".to_string()));
    }

    #[test]
    fn unknown_or_invalid_inputs_return_none() {
        assert_eq!(orientation_label(None, Some(100), Some(1)), None);
        assert_eq!(orientation_label(Some(0), Some(0), Some(1)), None);
        assert_eq!(orientation_label(Some(-5), Some(10), Some(1)), None);
    }
}

#[cfg(test)]
mod resolution_label_tests {
    use super::{orientation_label, resolution_label};

    #[test]
    fn resolution_reflects_displayed_dimensions() {
        // 384x256 sensor rotated 90° CW (6) displays as 256x384 portrait.
        assert_eq!(resolution_label(Some(384), Some(256), Some(6)), Some("256 × 384".to_string()));
        assert_eq!(orientation_label(Some(384), Some(256), Some(6)), Some("Portrait".to_string()));
        // Unrotated stays as stored.
        assert_eq!(resolution_label(Some(4000), Some(3000), Some(1)), Some("4000 × 3000".to_string()));
    }

    #[test]
    fn resolution_none_without_dimensions() {
        assert_eq!(resolution_label(None, None, None), None);
    }
}
