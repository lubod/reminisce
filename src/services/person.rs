use actix_web::{get, put, post, web, HttpRequest, HttpResponse};
use log::{error, info};
use serde::{Deserialize, Serialize};
use utoipa::{IntoParams, ToSchema};
use crate::config::Config;
use crate::utils;
use crate::db::MainDbPool;

#[derive(Deserialize, IntoParams)]
pub struct PersonQuery {
    #[serde(default = "default_page")]
    pub page: usize,
    #[serde(default = "default_limit")]
    pub limit: usize,
}

fn default_page() -> usize { 1 }
fn default_limit() -> usize { 50 }

#[derive(Serialize, ToSchema)]
pub struct Person {
    pub id: i64,
    pub name: Option<String>,
    pub face_count: i32,
    pub representative_face_hash: Option<String>,
    pub representative_face_deviceid: Option<String>,
    pub representative_face_id: Option<i64>,
    pub representative_bbox: Option<Vec<i32>>,
    pub representative_face_url: Option<String>,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Serialize, ToSchema)]
pub struct PersonsResponse {
    pub persons: Vec<Person>,
    pub total: usize,
    pub page: usize,
    pub limit: usize,
}

#[derive(Serialize, ToSchema)]
pub struct PersonResponse {
    pub person: Person,
}

#[derive(Serialize, ToSchema)]
pub struct PersonImage {
    pub hash: String,
    pub deviceid: String,
    pub name: String,
    pub created_at: String,
    pub bbox: Vec<i32>,
    pub confidence: f32,
    pub face_id: i64,
    pub place: Option<String>,
    pub starred: bool,
    pub thumbnail_url: String,
}

#[derive(Serialize, ToSchema)]
pub struct PersonImagesResponse {
    pub images: Vec<PersonImage>,
    pub total: usize,
}

#[derive(Deserialize, ToSchema)]
pub struct UpdatePersonNameRequest {
    pub name: String,
}

#[derive(Deserialize, ToSchema)]
pub struct MergePersonsRequest {
    pub source_person_id: i64,
    pub target_person_id: i64,
}

/// Get all persons for authenticated user
#[utoipa::path(
    get,
    path = "/api/persons",
    params(PersonQuery),
    responses(
        (status = 200, description = "List of persons", body = PersonsResponse),
        (status = 401, description = "Unauthorized"),
        (status = 500, description = "Internal server error")
    ),
    tag = "Persons"
)]
#[get("/persons")]
pub async fn get_persons(
    req: HttpRequest,
    pool: web::Data<MainDbPool>,
    config: web::Data<Config>,
    query: web::Query<PersonQuery>,
) -> Result<HttpResponse, actix_web::Error> {
    let claims = match utils::authenticate_request(&req, "get_persons", config.get_api_key()) {
        Ok(claims) => claims,
        Err(response) => return Ok(response),
    };

    let user_uuid = utils::parse_user_uuid(&claims.user_id)?;
    let is_admin = claims.role == "admin";

    let client = utils::get_db_client(&pool.0).await?;

    let page = query.page.max(1);
    let limit = query.limit;
    let offset = (page - 1) * limit;

    // Get total count - admins see all persons
    let total_rows = if is_admin {
        client.query_one("SELECT COUNT(*) FROM persons", &[]).await
    } else {
        client.query_one("SELECT COUNT(*) FROM persons WHERE user_id = $1", &[&user_uuid]).await
    }.map_err(|e| {
        error!("Failed to count persons: {}", e);
        actix_web::error::ErrorInternalServerError("Query failed")
    })?;
    let total: i64 = total_rows.get(0);

    let base_query = "SELECT p.id, p.name, p.face_count, p.created_at, p.updated_at,
                    f.image_hash, f.image_deviceid, f.bbox_x, f.bbox_y, f.bbox_width, f.bbox_height, p.representative_face_id
             FROM persons p
             LEFT JOIN faces f ON p.representative_face_id = f.id";

    let rows = if is_admin {
        client.query(
            &format!("{} ORDER BY p.face_count DESC, p.updated_at DESC LIMIT $1 OFFSET $2", base_query),
            &[&(limit as i64), &(offset as i64)],
        ).await
    } else {
        client.query(
            &format!("{} WHERE p.user_id = $1 ORDER BY p.face_count DESC, p.updated_at DESC LIMIT $2 OFFSET $3", base_query),
            &[&user_uuid, &(limit as i64), &(offset as i64)],
        ).await
    }.map_err(|e| {
        error!("Failed to query persons: {}", e);
        actix_web::error::ErrorInternalServerError("Query failed")
    })?;

    let persons: Vec<Person> = rows
        .iter()
        .map(|row| {
            let face_id: Option<i64> = row.get(11);
            Person {
                id: row.get(0),
                name: row.get(1),
                face_count: row.get(2),
                created_at: row.get::<_, chrono::DateTime<chrono::Utc>>(3).to_rfc3339(),
                updated_at: row.get::<_, chrono::DateTime<chrono::Utc>>(4).to_rfc3339(),
                representative_face_hash: row.get(5),
                representative_face_deviceid: row.get(6),
                representative_face_id: face_id,
                representative_face_url: face_id.map(|id| format!("/api/face/{}/thumbnail", id)),
                representative_bbox: if row.get::<_, Option<String>>(5).is_some() {
                    Some(vec![
                        row.get(7),
                        row.get(8),
                        row.get(9),
                        row.get(10),
                    ])
                } else {
                    None
                },
            }
        })
        .collect();

    info!("Retrieved {} persons (page {}, limit {}) for user {}", persons.len(), page, limit, user_uuid);

    Ok(HttpResponse::Ok().json(PersonsResponse {
        total: total as usize,
        persons,
        page,
        limit,
    }))
}

