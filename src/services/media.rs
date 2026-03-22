use actix_files;
use actix_web::{ get, post, web, HttpRequest, HttpResponse };
use log::{ error, info, warn };
use serde::{Serialize, Deserialize};
use serde_json;
use utoipa::{ToSchema, IntoParams};
use std::collections::HashSet;

use crate::config::Config;
use crate::utils;
use crate::db::MainDbPool;

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
    let claims = match utils::authenticate_request(&req, "get_image", config.get_api_key()) {
        Ok(claims) => claims,
        Err(response) => return Ok(response),
    };

    let hash_to_find = path.into_inner();
    let client = utils::get_db_client(&pool.0).await?;
    let user_uuid = utils::parse_user_uuid(&claims.user_id)?;

    // For admin users, allow access to any image; for regular users, filter by user_id
    let row = if claims.role == "admin" {
        client
            .query_opt(
                "SELECT name, place, ext FROM images WHERE hash = $1 AND deleted_at IS NULL",
                &[&hash_to_find]
            ).await
            .map_err(|e| {
                error!("Failed to query image from database: {}", e);
                actix_web::error::ErrorInternalServerError("Failed to retrieve image info")
            })?
    } else {
        client
            .query_opt(
                "SELECT name, place, ext FROM images WHERE user_id = $1 AND hash = $2 AND deleted_at IS NULL",
                &[&user_uuid, &hash_to_find]
            ).await
            .map_err(|e| {
                error!("Failed to query image from database: {}", e);
                actix_web::error::ErrorInternalServerError("Failed to retrieve image info")
            })?
    };

    if let Some(row) = row {
        let original_name: String = row.get(0);
        let place: Option<String> = row.get(1);
        let extension: String = row.get(2);

        let filename = format!("{}.{}", hash_to_find, extension);
        let sub_dir_path = utils::get_subdirectory_path(config.get_images_dir(), &hash_to_find);
        let image_path = sub_dir_path.join(&filename);

        // Guess the MIME type from the file extension for the Content-Type header.
        let mime_type = mime_guess::from_path(&image_path).first_or_octet_stream();

        match tokio::fs::read(&image_path).await {
            Ok(data) => {
                info!("Serving image: {:?}", &image_path);
                let mut response = HttpResponse::Ok();
                response.content_type(mime_type.as_ref());
                response.insert_header(("Content-Disposition", format!("inline; filename=\"{}\"", original_name)));

                // Add place as a custom header if available
                if let Some(place_value) = place {
                    response.insert_header(("X-Image-Place", place_value));
                }

                Ok(response.body(data))
            }
            Err(e) => {
                error!(
                    "Local image file not found. Hash: '{}', Path: {:?}, Error: {}",
                    &hash_to_find,
                    &image_path,
                    e
                );
                Err(actix_web::error::ErrorInternalServerError("Could not read image file."))
            }
        }
    } else {
        warn!("Image not found for hash: '{}'", &hash_to_find);
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
    let claims = match utils::authenticate_request(&req, "get_video", config.get_api_key()) {
        Ok(claims) => claims,
        Err(response) => return Ok(response),
    };

    let hash_to_find = path.into_inner();
    let client = utils::get_db_client(&pool.0).await?;
    let user_uuid = utils::parse_user_uuid(&claims.user_id)?;

    // For admin users, allow access to any video; for regular users, filter by user_id
    let row = if claims.role == "admin" {
        client
            .query_opt(
                "SELECT name, ext FROM videos WHERE hash = $1 AND deleted_at IS NULL",
                &[&hash_to_find]
            ).await
            .map_err(|e| {
                error!("Failed to query video from database: {}", e);
                actix_web::error::ErrorInternalServerError("Failed to retrieve video info")
            })?
    } else {
        client
            .query_opt(
                "SELECT name, ext FROM videos WHERE user_id = $1 AND hash = $2 AND deleted_at IS NULL",
                &[&user_uuid, &hash_to_find]
            ).await
            .map_err(|e| {
                error!("Failed to query video from database: {}", e);
                actix_web::error::ErrorInternalServerError("Failed to retrieve video info")
            })?
    };

    if let Some(row) = row {
        let original_name: String = row.get(0);
        let extension: String = row.get(1);

        let filename = format!("{}.{}", hash_to_find, extension);
        let sub_dir_path = utils::get_subdirectory_path(config.get_videos_dir(), &hash_to_find);
        let video_path = sub_dir_path.join(&filename);

        // Guess the MIME type from the file extension for the Content-Type header.
        let mime_type = mime_guess::from_path(&video_path).first_or_octet_stream();

        match actix_files::NamedFile::open(&video_path) {
            Ok(file) => {
                info!("Serving video: {:?}", &video_path);
                Ok(file
                    .set_content_type(mime_type)
                    .set_content_disposition(actix_web::http::header::ContentDisposition {
                        disposition: actix_web::http::header::DispositionType::Inline,
                        parameters: vec![actix_web::http::header::DispositionParam::Filename(original_name.clone())],
                    })
                    .into_response(&req))
            }
            Err(e) => {
                error!(
                    "Local video file not found. Hash: '{}', Path: {:?}, Error: {}",
                    &hash_to_find,
                    &video_path,
                    e
                );
                Err(actix_web::error::ErrorInternalServerError("Could not open video file for streaming."))
            }
        }
    } else {
        warn!("Video not found for hash: '{}'", &hash_to_find);
        Ok(
            HttpResponse::NotFound().json(
                serde_json::json!({"status": "error", "message": "Video not found."})
            )
        )
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
    "starred": false
}))]
pub struct ImageMetadata {
    pub hash: String,
    pub name: String,
    pub description: Option<String>,
    pub place: Option<String>,
    pub created_at: String,
    pub exif: Option<String>,
    pub starred: bool,
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
    let claims = match utils::authenticate_request(&req, "get_image_metadata", config.get_api_key()) {
        Ok(claims) => claims,
        Err(response) => return Ok(response),
    };

