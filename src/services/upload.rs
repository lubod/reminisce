use actix_multipart::Multipart;
use actix_web::{web, HttpResponse, Error, post};
use futures::{TryStreamExt};
use serde::{Deserialize, Serialize};
use tracing::{error};
use uuid::Uuid;
use chrono::Utc;

use crate::config::Config;
use crate::db::{MainDbPool, GeotaggingDbPool};
use crate::services::ingest;
use crate::utils;
use crate::Claims;

#[derive(Deserialize, utoipa::ToSchema)]
pub struct UploadImageRequest {
    pub hash: String,
    pub name: String,
}

#[derive(Deserialize, utoipa::ToSchema)]
pub struct UploadVideoRequest {
    pub hash: String,
    pub name: String,
}

#[derive(Deserialize, utoipa::ToSchema)]
pub struct UploadImageMetadataRequest {
    pub deviceid: String,
    pub hash: String,
    pub type_name: Option<String>,
    pub created_at: Option<String>,
    pub name: String,
    pub ext: String,
    pub exif: Option<String>,
    pub has_thumbnail: Option<bool>,
    pub last_verified_at: Option<String>,
    pub verification_status: Option<i32>,
    pub latitude: Option<f64>,
    pub longitude: Option<f64>,
    pub place: Option<String>,
}

#[derive(Serialize, utoipa::ToSchema)]
pub struct UploadImageMetadataResponse {
    pub status: String,
    pub message: String,
}

#[derive(Deserialize, utoipa::ToSchema)]
pub struct UploadVideoMetadataRequest {
    pub deviceid: String,
    pub hash: String,
    pub type_name: Option<String>,
    pub created_at: Option<String>,
    pub name: String,
    pub ext: String,
    pub metadata: Option<String>,
    pub has_thumbnail: Option<bool>,
    pub last_verified_at: Option<String>,
    pub verification_status: Option<i32>,
}

#[derive(Serialize, utoipa::ToSchema)]
pub struct UploadVideoMetadataResponse {
    pub status: String,
    pub message: String,
}

#[derive(Deserialize, utoipa::ToSchema)]
pub struct CheckImagesExistRequest {
    pub device_id: String,
    pub hashes: Vec<String>,
}

#[derive(Serialize, utoipa::ToSchema)]
pub struct CheckImagesExistResponse {
    pub existing_hashes: Vec<String>,
}

#[derive(Deserialize, utoipa::ToSchema)]
pub struct CheckVideosExistRequest {
    pub device_id: String,
    pub hashes: Vec<String>,
}

#[derive(Serialize, utoipa::ToSchema)]
pub struct CheckVideosExistResponse {
    pub existing_hashes: Vec<String>,
}