/// Get a single person by ID
#[utoipa::path(
    get,
    path = "/api/persons/{id}",
    params(
        ("id" = i64, Path, description = "Person ID")
    ),
    responses(
        (status = 200, description = "Person found", body = PersonResponse),
        (status = 401, description = "Unauthorized"),
        (status = 404, description = "Person not found"),
        (status = 500, description = "Internal server error")
    ),
    tag = "Persons"
)]
#[get("/persons/{id}")]
pub async fn get_person(
    req: HttpRequest,
    path: web::Path<i64>,
    pool: web::Data<MainDbPool>,
    config: web::Data<Config>,
) -> Result<HttpResponse, actix_web::Error> {
    let claims = match utils::authenticate_request(&req, "get_person", config.get_api_key()) {
        Ok(claims) => claims,
        Err(response) => return Ok(response),
    };

    let user_uuid = utils::parse_user_uuid(&claims.user_id)?;
    let is_admin = claims.role == "admin";

    let person_id = path.into_inner();

    let client = utils::get_db_client(&pool.0).await?;

    let base_query = "SELECT p.id, p.name, p.face_count, p.created_at, p.updated_at,
                    f.image_hash, f.image_deviceid, f.bbox_x, f.bbox_y, f.bbox_width, f.bbox_height, p.representative_face_id
             FROM persons p
             LEFT JOIN faces f ON p.representative_face_id = f.id
             LEFT JOIN images i ON f.image_hash = i.hash AND f.image_deviceid = i.deviceid AND i.deleted_at IS NULL";

    let row = if is_admin {
        client.query_opt(
            &format!("{} WHERE p.id = $1", base_query),
            &[&person_id],
        ).await
    } else {
        client.query_opt(
            &format!("{} WHERE p.id = $1 AND p.user_id = $2", base_query),
            &[&person_id, &user_uuid],
        ).await
    }.map_err(|e| {
        error!("Failed to query person: {}", e);
        actix_web::error::ErrorInternalServerError("Query failed")
    })?;

    if let Some(row) = row {
        let face_id: Option<i64> = row.get(11);
        let person = Person {
            id: row.get(0),
            name: row.get(1),
            face_count: row.get(2),
            created_at: row.get::<_, chrono::DateTime<chrono::Utc>>(3).to_rfc3339(),
            updated_at: row.get::<_, chrono::DateTime<chrono::Utc>>(4).to_rfc3339(),
            representative_face_hash: row.get(5),
            representative_face_deviceid: row.get(6),
            representative_face_id: face_id,
            representative_face_url: face_id.map(|id| format!("/api/face/{}/thumbnail", id)),
            representative_bbox: if row.get::<_, Option<String>>(5).is_some() {
                Some(vec![
                    row.get(7),
                    row.get(8),
                    row.get(9),
                    row.get(10),
                ])
            } else {
                None
            },
        };

        Ok(HttpResponse::Ok().json(PersonResponse { person }))
    } else {
        Ok(HttpResponse::NotFound().json(serde_json::json!({
            "error": "Person not found"
        })))
    }
}

