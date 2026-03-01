use actix_web::{get, post, delete, web, HttpRequest, HttpResponse};
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;
use log::error;

use crate::config::Config;
use crate::db::MainDbPool;
use crate::utils;

#[derive(Debug, Serialize, Deserialize, ToSchema, Clone)]
pub struct Label {
    pub id: i32,
    pub name: String,
    pub color: String,
    pub created_at: String,
}

#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct LabelsResponse {
    pub labels: Vec<Label>,
}

#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct CreateLabelRequest {
    pub name: String,
    #[serde(default = "default_label_color")]
    pub color: String,
}

fn default_label_color() -> String {
    "#3B82F6".to_string() // Blue
}

#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct AddLabelToMediaRequest {
    pub label_id: i32,
}

#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct MediaLabelsResponse {
    pub labels: Vec<Label>,
}

#[utoipa::path(
    get,
    path = "/api/labels",
    responses(
        (status = 200, description = "List of labels", body = LabelsResponse),
        (status = 401, description = "Unauthorized")
    ),
    tag = "Labels"
)]
#[get("/labels")]
pub async fn get_labels(
    req: HttpRequest,
    pool: web::Data<MainDbPool>,
    config: web::Data<Config>,
) -> Result<HttpResponse, actix_web::Error> {
    let claims = match utils::authenticate_request(&req, "get_labels", config.get_api_key()) {
        Ok(claims) => claims,
        Err(response) => return Ok(response),
    };

    let user_uuid = utils::parse_user_uuid(&claims.user_id)?;

    let client = utils::get_db_client(&pool.0).await?;

    let rows = client
        .query(
            "SELECT id, name, color, created_at FROM labels WHERE user_id = $1 ORDER BY name",
            &[&user_uuid],
        )
        .await
        .map_err(|e| {
            error!("Failed to query labels: {}", e);
            actix_web::error::ErrorInternalServerError("Query failed")
        })?;

    let labels: Vec<Label> = rows
        .iter()
        .map(|row| Label {
            id: row.get(0),
            name: row.get(1),
            color: row.get(2),
            created_at: row.get::<_, chrono::DateTime<chrono::Utc>>(3).to_rfc3339(),
        })
        .collect();

    Ok(HttpResponse::Ok().json(LabelsResponse { labels }))
}

#[utoipa::path(
    post,
    path = "/api/labels",
    request_body = CreateLabelRequest,
    responses(
        (status = 200, description = "Label created", body = Label),
        (status = 400, description = "Invalid request"),
        (status = 401, description = "Unauthorized"),
        (status = 409, description = "Label already exists")
    ),
    tag = "Labels"
)]
#[post("/labels")]
pub async fn create_label(
    req: HttpRequest,
    pool: web::Data<MainDbPool>,
    config: web::Data<Config>,
    body: web::Json<CreateLabelRequest>,
) -> Result<HttpResponse, actix_web::Error> {
    let claims = match utils::authenticate_request(&req, "create_label", config.get_api_key()) {
        Ok(claims) => claims,
        Err(response) => return Ok(response),
    };

    let user_uuid = utils::parse_user_uuid(&claims.user_id)?;

    if body.name.trim().is_empty() {
        return Ok(HttpResponse::BadRequest().json(serde_json::json!({
            "error": "Label name cannot be empty"
        })));
    }

    let client = utils::get_db_client(&pool.0).await?;

    let row = client
        .query_one(
            "INSERT INTO labels (user_id, name, color) VALUES ($1, $2, $3)
             ON CONFLICT (user_id, name) DO UPDATE SET color = EXCLUDED.color
             RETURNING id, name, color, created_at",
            &[&user_uuid, &body.name.trim(), &body.color],
        )
        .await
        .map_err(|e| {
            error!("Failed to create label: {}", e);
            actix_web::error::ErrorInternalServerError("Failed to create label")
        })?;

    let label = Label {
        id: row.get(0),
        name: row.get(1),
        color: row.get(2),
        created_at: row.get::<_, chrono::DateTime<chrono::Utc>>(3).to_rfc3339(),
    };

    Ok(HttpResponse::Ok().json(label))
}