#[utoipa::path(
    post,
    path = "/api/upload/image",
    request_body(content = UploadImageRequest, content_type = "multipart/form-data"),
    responses(
        (status = 201, description = "Image uploaded successfully", body = serde_json::Value),
        (status = 400, description = "Bad request"),
        (status = 401, description = "Unauthorized"),
        (status = 500, description = "Internal server error")
    ),
    security(
        ("jwt" = [])
    )
)]
#[post("/upload/image")]
pub async fn upload_image(
    mut payload: Multipart,
    pool: web::Data<MainDbPool>,
    geotagging_pool: web::Data<GeotaggingDbPool>,
    config: web::Data<Config>,
    claims: Claims,
) -> Result<HttpResponse, Error> {
    let mut image_hash_opt = None;
    let mut image_name_opt = None;
    let mut image_temp_file_path = None;
    let mut calculated_image_hash_opt = None;
    let mut device_id_opt = None;
    let mut client_created_at_opt: Option<chrono::DateTime<Utc>> = None;
    let user_uuid = Uuid::parse_str(&claims.user_id).map_err(|_| actix_web::error::ErrorUnauthorized("Invalid user ID"))?;

    while let Ok(Some(mut field)) = payload.try_next().await {
        let content_disposition = field.content_disposition();
        let name = content_disposition.get_name().unwrap_or("");

        match name {
            "hash" => image_hash_opt = Some(crate::media_utils::read_field_string(&mut field).await),
            "name" => image_name_opt = Some(crate::media_utils::read_field_string(&mut field).await),
            "device_id" => device_id_opt = Some(crate::media_utils::read_field_string(&mut field).await),
            "created_at" => {
                let s = crate::media_utils::read_field_string(&mut field).await;
                client_created_at_opt = chrono::DateTime::parse_from_rfc3339(s.trim())
                    .ok()
                    .map(|dt| dt.with_timezone(&Utc));
            }
            "image" => {
                let temp_dir = std::path::Path::new(config.get_images_dir()).join(".tmp");
                let (temp_path, hash) = crate::media_utils::streaming_hash_to_temp(&mut field, &temp_dir).await?;
                calculated_image_hash_opt = Some(hash);
                image_temp_file_path = Some(temp_path);
            }
            _ => (),
        }
    }

    let image_hash = image_hash_opt.ok_or_else(|| actix_web::error::ErrorBadRequest("Missing hash"))?;
    let image_name = image_name_opt.ok_or_else(|| actix_web::error::ErrorBadRequest("Missing name"))?;
    let image_temp_path = image_temp_file_path.ok_or_else(|| actix_web::error::ErrorBadRequest("Missing image"))?;
    let calculated_hash = calculated_image_hash_opt.ok_or_else(|| actix_web::error::ErrorInternalServerError("Hash calculation failed"))?;
    let device_id = device_id_opt.unwrap_or_else(|| "web-client".to_string());

    if calculated_hash != image_hash {
        utils::cleanup_temp_files_spawn(Some(image_temp_path), None);
        return Ok(HttpResponse::BadRequest().json(serde_json::json!({"status": "error", "message": "Hash verification failed"})));
    }

    let res = ingest::process_image_file(
        &image_temp_path,
        &image_name,
        &image_hash,
        &device_id,
        &user_uuid,
        &pool,
        &geotagging_pool,
        &config,
        true,
        client_created_at_opt,
    ).await;

    match res {
        Ok(ingest_res) => Ok(HttpResponse::Created().json(serde_json::json!({
            "status": "success",
            "hash": ingest_res.hash,
            "filename": ingest_res.filename,
            "name": ingest_res.name,
            "path": ingest_res.path,
            "thumbnail": if ingest_res.thumbnail { "yes" } else { "no" }
        }))),
        Err(e) => {
            error!("Failed to ingest image: {}", e);
            Ok(HttpResponse::InternalServerError().json(serde_json::json!({"status": "error", "message": e.to_string()})))
        }
    }
}

#[utoipa::path(
    post,
    path = "/api/upload/batch/image",
    request_body(content = Vec<UploadImageRequest>, content_type = "multipart/form-data"),
    responses(
        (status = 201, description = "Batch image upload successful", body = serde_json::Value),
        (status = 400, description = "Bad request"),
        (status = 401, description = "Unauthorized"),
        (status = 500, description = "Internal server error")
    ),
    security(
        ("jwt" = [])
    )
)]
#[post("/upload/batch/image")]
pub async fn batch_upload_image(
    req: actix_web::HttpRequest,
    mut payload: Multipart,
    pool: web::Data<MainDbPool>,
    geotagging_pool: web::Data<GeotaggingDbPool>,
    config: web::Data<Config>,
    claims: Claims,
) -> Result<HttpResponse, Error> {
    let mut results = Vec::new();
    // Read device_id from query param, default to "web-client"
    let device_id = {
        let query = web::Query::<std::collections::HashMap<String, String>>::from_query(req.query_string())
            .unwrap_or_else(|_| web::Query(std::collections::HashMap::new()));
        query.get("device_id").cloned().unwrap_or_else(|| "web-client".to_string())
    };
    let user_uuid = Uuid::parse_str(&claims.user_id).map_err(|_| actix_web::error::ErrorUnauthorized("Invalid user ID"))?;

    while let Ok(Some(mut field)) = payload.try_next().await {
        let content_disposition = field.content_disposition();
        let filename = content_disposition.get_filename().unwrap_or("unknown").to_string();
        
        let temp_dir = std::path::Path::new(config.get_images_dir()).join(".tmp");
        let (temp_path, hash) = crate::media_utils::streaming_hash_to_temp(&mut field, &temp_dir).await?;

        let res = ingest::process_image_file(
            &temp_path,
            &filename,
            &hash,
            &device_id,
            &user_uuid,
            &pool,
            &geotagging_pool,
            &config,
            true,
            None, // batch upload has no per-file client date
        ).await;

        match res {
            Ok(ingest_res) => results.push(serde_json::json!({
                "status": "success",
                "hash": ingest_res.hash,
                "filename": ingest_res.filename,
                "name": ingest_res.name,
                "path": ingest_res.path,
                "thumbnail": if ingest_res.thumbnail { "yes" } else { "no" },
                "thumbnail_generated": ingest_res.thumbnail_generated
            })),
            Err(e) => {
                error!("Failed to ingest image in batch: {}", e);
                results.push(serde_json::json!({"status": "error", "hash": hash, "message": e.to_string()}));
            }
        }
    }

    Ok(HttpResponse::Created().json(results))
}

