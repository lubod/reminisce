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

    // Single-pass aggregation: collapse all same-table counts with FILTER to avoid
    // repeated full-table scans. faces uses a JOIN instead of an IN subquery.
    let query = "
        WITH img AS (
            SELECT
                COUNT(*) FILTER (WHERE deleted_at IS NULL)                                                  AS total_images,
                COUNT(*) FILTER (WHERE description IS NOT NULL AND description != '' AND deleted_at IS NULL) AS images_with_description,
                COUNT(*) FILTER (WHERE embedding_generated_at IS NOT NULL AND deleted_at IS NULL)           AS images_with_embedding,
                COUNT(*) FILTER (WHERE verification_status = 1 AND deleted_at IS NULL)                     AS verified_images,
                COUNT(*) FILTER (WHERE face_detection_completed_at IS NULL AND deleted_at IS NULL)          AS images_face_pending,
                COUNT(*) FILTER (WHERE p2p_synced_at IS NOT NULL AND deleted_at IS NULL)                   AS p2p_synced,
                COUNT(*) FILTER (WHERE has_thumbnail = true AND deleted_at IS NULL)                         AS with_thumbnail
            FROM images
        ),
        vid AS (
            SELECT
                COUNT(*) FILTER (WHERE deleted_at IS NULL)                                                  AS total_videos,
                COUNT(*) FILTER (WHERE verification_status = 1 AND deleted_at IS NULL)                     AS verified_videos,
                COUNT(*) FILTER (WHERE p2p_synced_at IS NOT NULL AND deleted_at IS NULL)                   AS p2p_synced,
                COUNT(*) FILTER (WHERE has_thumbnail = true AND deleted_at IS NULL)                         AS with_thumbnail
            FROM videos
        ),
        face_stats AS (
            SELECT
                COUNT(*)                                            AS total_faces,
                COUNT(DISTINCT (f.image_user_id, f.image_hash))    AS images_with_faces
            FROM faces f
            JOIN images i ON i.user_id = f.image_user_id AND i.hash = f.image_hash
            WHERE i.deleted_at IS NULL
        )
        SELECT
            img.total_images,
            vid.total_videos,
            (SELECT COUNT(*) FROM users),
            img.images_with_description,
            (SELECT COUNT(DISTINCT si.hash) FROM starred_images si
                JOIN images i ON i.hash = si.hash WHERE i.deleted_at IS NULL),
            (SELECT COUNT(DISTINCT sv.hash) FROM starred_videos sv
                JOIN videos v ON v.hash = sv.hash WHERE v.deleted_at IS NULL),
            img.images_with_embedding,
            img.verified_images,
            vid.verified_videos,
            face_stats.total_faces,
            (SELECT COUNT(*) FROM persons),
            face_stats.images_with_faces,
            img.images_face_pending,
            img.p2p_synced,
            vid.p2p_synced,
            img.with_thumbnail + vid.with_thumbnail
        FROM img, vid, face_stats
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
