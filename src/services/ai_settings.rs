use actix_web::{get, put, web, HttpRequest, HttpResponse};
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;
use log::{error, info};

use crate::config::Config;
use crate::db::MainDbPool;
use crate::utils;

/// Load AI and Backup settings from database for the admin user on server startup
pub async fn load_ai_settings_from_db(
    pool: &deadpool_postgres::Pool,
    config: &Config,
) -> Result<(), Box<dyn std::error::Error>> {
    let client = pool.get().await?;

    // Get admin user's UUID
    let row = client
        .query_opt("SELECT id FROM users WHERE role = 'admin' LIMIT 1", &[])
        .await?;

    if let Some(row) = row {
        let admin_id: uuid::Uuid = row.get(0);

        // Try to get settings for admin user
        let settings_row = client
            .query_opt(
                "SELECT enable_ai_descriptions, enable_embeddings, embedding_parallel_count,
                        enable_face_detection, face_detection_parallel_count, enable_media_backup
                 FROM ai_settings WHERE user_id = $1",
                &[&admin_id],
            )
            .await?;

        if let Some(settings) = settings_row {
            // Load settings from database
            let enable_ai_descriptions = settings.get::<_, bool>(0);
            let enable_embeddings = settings.get::<_, bool>(1);
            let embedding_parallel_count = settings.get::<_, i32>(2) as usize;
            let enable_face_detection = settings.get::<_, bool>(3);
            let face_detection_parallel_count = settings.get::<_, i32>(4) as usize;
            let enable_media_backup = settings.get::<_, bool>(5);

            // Update in-memory config
            config.enable_ai_descriptions.store(enable_ai_descriptions, std::sync::atomic::Ordering::Relaxed);
            config.enable_embeddings.store(enable_embeddings, std::sync::atomic::Ordering::Relaxed);
            config.embedding_parallel_count.store(embedding_parallel_count, std::sync::atomic::Ordering::Relaxed);
            config.enable_face_detection.store(enable_face_detection, std::sync::atomic::Ordering::Relaxed);
            config.face_detection_parallel_count.store(face_detection_parallel_count, std::sync::atomic::Ordering::Relaxed);
            config.enable_media_backup.store(enable_media_backup, std::sync::atomic::Ordering::Relaxed);

            info!("Loaded Settings from DB: descriptions={}, embeddings={}, embedding_parallel={}, face_detection={}, face_parallel={}, backup={}",
                  enable_ai_descriptions, enable_embeddings, embedding_parallel_count,
                  enable_face_detection, face_detection_parallel_count, enable_media_backup);
        } else {
            // Create default settings for admin user
            client
                .execute(
                    "INSERT INTO ai_settings (user_id) VALUES ($1)",
                    &[&admin_id],
                )
                .await?;

            info!("Created default settings for admin user in database");
        }
    } else {
        info!("No admin user found, using default settings");
    }

    Ok(())
}

#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct AiSettingsResponse {
    pub enable_ai_descriptions: bool,
    pub enable_embeddings: bool,
    pub embedding_parallel_count: usize,
    pub enable_face_detection: bool,
    pub face_detection_parallel_count: usize,
    pub enable_media_backup: bool,
}

#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct UpdateAiSettingsRequest {
    pub enable_ai_descriptions: Option<bool>,
    pub enable_embeddings: Option<bool>,
    pub embedding_parallel_count: Option<usize>,
    pub enable_face_detection: Option<bool>,
    pub face_detection_parallel_count: Option<usize>,
    pub enable_media_backup: Option<bool>,
}