#[utoipa::path(
    post,
    path = "/api/upload/image/metadata",
    request_body = UploadImageMetadataRequest,
    responses(
        (status = 200, description = "Image metadata uploaded successfully", body = UploadImageMetadataResponse),
        (status = 400, description = "Bad request"),
        (status = 401, description = "Unauthorized"),
        (status = 500, description = "Internal server error")
    ),
    security(
        ("jwt" = [])
    )
)]
#[post("/upload/image/metadata")]
pub async fn upload_image_metadata(
    metadata: web::Json<UploadImageMetadataRequest>,
    pool: web::Data<MainDbPool>,
    claims: Claims,
) -> HttpResponse {
    let user_uuid = match Uuid::parse_str(&claims.user_id) {
        Ok(u) => u,
        Err(_) => return HttpResponse::Unauthorized().json(UploadImageMetadataResponse {
            status: "error".to_string(),
            message: "Invalid user ID".to_string(),
        }),
    };

    let client = match pool.0.get().await {
        Ok(c) => c,
        Err(_) => return HttpResponse::InternalServerError().json(UploadImageMetadataResponse {
            status: "error".to_string(),
            message: "Database connection failed".to_string(),
        }),
    };

    let created_at = metadata.created_at.as_ref()
        .and_then(|s| chrono::DateTime::parse_from_rfc3339(s).ok())
        .map(|dt| dt.with_timezone(&Utc))
        .or_else(|| utils::parse_date_from_image_name(&metadata.name))
        .unwrap_or_else(Utc::now);

    let last_verified_at = metadata.last_verified_at.as_ref()
        .and_then(|s| chrono::DateTime::parse_from_rfc3339(s).ok())
        .map(|dt| dt.with_timezone(&Utc));

    let query = "INSERT INTO images (user_id, hash, deviceid, type, created_at, name, ext, exif, has_thumbnail, last_verified_at, verification_status, location, place, added_at)
                 VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, ST_SetSRID(ST_MakePoint($12, $13), 4326), $14, NOW())
                 ON CONFLICT (user_id, hash) DO UPDATE SET
                 type = EXCLUDED.type, name = EXCLUDED.name, ext = EXCLUDED.ext,
                 exif = COALESCE(EXCLUDED.exif, images.exif),
                 has_thumbnail = EXCLUDED.has_thumbnail, last_verified_at = EXCLUDED.last_verified_at,
                 verification_status = EXCLUDED.verification_status, location = EXCLUDED.location, place = EXCLUDED.place,
                 added_at = NOW()";

    let lat = metadata.latitude.unwrap_or(0.0);
    let lon = metadata.longitude.unwrap_or(0.0);

    if let Err(e) = client.execute(query, &[
        &user_uuid, &metadata.hash, &metadata.deviceid, &metadata.type_name, &created_at, &metadata.name, &metadata.ext,
        &metadata.exif, &metadata.has_thumbnail.unwrap_or(false), &last_verified_at,
        &metadata.verification_status.unwrap_or(0), &lon, &lat, &metadata.place
    ]).await {
        error!("Failed to update image metadata: {}", e);
        return HttpResponse::InternalServerError().json(UploadImageMetadataResponse {
            status: "error".to_string(),
            message: format!("Database error: {}", e),
        });
    }

    let _ = client.execute(
        "INSERT INTO media_sources (user_id, hash, media_type, device_id, uploaded_at)
         VALUES ($1, $2, 'image', $3, NOW())
         ON CONFLICT DO NOTHING",
        &[&user_uuid, &metadata.hash, &metadata.deviceid],
    ).await;

    HttpResponse::Ok().json(UploadImageMetadataResponse {
        status: "success".to_string(),
        message: "Metadata updated".to_string(),
    })
}