/// Get all images containing a specific person
#[utoipa::path(
    get,
    path = "/api/persons/{id}/images",
    params(
        ("id" = i64, Path, description = "Person ID")
    ),
    responses(
        (status = 200, description = "List of images", body = PersonImagesResponse),
        (status = 401, description = "Unauthorized"),
        (status = 404, description = "Person not found"),
        (status = 500, description = "Internal server error")
    ),
    tag = "Persons"
)]
#[get("/persons/{id}/images")]
pub async fn get_person_images(
    req: HttpRequest,
    path: web::Path<i64>,
    pool: web::Data<MainDbPool>,
    config: web::Data<Config>,
) -> Result<HttpResponse, actix_web::Error> {
    let claims = match utils::authenticate_request(&req, "get_person_images", config.get_api_key()) {
        Ok(claims) => claims,
        Err(response) => return Ok(response),
    };

    let user_uuid = utils::parse_user_uuid(&claims.user_id)?;
    let is_admin = claims.role == "admin";

    let person_id = path.into_inner();

    let client = utils::get_db_client(&pool.0).await?;

    // Verify person belongs to user (admins can access any person)
    let person_exists = if is_admin {
        client.query_opt("SELECT 1 FROM persons WHERE id = $1", &[&person_id]).await
    } else {
        client.query_opt("SELECT 1 FROM persons WHERE id = $1 AND user_id = $2", &[&person_id, &user_uuid]).await
    }.map_err(|e| {
        error!("Failed to verify person ownership: {}", e);
        actix_web::error::ErrorInternalServerError("Query failed")
    })?;

    if person_exists.is_none() {
        return Ok(HttpResponse::NotFound().json(serde_json::json!({
            "error": "Person not found"
        })));
    }

    let rows = client
        .query(
            "SELECT f.image_hash, f.image_deviceid, i.name, i.created_at,
                    f.bbox_x, f.bbox_y, f.bbox_width, f.bbox_height, f.confidence, f.id,
                    i.place,
                    (CASE WHEN s.hash IS NOT NULL THEN true ELSE false END) as starred
             FROM faces f
             JOIN images i ON f.image_hash = i.hash AND f.image_deviceid = i.deviceid
             LEFT JOIN starred_images s ON i.hash = s.hash AND s.user_id = $2
             WHERE f.person_id = $1 AND i.deleted_at IS NULL
             ORDER BY i.created_at DESC",
            &[&person_id, &user_uuid],
        )
        .await
        .map_err(|e| {
            error!("Failed to query person images: {}", e);
            actix_web::error::ErrorInternalServerError("Query failed")
        })?;

    let images: Vec<PersonImage> = rows
        .iter()
        .map(|row| {
            let hash: String = row.get(0);
            PersonImage {
                hash: hash.clone(),
                deviceid: row.get(1),
                name: row.get(2),
                created_at: row.get::<_, chrono::DateTime<chrono::Utc>>(3).to_rfc3339(),
                bbox: vec![row.get(4), row.get(5), row.get(6), row.get(7)],
                confidence: row.get(8),
                face_id: row.get(9),
                place: row.get(10),
                starred: row.get(11),
                thumbnail_url: format!("/api/thumbnail/{}", hash),
            }
        })
        .collect();

    info!("Retrieved {} images for person {}", images.len(), person_id);

    Ok(HttpResponse::Ok().json(PersonImagesResponse {
        total: images.len(),
        images,
    }))
}