#[utoipa::path(
    get,
    path = "/ai-settings",
    responses(
        (status = 200, description = "Current processing settings", body = AiSettingsResponse),
        (status = 401, description = "Unauthorized"),
        (status = 403, description = "Forbidden - Admin access required")
    ),
    security(
        ("bearer_auth" = [])
    ),
    tag = "Settings"
)]
#[get("/ai-settings")]
pub async fn get_ai_settings(
    req: HttpRequest,
    pool: web::Data<MainDbPool>,
    config: web::Data<Config>,
) -> Result<HttpResponse, actix_web::Error> {
    let claims = match utils::authenticate_request(&req, "get_ai_settings", config.get_api_key()) {
        Ok(claims) => claims,
        Err(response) => return Ok(response),
    };

    // Check if user is admin
    if claims.role != "admin" {
        return Ok(HttpResponse::Forbidden().json(serde_json::json!({
            "error": "Admin access required"
        })));
    }

    let user_uuid = utils::parse_user_uuid(&claims.user_id)?;
    let client = utils::get_db_client(&pool.0).await?;

    // Try to get settings from database, create with defaults if not exists
    let row = client
        .query_opt(
            "SELECT enable_ai_descriptions, enable_embeddings, embedding_parallel_count,
                    enable_face_detection, face_detection_parallel_count, enable_media_backup
             FROM ai_settings WHERE user_id = $1",
            &[&user_uuid],
        )
        .await
        .map_err(|e| {
            error!("Failed to query settings: {}", e);
            actix_web::error::ErrorInternalServerError("Query failed")
        })?;

    let (enable_ai_descriptions, enable_embeddings, embedding_parallel_count,
         enable_face_detection, face_detection_parallel_count, enable_media_backup) = if let Some(row) = row {
        (
            row.get::<_, bool>(0),
            row.get::<_, bool>(1),
            row.get::<_, i32>(2) as usize,
            row.get::<_, bool>(3),
            row.get::<_, i32>(4) as usize,
            row.get::<_, bool>(5),
        )
    } else {
        // Auto-provision user from relay JWT if they don't exist locally
        crate::utils::ensure_user_exists(&client, &claims).await?;

        // Create default settings for user
        client
            .execute(
                "INSERT INTO ai_settings (user_id) VALUES ($1)",
                &[&user_uuid],
            )
            .await
            .map_err(|e| {
                error!("Failed to create default settings: {}", e);
                actix_web::error::ErrorInternalServerError("Failed to create settings")
            })?;

        // Return defaults
        (true, true, 10, true, 10, false)
    };

    // Update in-memory config to match database
    config.enable_ai_descriptions.store(enable_ai_descriptions, std::sync::atomic::Ordering::Relaxed);
    config.enable_embeddings.store(enable_embeddings, std::sync::atomic::Ordering::Relaxed);
    config.embedding_parallel_count.store(embedding_parallel_count, std::sync::atomic::Ordering::Relaxed);
    config.enable_face_detection.store(enable_face_detection, std::sync::atomic::Ordering::Relaxed);
    config.face_detection_parallel_count.store(face_detection_parallel_count, std::sync::atomic::Ordering::Relaxed);
    config.enable_media_backup.store(enable_media_backup, std::sync::atomic::Ordering::Relaxed);

    Ok(HttpResponse::Ok().json(AiSettingsResponse {
        enable_ai_descriptions,
        enable_embeddings,
        embedding_parallel_count,
        enable_face_detection,
        face_detection_parallel_count,
        enable_media_backup,
    }))
}