#[utoipa::path(
    post,
    path = "/api/upload/video/metadata",
    request_body = UploadVideoMetadataRequest,
    responses(
        (status = 200, description = "Video metadata uploaded successfully", body = UploadVideoMetadataResponse),
        (status = 400, description = "Bad request"),
        (status = 401, description = "Unauthorized"),
        (status = 500, description = "Internal server error")
    ),
    security(
        ("jwt" = [])
    )
)]
#[post("/upload/video/metadata")]
pub async fn upload_video_metadata(
    metadata: web::Json<UploadVideoMetadataRequest>,
    pool: web::Data<MainDbPool>,
    claims: Claims,
) -> HttpResponse {
    let user_uuid = match Uuid::parse_str(&claims.user_id) {
        Ok(u) => u,
        Err(_) => return HttpResponse::Unauthorized().json(UploadVideoMetadataResponse {
            status: "error".to_string(),
            message: "Invalid user ID".to_string(),
        }),
    };

    let client = match pool.0.get().await {
        Ok(c) => c,
        Err(_) => return HttpResponse::InternalServerError().json(UploadVideoMetadataResponse {
            status: "error".to_string(),
            message: "Database connection failed".to_string(),
        }),
    };

    let created_at = metadata.created_at.as_ref()
        .and_then(|s| chrono::DateTime::parse_from_rfc3339(s).ok())
        .map(|dt| dt.with_timezone(&Utc))
        .or_else(|| utils::parse_date_from_video_name(&metadata.name))
        .unwrap_or_else(Utc::now);

    let last_verified_at = metadata.last_verified_at.as_ref()
        .and_then(|s| chrono::DateTime::parse_from_rfc3339(s).ok())
        .map(|dt| dt.with_timezone(&Utc));

    let query = "INSERT INTO videos (user_id, hash, deviceid, type, created_at, name, ext, metadata, has_thumbnail, last_verified_at, verification_status, added_at)
                 VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, NOW())
                 ON CONFLICT (user_id, hash) DO UPDATE SET
                 type = EXCLUDED.type, name = EXCLUDED.name, ext = EXCLUDED.ext, metadata = EXCLUDED.metadata,
                 has_thumbnail = EXCLUDED.has_thumbnail, last_verified_at = EXCLUDED.last_verified_at,
                 verification_status = EXCLUDED.verification_status,
                 added_at = NOW()";

    if let Err(e) = client.execute(query, &[
        &user_uuid, &metadata.hash, &metadata.deviceid, &metadata.type_name, &created_at, &metadata.name, &metadata.ext,
        &metadata.metadata, &metadata.has_thumbnail.unwrap_or(false), &last_verified_at,
        &metadata.verification_status.unwrap_or(0)
    ]).await {
        error!("Failed to update video metadata: {}", e);
        return HttpResponse::InternalServerError().json(UploadVideoMetadataResponse {
            status: "error".to_string(),
            message: format!("Database error: {}", e),
        });
    }

    let _ = client.execute(
        "INSERT INTO media_sources (user_id, hash, media_type, device_id, uploaded_at)
         VALUES ($1, $2, 'video', $3, NOW())
         ON CONFLICT DO NOTHING",
        &[&user_uuid, &metadata.hash, &metadata.deviceid],
    ).await;

    HttpResponse::Ok().json(UploadVideoMetadataResponse {
        status: "success".to_string(),
        message: "Metadata updated".to_string(),
    })
}

async fn internal_check_images_exist(
    user_id: &Uuid,
    req: CheckImagesExistRequest,
    pool: web::Data<MainDbPool>,
) -> HttpResponse {
    let client = match pool.0.get().await {
        Ok(c) => c,
        Err(_) => return HttpResponse::InternalServerError().finish(),
    };

    if req.hashes.is_empty() {
        return HttpResponse::Ok().json(CheckImagesExistResponse { existing_hashes: vec![] });
    }

    match client.query(
        "SELECT hash FROM images WHERE user_id = $1 AND hash = ANY($2) AND deleted_at IS NULL",
        &[user_id, &req.hashes]
    ).await {
        Ok(rows) => {
            let existing_hashes: Vec<String> = rows.iter().map(|r| r.get(0)).collect();
            HttpResponse::Ok().json(CheckImagesExistResponse { existing_hashes })
        }
        Err(e) => {
            error!("Failed to check image existence batch: {}", e);
            HttpResponse::InternalServerError().finish()
        }
    }
}

