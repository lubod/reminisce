use actix_web::{ get, web, HttpResponse, HttpRequest };
use serde::{Deserialize, Serialize};
use log::error;

use crate::config::Config;
use crate::db::MainDbPool;
use crate::db_instrumentation;
use crate::utils;

use utoipa::ToSchema;

#[derive(Serialize, Deserialize, ToSchema)]
pub struct StatsResponse {
    pub total_images: i64,
    pub total_videos: i64,
    pub total_users: i64,
    pub images_with_description: i64,
    pub starred_images: i64,
    pub starred_videos: i64,
    pub images_with_embedding: i64,
    pub verified_images: i64,
    pub verified_videos: i64,
    pub total_faces: i64,
    pub total_persons: i64,
    pub images_with_faces: i64,
    pub images_face_pending: i64,
    pub total_p2p_synced_images: i64,
    pub total_p2p_synced_videos: i64,
    pub thumbnail_count: i64,
}

#[utoipa::path(
    get,
    path = "/api/stats",
    responses(
        (status = 200, description = "Statistics", body = StatsResponse),
        (status = 401, description = "Unauthorized"),
        (status = 403, description = "Forbidden"),
        (status = 500, description = "Internal server error")
    )
)]
#[get("/stats")]
pub async fn get_stats(
    req: HttpRequest,
    pool: web::Data<MainDbPool>,
    config: web::Data<Config>
) -> Result<HttpResponse, actix_web::Error> {
    let claims = match utils::authenticate_request(&req, "get_stats", config.get_api_key()) {
        Ok(claims) => claims,
        Err(response) => return Ok(response),
    };

    if claims.role != "admin" {
        return Ok(HttpResponse::Forbidden().finish());
    }

    let client = utils::get_db_client(&pool.0).await?;

    // Combine all stats into a single query for better performance
    let query = "
        SELECT
            (SELECT COUNT(*) FROM images WHERE deleted_at IS NULL) as total_images,
            (SELECT COUNT(*) FROM videos WHERE deleted_at IS NULL) as total_videos,
            (SELECT COUNT(*) FROM users) as total_users,
            (SELECT COUNT(*) FROM images WHERE description IS NOT NULL AND description != '' AND deleted_at IS NULL) as images_with_description,
            (SELECT COUNT(DISTINCT hash) FROM starred_images WHERE hash IN (SELECT hash FROM images WHERE deleted_at IS NULL)) as starred_images,
            (SELECT COUNT(DISTINCT hash) FROM starred_videos WHERE hash IN (SELECT hash FROM videos WHERE deleted_at IS NULL)) as starred_videos,
            (SELECT COUNT(*) FROM images WHERE embedding_generated_at IS NOT NULL AND deleted_at IS NULL) as images_with_embedding,
            (SELECT COUNT(*) FROM images WHERE verification_status = 1 AND deleted_at IS NULL) as verified_images,
            (SELECT COUNT(*) FROM videos WHERE verification_status = 1 AND deleted_at IS NULL) as verified_videos,
            (SELECT COUNT(*) FROM faces WHERE (image_deviceid, image_hash) IN (SELECT deviceid, hash FROM images WHERE deleted_at IS NULL)) as total_faces,
            (SELECT COUNT(*) FROM persons) as total_persons,
            (SELECT COUNT(*) FROM (SELECT DISTINCT image_hash, image_deviceid FROM faces WHERE (image_deviceid, image_hash) IN (SELECT deviceid, hash FROM images WHERE deleted_at IS NULL)) AS distinct_images) as images_with_faces,
            (SELECT COUNT(*) FROM images WHERE face_detection_completed_at IS NULL AND deleted_at IS NULL) as images_face_pending,
            (SELECT COUNT(*) FROM images WHERE p2p_synced_at IS NOT NULL AND deleted_at IS NULL) as total_p2p_synced_images,
            (SELECT COUNT(*) FROM videos WHERE p2p_synced_at IS NOT NULL AND deleted_at IS NULL) as total_p2p_synced_videos,
            ((SELECT COUNT(*) FROM images WHERE has_thumbnail = true AND deleted_at IS NULL) + (SELECT COUNT(*) FROM videos WHERE has_thumbnail = true AND deleted_at IS NULL)) as thumbnail_count
    ";

    // Use instrumented query to log performance
    let row = db_instrumentation::instrumented_query_one(&client, query, &[], "get_stats")
        .await
        .map_err(|e| {
            error!("Failed to query stats: {}", e);
            actix_web::error::ErrorInternalServerError("Failed to retrieve statistics")
        })?;

    let stats = StatsResponse {
        total_images: row.get(0),
        total_videos: row.get(1),
        total_users: row.get(2),
        images_with_description: row.get(3),
        starred_images: row.get(4),
        starred_videos: row.get(5),
        images_with_embedding: row.get(6),
        verified_images: row.get(7),
        verified_videos: row.get(8),
        total_faces: row.get(9),
        total_persons: row.get(10),
        images_with_faces: row.get(11),
        images_face_pending: row.get(12),
        total_p2p_synced_images: row.get(13),
        total_p2p_synced_videos: row.get(14),
        thumbnail_count: row.get(15),
    };

    Ok(HttpResponse::Ok().json(stats))
}