#[utoipa::path(
    delete,
    path = "/api/labels/{id}",
    params(
        ("id" = i32, Path, description = "Label ID")
    ),
    responses(
        (status = 200, description = "Label deleted"),
        (status = 401, description = "Unauthorized"),
        (status = 404, description = "Label not found")
    ),
    tag = "Labels"
)]
#[delete("/labels/{id}")]
pub async fn delete_label(
    req: HttpRequest,
    pool: web::Data<MainDbPool>,
    config: web::Data<Config>,
    path: web::Path<i32>,
) -> Result<HttpResponse, actix_web::Error> {
    let claims = match utils::authenticate_request(&req, "delete_label", config.get_api_key()) {
        Ok(claims) => claims,
        Err(response) => return Ok(response),
    };

    let user_uuid = utils::parse_user_uuid(&claims.user_id)?;

    let label_id = path.into_inner();

    let client = utils::get_db_client(&pool.0).await?;

    let rows_affected = client
        .execute(
            "DELETE FROM labels WHERE id = $1 AND user_id = $2",
            &[&label_id, &user_uuid],
        )
        .await
        .map_err(|e| {
            error!("Failed to delete label: {}", e);
            actix_web::error::ErrorInternalServerError("Delete failed")
        })?;

    if rows_affected == 0 {
        return Ok(HttpResponse::NotFound().json(serde_json::json!({
            "error": "Label not found"
        })));
    }

    Ok(HttpResponse::Ok().json(serde_json::json!({
        "message": "Label deleted"
    })))
}

// Image label endpoints
#[utoipa::path(
    get,
    path = "/api/images/{hash}/labels",
    params(
        ("hash" = String, Path, description = "Image hash")
    ),
    responses(
        (status = 200, description = "Image labels", body = MediaLabelsResponse),
        (status = 401, description = "Unauthorized")
    ),
    tag = "Labels"
)]
#[get("/images/{hash}/labels")]
pub async fn get_image_labels(
    req: HttpRequest,
    pool: web::Data<MainDbPool>,
    config: web::Data<Config>,
    path: web::Path<String>,
) -> Result<HttpResponse, actix_web::Error> {
    let claims = match utils::authenticate_request(&req, "get_image_labels", config.get_api_key()) {
        Ok(claims) => claims,
        Err(response) => return Ok(response),
    };

    let user_uuid = utils::parse_user_uuid(&claims.user_id)?;

    let hash = path.into_inner();

    let client = utils::get_db_client(&pool.0).await?;

    let rows = client
        .query(
            "SELECT l.id, l.name, l.color, l.created_at
             FROM labels l
             INNER JOIN image_labels il ON il.label_id = l.id
             WHERE il.image_hash = $1 AND l.user_id = $2
             ORDER BY l.name",
            &[&hash, &user_uuid],
        )
        .await
        .map_err(|e| {
            error!("Failed to query image labels: {}", e);
            actix_web::error::ErrorInternalServerError("Query failed")
        })?;

    let labels: Vec<Label> = rows
        .iter()
        .map(|row| Label {
            id: row.get(0),
            name: row.get(1),
            color: row.get(2),
            created_at: row.get::<_, chrono::DateTime<chrono::Utc>>(3).to_rfc3339(),
        })
        .collect();

    Ok(HttpResponse::Ok().json(MediaLabelsResponse { labels }))
}