async fn internal_check_videos_exist(
    user_id: &Uuid,
    req: CheckVideosExistRequest,
    pool: web::Data<MainDbPool>,
) -> HttpResponse {
    let client = match pool.0.get().await {
        Ok(c) => c,
        Err(_) => return HttpResponse::InternalServerError().finish(),
    };

    if req.hashes.is_empty() {
        return HttpResponse::Ok().json(CheckVideosExistResponse { existing_hashes: vec![] });
    }

    match client.query(
        "SELECT hash FROM videos WHERE user_id = $1 AND hash = ANY($2) AND deleted_at IS NULL",
        &[user_id, &req.hashes]
    ).await {
        Ok(rows) => {
            let existing_hashes: Vec<String> = rows.iter().map(|r| r.get(0)).collect();
            HttpResponse::Ok().json(CheckVideosExistResponse { existing_hashes })
        }
        Err(e) => {
            error!("Failed to check video existence batch: {}", e);
            HttpResponse::InternalServerError().finish()
        }
    }
}

#[utoipa::path(
    post,
    path = "/api/upload/check-images",
    request_body = CheckImagesExistRequest,
    responses(
        (status = 200, description = "Checked image hashes successfully", body = CheckImagesExistResponse),
        (status = 401, description = "Unauthorized"),
        (status = 500, description = "Internal server error")
    ),
    security(
        ("jwt" = [])
    )
)]
#[post("/upload/check-images")]
pub async fn check_images_exist_batch(
    req: web::Json<CheckImagesExistRequest>,
    pool: web::Data<MainDbPool>,
    claims: Claims,
) -> HttpResponse {
    let user_uuid = match Uuid::parse_str(&claims.user_id) {
        Ok(u) => u,
        Err(_) => return HttpResponse::Unauthorized().finish(),
    };
    internal_check_images_exist(&user_uuid, req.into_inner(), pool).await
}

#[utoipa::path(
    post,
    path = "/api/upload/check-videos",
    request_body = CheckVideosExistRequest,
    responses(
        (status = 200, description = "Checked video hashes successfully", body = CheckVideosExistResponse),
        (status = 401, description = "Unauthorized"),
        (status = 500, description = "Internal server error")
    ),
    security(
        ("jwt" = [])
    )
)]
#[post("/upload/check-videos")]
pub async fn check_videos_exist_batch(
    req: web::Json<CheckVideosExistRequest>,
    pool: web::Data<MainDbPool>,
    claims: Claims,
) -> HttpResponse {
    let user_uuid = match Uuid::parse_str(&claims.user_id) {
        Ok(u) => u,
        Err(_) => return HttpResponse::Unauthorized().finish(),
    };
    internal_check_videos_exist(&user_uuid, req.into_inner(), pool).await
}

#[utoipa::path(
    post,
    path = "/api/upload/batch-check-images",
    request_body = CheckImagesExistRequest,
    responses(
        (status = 200, description = "Checked image hashes successfully", body = CheckImagesExistResponse),
        (status = 401, description = "Unauthorized"),
        (status = 500, description = "Internal server error")
    ),
    security(
        ("jwt" = [])
    )
)]
#[post("/upload/batch-check-images")]
pub async fn batch_check_images(
    req: web::Json<CheckImagesExistRequest>,
    pool: web::Data<MainDbPool>,
    claims: Claims,
) -> HttpResponse {
    let user_uuid = match Uuid::parse_str(&claims.user_id) {
        Ok(u) => u,
        Err(_) => return HttpResponse::Unauthorized().finish(),
    };
    internal_check_images_exist(&user_uuid, req.into_inner(), pool).await
}

#[utoipa::path(
    post,
    path = "/api/upload/batch-check-videos",
    request_body = CheckVideosExistRequest,
    responses(
        (status = 200, description = "Checked video hashes successfully", body = CheckVideosExistResponse),
        (status = 401, description = "Unauthorized"),
        (status = 500, description = "Internal server error")
    ),
    security(
        ("jwt" = [])
    )
)]
#[post("/upload/batch-check-videos")]
pub async fn batch_check_videos(
    req: web::Json<CheckVideosExistRequest>,
    pool: web::Data<MainDbPool>,
    claims: Claims,
) -> HttpResponse {
    let user_uuid = match Uuid::parse_str(&claims.user_id) {
        Ok(u) => u,
        Err(_) => return HttpResponse::Unauthorized().finish(),
    };
    internal_check_videos_exist(&user_uuid, req.into_inner(), pool).await
}