    let hash_to_find = path.into_inner();
    let user_uuid = utils::parse_user_uuid(&claims.user_id)?;
    let client = utils::get_db_client(&pool.0).await?;

    // For admin users, allow access to any image; for regular users, filter by user_id
    let row = if claims.role == "admin" {
        client
            .query_opt(
                "SELECT i.hash, i.name, i.description, i.place, i.created_at, i.exif, CASE WHEN s.hash IS NOT NULL THEN true ELSE false END as starred FROM images i LEFT JOIN starred_images s ON i.hash = s.hash AND s.user_id = $1 WHERE i.hash = $2 AND i.deleted_at IS NULL",
                &[&user_uuid, &hash_to_find]
            ).await
            .map_err(|e| {
                error!("Failed to query image metadata from database: {}", e);
                actix_web::error::ErrorInternalServerError("Failed to retrieve image metadata")
            })?
    } else {
        client
            .query_opt(
                "SELECT i.hash, i.name, i.description, i.place, i.created_at, i.exif, CASE WHEN s.hash IS NOT NULL THEN true ELSE false END as starred FROM images i LEFT JOIN starred_images s ON i.hash = s.hash AND s.user_id = $1 WHERE i.user_id = $1 AND i.hash = $2 AND i.deleted_at IS NULL",
                &[&user_uuid, &hash_to_find]
            ).await
            .map_err(|e| {
                error!("Failed to query image metadata from database: {}", e);
                actix_web::error::ErrorInternalServerError("Failed to retrieve image metadata")
            })?
    };