#[utoipa::path(
    post,
    path = "/api/images/{hash}/labels",
    params(
        ("hash" = String, Path, description = "Image hash")
    ),
    request_body = AddLabelToMediaRequest,
    responses(
        (status = 200, description = "Label added to image"),
        (status = 401, description = "Unauthorized"),
        (status = 404, description = "Image or label not found")
    ),
    tag = "Labels"
)]
#[post("/images/{hash}/labels")]
pub async fn add_image_label(
    req: HttpRequest,
    pool: web::Data<MainDbPool>,
    config: web::Data<Config>,
    path: web::Path<String>,
    body: web::Json<AddLabelToMediaRequest>,
) -> Result<HttpResponse, actix_web::Error> {
    let claims = match utils::authenticate_request(&req, "add_image_label", config.get_api_key()) {
        Ok(claims) => claims,
        Err(response) => return Ok(response),
    };

    let user_uuid = utils::parse_user_uuid(&claims.user_id)?;

    let hash = path.into_inner();

    let client = utils::get_db_client(&pool.0).await?;

    // Get deviceid for this image
    let row = client
        .query_opt(
            "SELECT deviceid FROM images WHERE hash = $1 AND user_id = $2 AND deleted_at IS NULL",
            &[&hash, &user_uuid],
        )
        .await
        .map_err(|e| {
            error!("Failed to query image: {}", e);
            actix_web::error::ErrorInternalServerError("Query failed")
        })?;

    let deviceid: String = match row {
        Some(r) => r.get(0),
        None => {
            return Ok(HttpResponse::NotFound().json(serde_json::json!({
                "error": "Image not found"
            })));
        }
    };

    // Verify label belongs to user
    let label_exists = client
        .query_opt(
            "SELECT 1 FROM labels WHERE id = $1 AND user_id = $2",
            &[&body.label_id, &user_uuid],
        )
        .await
        .map_err(|e| {
            error!("Failed to verify label: {}", e);
            actix_web::error::ErrorInternalServerError("Query failed")
        })?
        .is_some();

    if !label_exists {
        return Ok(HttpResponse::NotFound().json(serde_json::json!({
            "error": "Label not found"
        })));
    }

    // Add label to image
    client
        .execute(
            "INSERT INTO image_labels (image_hash, image_deviceid, label_id)
             VALUES ($1, $2, $3)
             ON CONFLICT DO NOTHING",
            &[&hash, &deviceid, &body.label_id],
        )
        .await
        .map_err(|e| {
            error!("Failed to add label to image: {}", e);
            actix_web::error::ErrorInternalServerError("Failed to add label")
        })?;

    Ok(HttpResponse::Ok().json(serde_json::json!({
        "message": "Label added to image"
    })))
}

#[utoipa::path(
    delete,
    path = "/api/images/{hash}/labels/{label_id}",
    params(
        ("hash" = String, Path, description = "Image hash"),
        ("label_id" = i32, Path, description = "Label ID")
    ),
    responses(
        (status = 200, description = "Label removed from image"),
        (status = 401, description = "Unauthorized")
    ),
    tag = "Labels"
)]
#[delete("/images/{hash}/labels/{label_id}")]
pub async fn remove_image_label(
    req: HttpRequest,
    pool: web::Data<MainDbPool>,
    config: web::Data<Config>,
    path: web::Path<(String, i32)>,
) -> Result<HttpResponse, actix_web::Error> {
    let _claims = match utils::authenticate_request(&req, "remove_image_label", config.get_api_key()) {
        Ok(claims) => claims,
        Err(response) => return Ok(response),
    };

    let (hash, label_id) = path.into_inner();

    let client = utils::get_db_client(&pool.0).await?;

    client
        .execute(
            "DELETE FROM image_labels WHERE image_hash = $1 AND label_id = $2",
            &[&hash, &label_id],
        )
        .await
        .map_err(|e| {
            error!("Failed to remove label from image: {}", e);
            actix_web::error::ErrorInternalServerError("Delete failed")
        })?;

    Ok(HttpResponse::Ok().json(serde_json::json!({
        "message": "Label removed from image"
    })))
}