/// Update person name
#[utoipa::path(
    put,
    path = "/api/persons/{id}/name",
    params(
        ("id" = i64, Path, description = "Person ID")
    ),
    request_body = UpdatePersonNameRequest,
    responses(
        (status = 200, description = "Name updated"),
        (status = 401, description = "Unauthorized"),
        (status = 404, description = "Person not found"),
        (status = 500, description = "Internal server error")
    ),
    tag = "Persons"
)]
#[put("/persons/{id}/name")]
pub async fn update_person_name(
    req: HttpRequest,
    path: web::Path<i64>,
    body: web::Json<UpdatePersonNameRequest>,
    pool: web::Data<MainDbPool>,
    config: web::Data<Config>,
) -> Result<HttpResponse, actix_web::Error> {
    let claims = match utils::authenticate_request(&req, "update_person_name", config.get_api_key()) {
        Ok(claims) => claims,
        Err(response) => return Ok(response),
    };

    let user_uuid = utils::parse_user_uuid(&claims.user_id)?;

    let person_id = path.into_inner();

    if body.name.trim().is_empty() {
        return Ok(HttpResponse::BadRequest().json(serde_json::json!({
            "error": "Name cannot be empty"
        })));
    }

    let client = utils::get_db_client(&pool.0).await?;

    let result = client
        .execute(
            "UPDATE persons SET name = $1, updated_at = NOW() WHERE id = $2 AND user_id = $3",
            &[&body.name, &person_id, &user_uuid],
        )
        .await
        .map_err(|e| {
            error!("Failed to update person name: {}", e);
            actix_web::error::ErrorInternalServerError("Update failed")
        })?;

    if result == 0 {
        return Ok(HttpResponse::NotFound().json(serde_json::json!({
            "error": "Person not found"
        })));
    }

    info!("Updated name for person {} to '{}'", person_id, body.name);

    Ok(HttpResponse::Ok().json(serde_json::json!({
        "success": true,
        "name": body.name
    })))
}

/// Merge two person clusters
#[utoipa::path(
    post,
    path = "/api/persons/merge",
    request_body = MergePersonsRequest,
    responses(
        (status = 200, description = "Persons merged"),
        (status = 400, description = "Invalid request"),
        (status = 401, description = "Unauthorized"),
        (status = 500, description = "Internal server error")
    ),
    tag = "Persons"
)]
#[post("/persons/merge")]
pub async fn merge_persons(
    req: HttpRequest,
    body: web::Json<MergePersonsRequest>,
    pool: web::Data<MainDbPool>,
    config: web::Data<Config>,
) -> Result<HttpResponse, actix_web::Error> {
    let claims = match utils::authenticate_request(&req, "merge_persons", config.get_api_key()) {
        Ok(claims) => claims,
        Err(response) => return Ok(response),
    };

    let user_uuid = utils::parse_user_uuid(&claims.user_id)?;

    if body.source_person_id == body.target_person_id {
        return Ok(HttpResponse::BadRequest().json(serde_json::json!({
            "error": "Cannot merge person with itself"
        })));
    }

    let client = utils::get_db_client(&pool.0).await?;

    // Verify both persons belong to user
    let count = client
        .query_one(
            "SELECT COUNT(*) FROM persons WHERE id IN ($1, $2) AND user_id = $3",
            &[&body.source_person_id, &body.target_person_id, &user_uuid],
        )
        .await
        .map_err(|e| {
            error!("Failed to verify person ownership: {}", e);
            actix_web::error::ErrorInternalServerError("Query failed")
        })?;

    let count: i64 = count.get(0);
    if count != 2 {
        return Ok(HttpResponse::BadRequest().json(serde_json::json!({
            "error": "Invalid person IDs"
        })));
    }

    // Move all faces from source to target
    client
        .execute(
            "UPDATE faces SET person_id = $1 WHERE person_id = $2",
            &[&body.target_person_id, &body.source_person_id],
        )
        .await
        .map_err(|e| {
            error!("Failed to merge faces: {}", e);
            actix_web::error::ErrorInternalServerError("Merge failed")
        })?;

    // Update target person stats
    client
        .execute(
            "UPDATE persons SET
                face_count = (SELECT COUNT(*) FROM faces WHERE person_id = $1),
                representative_embedding = (SELECT AVG(embedding) FROM faces WHERE person_id = $1),
                updated_at = NOW()
             WHERE id = $1",
            &[&body.target_person_id],
        )
        .await
        .map_err(|e| {
            error!("Failed to update target person: {}", e);
            actix_web::error::ErrorInternalServerError("Update failed")
        })?;

    // Delete source person
    client
        .execute(
            "DELETE FROM persons WHERE id = $1",
            &[&body.source_person_id],
        )
        .await
        .map_err(|e| {
            error!("Failed to delete source person: {}", e);
            actix_web::error::ErrorInternalServerError("Delete failed")
        })?;

    info!("Merged person {} into {}", body.source_person_id, body.target_person_id);

    Ok(HttpResponse::Ok().json(serde_json::json!({
        "success": true
    })))
}