    if let Some(row) = row {
        let metadata = ImageMetadata {
            hash: row.get(0),
            name: row.get(1),
            description: row.get(2),
            place: row.get(3),
            created_at: row.get::<_, chrono::DateTime<chrono::Utc>>(4).to_rfc3339(),
            exif: row.get(5),
            starred: row.get(6),
        };

        info!("Serving metadata for image: {}", hash_to_find);
        Ok(HttpResponse::Ok().json(metadata))
    } else {
        warn!("Image not found for hash: '{}'", &hash_to_find);
        Ok(
            HttpResponse::NotFound().json(
                serde_json::json!({"status": "error", "message": "Image not found."})
            )
        )
    }
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
    is_admin: bool,
) -> Result<HttpResponse, actix_web::Error> {
    let mut client = utils::get_db_client(pool).await?;

    let transaction = client.transaction().await.map_err(|e| {
        error!("Failed to start transaction: {}", e);
        actix_web::error::ErrorInternalServerError("Database error")
    })?;

    // Verify the media exists and user has access
    let exists = if is_admin {
        transaction
            .query_opt(&format!("SELECT 1 FROM {} WHERE hash = $1 AND deleted_at IS NULL", media_table), &[&hash])
            .await
    } else {
        transaction
            .query_opt(
                &format!("SELECT 1 FROM {} WHERE user_id = $1 AND hash = $2 AND deleted_at IS NULL", media_table),
                &[user_uuid, &hash]
            )
            .await
    }.map_err(|e| {
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
    let claims = match utils::authenticate_request(&req, "toggle_image_star", config.get_api_key()) {
        Ok(claims) => claims,
        Err(response) => return Ok(response),
    };
    let user_uuid = utils::parse_user_uuid(&claims.user_id)?;
    let hash = path.into_inner();
    toggle_media_star_inner(&pool.0, "images", "starred_images", &hash, &user_uuid, claims.role == "admin").await
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
    let claims = match utils::authenticate_request(&req, "toggle_video_star", config.get_api_key()) {
        Ok(claims) => claims,
        Err(response) => return Ok(response),
    };
    let user_uuid = utils::parse_user_uuid(&claims.user_id)?;
    let hash = path.into_inner();
    toggle_media_star_inner(&pool.0, "videos", "starred_videos", &hash, &user_uuid, claims.role == "admin").await
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
    let claims = match utils::authenticate_request(&req, "get_device_ids", config.get_api_key()) {
        Ok(claims) => claims,
        Err(response) => return Ok(response),
    };

    let client = utils::get_db_client(&pool.0).await?;
    let user_uuid = utils::parse_user_uuid(&claims.user_id)?;
    let is_admin = claims.role == "admin";

    // Build queries dynamically: admin sees all, non-admin filtered by user_id
    let mut device_set = HashSet::new();

    for table in &["images", "videos"] {
        let (query, params): (String, Vec<&(dyn tokio_postgres::types::ToSql + Sync)>) = if is_admin {
            (format!("SELECT DISTINCT deviceid FROM {} WHERE deviceid IS NOT NULL AND deleted_at IS NULL", table), vec![])
        } else {
            (format!("SELECT DISTINCT deviceid FROM {} WHERE user_id = $1 AND deviceid IS NOT NULL AND deleted_at IS NULL", table),
             vec![&user_uuid as &(dyn tokio_postgres::types::ToSql + Sync)])
        };

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
    let claims = match utils::authenticate_request(&req, "get_random_image", config.get_api_key()) {
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

    if claims.role != "admin" {
        conditions.push(format!("i.user_id = ${}", params.len() + 1));
        params.push(&user_uuid as &(dyn tokio_postgres::types::ToSql + Sync));
    }

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
        (status = 403, description = "Forbidden (Admin only)"),
        (status = 500, description = "Internal server error")
    )
)]
#[get("/trash")]
pub async fn get_trash(
    req: HttpRequest,
    pool: web::Data<MainDbPool>,
    config: web::Data<Config>,
) -> Result<HttpResponse, actix_web::Error> {
    let claims = match utils::authenticate_request(&req, "get_trash", config.get_api_key()) {
        Ok(claims) => claims,
        Err(response) => return Ok(response),
    };

    if claims.role != "admin" {
        return Ok(HttpResponse::Forbidden().json(serde_json::json!({"status": "error", "message": "Only admins can view trash."})));
    }

    let client = utils::get_db_client(&pool.0).await?;

    let rows = client
        .query(
            "SELECT hash, name, created_at, ext, COALESCE(type, ''), deviceid, deleted_at, 'image' as media_type \
             FROM images WHERE deleted_at IS NOT NULL \
             UNION ALL \
             SELECT hash, name, created_at, ext, COALESCE(type, ''), deviceid, deleted_at, 'video' as media_type \
             FROM videos WHERE deleted_at IS NOT NULL \
             ORDER BY deleted_at DESC \
             LIMIT 200",
            &[]
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

/// Shared implementation for soft-restoring images or videos (admin only).
async fn soft_restore_media(
    pool: &deadpool_postgres::Pool,
    table: &str,
    hash: &str,
) -> Result<HttpResponse, actix_web::Error> {
    let client = utils::get_db_client(pool).await?;

    let result = client
        .execute(
            &format!("UPDATE {} SET deleted_at = NULL WHERE hash = $1 AND deleted_at IS NOT NULL", table),
            &[&hash]
        ).await
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
        (status = 403, description = "Forbidden (Admin only)"),
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
    let claims = match utils::authenticate_request(&req, "restore_image", config.get_api_key()) {
        Ok(claims) => claims,
        Err(response) => return Ok(response),
    };

    if claims.role != "admin" {
        return Ok(HttpResponse::Forbidden().json(serde_json::json!({"status": "error", "message": "Only admins can restore media."})));
    }

    let hash = path.into_inner();
    soft_restore_media(&pool.0, "images", &hash).await
}

#[utoipa::path(
    post,
    path = "/video/{video_hash}/restore",
    responses(
        (status = 200, description = "Video restored"),
        (status = 401, description = "Unauthorized"),
        (status = 403, description = "Forbidden (Admin only)"),
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
    let claims = match utils::authenticate_request(&req, "restore_video", config.get_api_key()) {
        Ok(claims) => claims,
        Err(response) => return Ok(response),
    };

    if claims.role != "admin" {
        return Ok(HttpResponse::Forbidden().json(serde_json::json!({"status": "error", "message": "Only admins can restore media."})));
    }

    let hash = path.into_inner();
    soft_restore_media(&pool.0, "videos", &hash).await
}

/// Shared implementation for soft-deleting images or videos (admin only).
async fn soft_delete_media(
    pool: &deadpool_postgres::Pool,
    table: &str,
    hash: &str,
) -> Result<HttpResponse, actix_web::Error> {
    let client = utils::get_db_client(pool).await?;

    let result = client
        .execute(
            &format!("UPDATE {} SET deleted_at = NOW() WHERE hash = $1 AND deleted_at IS NULL", table),
            &[&hash]
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

    info!("{} marked as deleted: {}", table, hash);
    Ok(HttpResponse::Ok().json(serde_json::json!({"status": "success", "hash": hash})))
}

#[utoipa::path(
    post,
    path = "/image/{image_hash}/delete",
    responses(
        (status = 200, description = "Image marked as deleted"),
        (status = 401, description = "Unauthorized"),
        (status = 403, description = "Forbidden (Admin only)"),
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
    let claims = match utils::authenticate_request(&req, "delete_image", config.get_api_key()) {
        Ok(claims) => claims,
        Err(response) => return Ok(response),
    };

    if claims.role != "admin" {
        return Ok(HttpResponse::Forbidden().json(serde_json::json!({"status": "error", "message": "Only admins can delete media."})));
    }

    let hash = path.into_inner();
    soft_delete_media(&pool.0, "images", &hash).await
}

#[utoipa::path(
    post,
    path = "/video/{video_hash}/delete",
    responses(
        (status = 200, description = "Video marked as deleted"),
        (status = 401, description = "Unauthorized"),
        (status = 403, description = "Forbidden (Admin only)"),
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
    let claims = match utils::authenticate_request(&req, "delete_video", config.get_api_key()) {
        Ok(claims) => claims,
        Err(response) => return Ok(response),
    };

    if claims.role != "admin" {
        return Ok(HttpResponse::Forbidden().json(serde_json::json!({"status": "error", "message": "Only admins can delete media."})));
    }

    let hash = path.into_inner();
    soft_delete_media(&pool.0, "videos", &hash).await
}