#[utoipa::path(
    post,
    path = "/api/upload/video",
    request_body(content = UploadVideoRequest, content_type = "multipart/form-data"),
    responses(
        (status = 201, description = "Video uploaded successfully", body = serde_json::Value),
        (status = 400, description = "Bad request"),
        (status = 401, description = "Unauthorized"),
        (status = 500, description = "Internal server error")
    ),
    security(
        ("jwt" = [])
    )
)]
#[post("/upload/video")]
pub async fn upload_video(
    mut payload: Multipart,
    pool: web::Data<MainDbPool>,
    config: web::Data<Config>,
    claims: Claims,
) -> Result<HttpResponse, Error> {
    let mut video_hash_opt = None;
    let mut video_name_opt = None;
    let mut video_temp_file_path = None;
    let mut calculated_video_hash_opt = None;
    let mut device_id_opt = None;
    let mut client_created_at_opt: Option<chrono::DateTime<Utc>> = None;
    let user_uuid = Uuid::parse_str(&claims.user_id).map_err(|_| actix_web::error::ErrorUnauthorized("Invalid user ID"))?;

    while let Ok(Some(mut field)) = payload.try_next().await {
        let content_disposition = field.content_disposition();
        let name = content_disposition.get_name().unwrap_or("");

        match name {
            "hash" => video_hash_opt = Some(crate::media_utils::read_field_string(&mut field).await),
            "name" => video_name_opt = Some(crate::media_utils::read_field_string(&mut field).await),
            "device_id" => device_id_opt = Some(crate::media_utils::read_field_string(&mut field).await),
            "created_at" => {
                let s = crate::media_utils::read_field_string(&mut field).await;
                client_created_at_opt = chrono::DateTime::parse_from_rfc3339(s.trim())
                    .ok()
                    .map(|dt| dt.with_timezone(&Utc));
            }
            "video" => {
                let temp_dir = std::path::Path::new(config.get_videos_dir()).join(".tmp");
                let (temp_path, hash) = crate::media_utils::streaming_hash_to_temp(&mut field, &temp_dir).await?;
                calculated_video_hash_opt = Some(hash);
                video_temp_file_path = Some(temp_path);
            }
            _ => (),
        }
    }

    let video_hash = video_hash_opt.ok_or_else(|| actix_web::error::ErrorBadRequest("Missing hash"))?;
    let video_name = video_name_opt.ok_or_else(|| actix_web::error::ErrorBadRequest("Missing name"))?;
    let video_temp_path = video_temp_file_path.ok_or_else(|| actix_web::error::ErrorBadRequest("Missing video"))?;
    let calculated_hash = calculated_video_hash_opt.ok_or_else(|| actix_web::error::ErrorInternalServerError("Hash calculation failed"))?;
    let device_id = device_id_opt.unwrap_or_else(|| "web-client".to_string());

    if calculated_hash != video_hash {
        utils::cleanup_temp_files_spawn(Some(video_temp_path), None);
        return Ok(HttpResponse::BadRequest().json(serde_json::json!({"status": "error", "message": "Hash verification failed"})));
    }

    let res = ingest::process_video_file(
        &video_temp_path,
        &video_name,
        &video_hash,
        &device_id,
        &user_uuid,
        &pool,
        &config,
        true,
        client_created_at_opt,
    ).await;

    match res {
        Ok(ingest_res) => Ok(HttpResponse::Created().json(serde_json::json!({
            "status": "success",
            "hash": ingest_res.hash,
            "filename": ingest_res.filename,
            "name": ingest_res.name,
            "path": ingest_res.path,
            "thumbnail": if ingest_res.thumbnail { "yes" } else { "no" }
        }))),
        Err(e) => {
            error!("Failed to ingest video: {}", e);
            Ok(HttpResponse::InternalServerError().json(serde_json::json!({"status": "error", "message": e.to_string()})))
        }
    }
}