// Video label endpoints (same as image endpoints but for videos)
#[get("/videos/{hash}/labels")]
pub async fn get_video_labels(
    req: HttpRequest,
    pool: web::Data<MainDbPool>,
    config: web::Data<Config>,
    path: web::Path<String>,
) -> Result<HttpResponse, actix_web::Error> {
    let claims = match utils::authenticate_request(&req, "get_video_labels", config.get_api_key()) {
        Ok(claims) => claims,
        Err(response) => return Ok(response),
    };

    let user_uuid = utils::parse_user_uuid(&claims.user_id)?;

    let hash = path.into_inner();

    let client = utils::get_db_client(&pool.0).await?;

    let rows = client
        .query(
            "SELECT l.id, l.name, l.color, l.created_at
             FROM labels l
             INNER JOIN video_labels vl ON vl.label_id = l.id
             WHERE vl.video_hash = $1 AND l.user_id = $2
             ORDER BY l.name",
            &[&hash, &user_uuid],
        )
        .await
        .map_err(|e| {
            error!("Failed to query video labels: {}", e);
            actix_web::error::ErrorInternalServerError("Query failed")
        })?;

    let labels: Vec<Label> = rows
        .iter()
        .map(|row| Label {
            id: row.get(0),
            name: row.get(1),
            color: row.get(2),
            created_at: row.get::<_, chrono::DateTime<chrono::Utc>>(3).to_rfc3339(),
        })
        .collect();

    Ok(HttpResponse::Ok().json(MediaLabelsResponse { labels }))
}

#[post("/videos/{hash}/labels")]
pub async fn add_video_label(
    req: HttpRequest,
    pool: web::Data<MainDbPool>,
    config: web::Data<Config>,
    path: web::Path<String>,
    body: web::Json<AddLabelToMediaRequest>,
) -> Result<HttpResponse, actix_web::Error> {
    let claims = match utils::authenticate_request(&req, "add_video_label", config.get_api_key()) {
        Ok(claims) => claims,
        Err(response) => return Ok(response),
    };

    let user_uuid = utils::parse_user_uuid(&claims.user_id)?;

    let hash = path.into_inner();

    let client = utils::get_db_client(&pool.0).await?;

    // Get deviceid for this video
    let row = client
        .query_opt(
            "SELECT deviceid FROM videos WHERE hash = $1 AND user_id = $2 AND deleted_at IS NULL",
            &[&hash, &user_uuid],
        )
        .await
        .map_err(|e| {
            error!("Failed to query video: {}", e);
            actix_web::error::ErrorInternalServerError("Query failed")
        })?;

    let deviceid: String = match row {
        Some(r) => r.get(0),
        None => {
            return Ok(HttpResponse::NotFound().json(serde_json::json!({
                "error": "Video not found"
            })));
        }
    };

    // Verify label belongs to user
    let label_exists = client
        .query_opt(
            "SELECT 1 FROM labels WHERE id = $1 AND user_id = $2",
            &[&body.label_id, &user_uuid],
        )
        .await
        .map_err(|e| {
            error!("Failed to verify label: {}", e);
            actix_web::error::ErrorInternalServerError("Query failed")
        })?
        .is_some();

    if !label_exists {
        return Ok(HttpResponse::NotFound().json(serde_json::json!({
            "error": "Label not found"
        })));
    }

    // Add label to video
    client
        .execute(
            "INSERT INTO video_labels (video_hash, video_deviceid, label_id)
             VALUES ($1, $2, $3)
             ON CONFLICT DO NOTHING",
            &[&hash, &deviceid, &body.label_id],
        )
        .await
        .map_err(|e| {
            error!("Failed to add label to video: {}", e);
            actix_web::error::ErrorInternalServerError("Failed to add label")
        })?;

    Ok(HttpResponse::Ok().json(serde_json::json!({
        "message": "Label added to video"
    })))
}

#[delete("/videos/{hash}/labels/{label_id}")]
pub async fn remove_video_label(
    req: HttpRequest,
    pool: web::Data<MainDbPool>,
    config: web::Data<Config>,
    path: web::Path<(String, i32)>,
) -> Result<HttpResponse, actix_web::Error> {
    let _claims = match utils::authenticate_request(&req, "remove_video_label", config.get_api_key()) {
        Ok(claims) => claims,
        Err(response) => return Ok(response),
    };

    let (hash, label_id) = path.into_inner();

    let client = utils::get_db_client(&pool.0).await?;

    client
        .execute(
            "DELETE FROM video_labels WHERE video_hash = $1 AND label_id = $2",
            &[&hash, &label_id],
        )
        .await
        .map_err(|e| {
            error!("Failed to remove label from video: {}", e);
            actix_web::error::ErrorInternalServerError("Delete failed")
        })?;

    Ok(HttpResponse::Ok().json(serde_json::json!({
        "message": "Label removed from video"
    })))
}