#[utoipa::path(
    put,
    path = "/ai-settings",
    request_body = UpdateAiSettingsRequest,
    responses(
        (status = 200, description = "Settings updated successfully", body = AiSettingsResponse),
        (status = 400, description = "Invalid request"),
        (status = 401, description = "Unauthorized"),
        (status = 403, description = "Forbidden - Admin access required")
    ),
    security(
        ("bearer_auth" = [])
    ),
    tag = "Settings"
)]
#[put("/ai-settings")]
pub async fn update_ai_settings(
    http_req: HttpRequest,
    pool: web::Data<MainDbPool>,
    config: web::Data<Config>,
    req: web::Json<UpdateAiSettingsRequest>,
) -> Result<HttpResponse, actix_web::Error> {
    let claims = match utils::authenticate_request(&http_req, "update_ai_settings", config.get_api_key()) {
        Ok(claims) => claims,
        Err(response) => return Ok(response),
    };

    // Check if user is admin
    if claims.role != "admin" {
        return Ok(HttpResponse::Forbidden().json(serde_json::json!({
            "error": "Admin access required"
        })));
    }

    let user_uuid = utils::parse_user_uuid(&claims.user_id)?;

    // Validate counts if provided
    if let Some(count) = req.embedding_parallel_count {
        if count == 0 || count > 100 {
            return Ok(HttpResponse::BadRequest().json(serde_json::json!({
                "error": "embedding_parallel_count must be between 1 and 100"
            })));
        }
    }
    if let Some(count) = req.face_detection_parallel_count {
        if count == 0 || count > 100 {
            return Ok(HttpResponse::BadRequest().json(serde_json::json!({
                "error": "face_detection_parallel_count must be between 1 and 100"
            })));
        }
    }

    let client = utils::get_db_client(&pool.0).await?;

    // Ensure the user has a settings row (create with defaults if not exists)
    client
        .execute(
            "INSERT INTO ai_settings (user_id)
             VALUES ($1)
             ON CONFLICT (user_id) DO NOTHING",
            &[&user_uuid],
        )
        .await
        .map_err(|e| {
            error!("Failed to ensure settings exist: {}", e);
            actix_web::error::ErrorInternalServerError("Database error")
        })?;

    // Update provided fields
    if let Some(enable) = req.enable_ai_descriptions {
        client.execute("UPDATE ai_settings SET enable_ai_descriptions = $1, updated_at = NOW() WHERE user_id = $2", &[&enable, &user_uuid]).await.ok();
    }
    if let Some(enable) = req.enable_embeddings {
        client.execute("UPDATE ai_settings SET enable_embeddings = $1, updated_at = NOW() WHERE user_id = $2", &[&enable, &user_uuid]).await.ok();
    }
    if let Some(count) = req.embedding_parallel_count {
        client.execute("UPDATE ai_settings SET embedding_parallel_count = $1, updated_at = NOW() WHERE user_id = $2", &[&(count as i32), &user_uuid]).await.ok();
    }
    if let Some(enable) = req.enable_face_detection {
        client.execute("UPDATE ai_settings SET enable_face_detection = $1, updated_at = NOW() WHERE user_id = $2", &[&enable, &user_uuid]).await.ok();
    }
    if let Some(count) = req.face_detection_parallel_count {
        client.execute("UPDATE ai_settings SET face_detection_parallel_count = $1, updated_at = NOW() WHERE user_id = $2", &[&(count as i32), &user_uuid]).await.ok();
    }
    if let Some(enable) = req.enable_media_backup {
        client.execute("UPDATE ai_settings SET enable_media_backup = $1, updated_at = NOW() WHERE user_id = $2", &[&enable, &user_uuid]).await.ok();
    }

    // Fetch updated settings
    let row = client
        .query_one(
            "SELECT enable_ai_descriptions, enable_embeddings, embedding_parallel_count,
                    enable_face_detection, face_detection_parallel_count, enable_media_backup
             FROM ai_settings WHERE user_id = $1",
            &[&user_uuid],
        )
        .await
        .map_err(|e| {
            error!("Failed to fetch updated settings: {}", e);
            actix_web::error::ErrorInternalServerError("Query failed")
        })?;

    let enable_ai_descriptions = row.get::<_, bool>(0);
    let enable_embeddings = row.get::<_, bool>(1);
    let embedding_parallel_count = row.get::<_, i32>(2) as usize;
    let enable_face_detection = row.get::<_, bool>(3);
    let face_detection_parallel_count = row.get::<_, i32>(4) as usize;
    let enable_media_backup = row.get::<_, bool>(5);

    // Update in-memory config
    config.enable_ai_descriptions.store(enable_ai_descriptions, std::sync::atomic::Ordering::Relaxed);
    config.enable_embeddings.store(enable_embeddings, std::sync::atomic::Ordering::Relaxed);
    config.embedding_parallel_count.store(embedding_parallel_count, std::sync::atomic::Ordering::Relaxed);
    config.enable_face_detection.store(enable_face_detection, std::sync::atomic::Ordering::Relaxed);
    config.face_detection_parallel_count.store(face_detection_parallel_count, std::sync::atomic::Ordering::Relaxed);
    config.enable_media_backup.store(enable_media_backup, std::sync::atomic::Ordering::Relaxed);

    info!("Updated settings for user {}: descriptions={}, embeddings={}, embedding_parallel={}, face_detection={}, face_parallel={}, backup={}",
          user_uuid, enable_ai_descriptions, enable_embeddings, embedding_parallel_count,
          enable_face_detection, face_detection_parallel_count, enable_media_backup);

    Ok(HttpResponse::Ok().json(AiSettingsResponse {
        enable_ai_descriptions,
        enable_embeddings,
        embedding_parallel_count,
        enable_face_detection,
        face_detection_parallel_count,
        enable_media_backup,
    }))
}