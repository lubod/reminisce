use actix_web::{get, web, HttpRequest, HttpResponse};
use log::{error, info};
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;
use pgvector::Vector;

use crate::config::Config;
use crate::utils;
use crate::db::MainDbPool;

#[derive(Deserialize)]
pub struct SearchQuery {
    pub query: String,
    #[serde(default = "default_limit")]
    pub limit: usize,
    #[serde(default = "default_offset")]
    pub offset: usize,
    pub device_id: Option<String>,
    pub starred_only: Option<bool>,
    #[serde(default = "default_min_similarity")]
    pub min_similarity: f32,
    #[serde(default = "default_search_mode")]
    pub mode: String,  // "semantic", "text", or "hybrid"
    // Location filtering parameters
    pub location_lat: Option<f64>,
    pub location_lon: Option<f64>,
    #[serde(default = "default_location_radius_km")]
    pub location_radius_km: Option<f64>,
    // Date filtering parameters
    pub start_date: Option<String>,
    pub end_date: Option<String>,
}

fn default_location_radius_km() -> Option<f64> { Some(10.0) }

fn default_limit() -> usize { 20 }
fn default_offset() -> usize { 0 }
fn default_min_similarity() -> f32 { 0.08 }  // Lowered to 8% for SigLIP which produces lower cosine similarity scores than CLIP
fn default_search_mode() -> String { "semantic".to_string() }

#[derive(Serialize, ToSchema)]
pub struct SearchResult {
    pub hash: String,
    pub name: String,
    pub description: Option<String>,
    pub place: Option<String>,
    pub created_at: String,
    pub similarity: f32,
    pub starred: bool,
    pub device_id: String,
    pub distance_km: Option<f32>,  // Distance from search location in kilometers
    pub thumbnail_url: Option<String>,
    pub media_type: String,
}

#[derive(Serialize)]
pub struct SearchResponse {
    pub results: Vec<SearchResult>,
    pub total: usize,
    pub query: String,
    pub min_similarity: f32,
    pub search_mode: String,
}

/// Search images by semantic similarity or full-text search
/// Supports three modes:
/// - semantic: Uses CLIP embeddings for AI-powered semantic search (default)
/// - text: Uses PostgreSQL full-text search on descriptions (faster, keyword-based)
/// - hybrid: Combines both approaches for best results
#[utoipa::path(
    get,
    path = "/api/search/images",
    params(
        ("query" = String, Query, description = "Search query text"),
        ("limit" = Option<usize>, Query, description = "Number of results (default: 20)"),
        ("offset" = Option<usize>, Query, description = "Offset for pagination (default: 0)"),
        ("device_id" = Option<String>, Query, description = "Filter by device"),
        ("starred_only" = Option<bool>, Query, description = "Show only starred images"),
        ("min_similarity" = Option<f32>, Query, description = "Minimum similarity threshold 0.0-1.0 (default: 0.35)"),
        ("mode" = Option<String>, Query, description = "Search mode: 'semantic' (default), 'text', or 'hybrid'"),
        ("location_lat" = Option<f64>, Query, description = "Latitude for location filtering"),
        ("location_lon" = Option<f64>, Query, description = "Longitude for location filtering"),
        ("location_radius_km" = Option<f64>, Query, description = "Search radius in kilometers (default: 10)")
    ),
    responses(
        (status = 200, description = "Search results", body = SearchResponse),
        (status = 401, description = "Unauthorized"),
        (status = 500, description = "Internal server error")
    ),
    tag = "Search"
)]
#[get("/search/images")]
pub async fn search_images(
    req: HttpRequest,
    query: web::Query<SearchQuery>,
    pool: web::Data<MainDbPool>,
    config: web::Data<Config>,
) -> Result<HttpResponse, actix_web::Error> {
    let claims = match utils::authenticate_request(&req, "search_images", config.get_api_key()).await {
        Ok(claims) => claims,
        Err(response) => return Ok(response),
    };

    info!("Searching images with query: '{}' for user: {} (mode: {})", query.query, claims.user_id, query.mode);

    // Validate query length
    if query.query.trim().is_empty() {
        return Ok(HttpResponse::BadRequest().json(serde_json::json!({
            "error": "Query cannot be empty"
        })));
    }

    if query.query.len() > 500 {
        return Ok(HttpResponse::BadRequest().json(serde_json::json!({
            "error": "Query too long (max 500 characters)"
        })));
    }

    let user_uuid = utils::parse_user_uuid(&claims.user_id)?;

    let device_filter: Option<&String> = query.device_id.as_ref();

    let limit_i64 = query.limit as i64;
    let offset_i64 = query.offset as i64;

    // Route to appropriate search based on mode
    let results = match query.mode.as_str() {
        "text" => {
            // Full-text search only
            info!("Using text-based search");
            crate::services::text_search::search_by_text(
                &query.query,
                &user_uuid,
                device_filter,
                query.starred_only.unwrap_or(false),
                limit_i64,
                offset_i64,
                query.location_lat,
                query.location_lon,
                query.location_radius_km,
                query.start_date.as_ref(),
                query.end_date.as_ref(),
                &pool,
            ).await?
        },
        "hybrid" => {
            // Hybrid search (semantic + text)
            info!("Using hybrid search (semantic + text)");

            // Get embedding for semantic component
            let embedding = match get_text_embedding(&query.query, &config).await {
                Ok(emb) => {
                    info!("Generated embedding for hybrid search (dimension: {})", emb.as_slice().len());
                    emb
                },
                Err(e) => {
                    error!("Failed to get text embedding for hybrid search: {}", e);
                    return Ok(HttpResponse::InternalServerError().json(serde_json::json!({
                        "error": "Failed to generate embedding for search query"
                    })));
                }
            };

            crate::services::text_search::search_hybrid(
                &query.query,
                &embedding,
                &user_uuid,
                device_filter,
                query.starred_only.unwrap_or(false),
                query.min_similarity,
                limit_i64,
                offset_i64,
                query.location_lat,
                query.location_lon,
                query.location_radius_km,
                query.start_date.as_ref(),
                query.end_date.as_ref(),
                &pool,
            ).await?
        },
        _ => {
            // Default: semantic search only
            info!("Using semantic search (embedding-based)");

            // Get text embedding from CLIP service
            let embedding = match get_text_embedding(&query.query, &config).await {
                Ok(emb) => {
                    info!("Generated embedding for query '{}' (dimension: {})", query.query, emb.as_slice().len());
                    info!("Embedding sample (first 5 values): {:?}", &emb.as_slice()[..5.min(emb.as_slice().len())]);
                    emb
                },
                Err(e) => {
                    error!("Failed to get text embedding: {}", e);
                    return Ok(HttpResponse::InternalServerError().json(serde_json::json!({
                        "error": "Failed to generate embedding for search query"
                    })));
                }
            };

            perform_semantic_search(
                &embedding,
                &user_uuid,
                device_filter,
                query.starred_only.unwrap_or(false),
                query.min_similarity,
                limit_i64,
                offset_i64,
                query.location_lat,
                query.location_lon,
                query.location_radius_km,
                query.start_date.as_ref(),
                query.end_date.as_ref(),
                &pool,
            ).await?
        }
    };

    let total = results.len();

    info!("Search completed: found {} results for query: '{}' (mode: {})", total, query.query, query.mode);

    Ok(HttpResponse::Ok().json(SearchResponse {
        results,
        total,
        query: query.query.clone(),
        min_similarity: query.min_similarity,
        search_mode: query.mode.clone(),
    }))
}

#[derive(Deserialize)]
pub struct VideoKeyframeQuery {
    pub query: String,
    #[serde(default = "default_limit")]
    pub limit: usize,
    #[serde(default = "default_offset")]
    pub offset: usize,
    #[serde(default = "default_min_similarity")]
    pub min_similarity: f32,
    /// Optional: restrict the search to a single video by its hash
    pub video_hash: Option<String>,
}

#[derive(Serialize, ToSchema)]
pub struct VideoKeyframeResult {
    pub video_hash: String,
    pub timestamp_secs: f32,
    pub similarity: f32,
    pub name: String,
    pub created_at: String,
    pub thumbnail_url: Option<String>,
}

#[derive(Serialize)]
pub struct VideoKeyframeSearchResponse {
    pub results: Vec<VideoKeyframeResult>,
    pub total: usize,
    pub query: String,
}

/// Timestamp-accurate video search: finds the specific moments (keyframes)
/// within videos that match a natural-language query, using per-keyframe
/// SigLIP2 embeddings stored in `video_keyframes`.
#[utoipa::path(
    get,
    path = "/api/search/video-keyframes",
    params(
        ("query" = String, Query, description = "Search query text"),
        ("limit" = Option<usize>, Query, description = "Number of results (default: 20)"),
        ("offset" = Option<usize>, Query, description = "Offset for pagination (default: 0)"),
        ("min_similarity" = Option<f32>, Query, description = "Minimum similarity threshold 0.0-1.0 (default: 0.08)"),
        ("video_hash" = Option<String>, Query, description = "Restrict search to a single video")
    ),
    responses(
        (status = 200, description = "Matching video keyframes", body = VideoKeyframeSearchResponse),
        (status = 401, description = "Unauthorized"),
        (status = 500, description = "Internal server error")
    ),
    tag = "Search"
)]
#[get("/search/video-keyframes")]
pub async fn search_video_keyframes(
    req: HttpRequest,
    query: web::Query<VideoKeyframeQuery>,
    pool: web::Data<MainDbPool>,
    config: web::Data<Config>,
) -> Result<HttpResponse, actix_web::Error> {
    let claims = match utils::authenticate_request(&req, "search_video_keyframes", config.get_api_key()).await {
        Ok(claims) => claims,
        Err(response) => return Ok(response),
    };

    if query.query.trim().is_empty() {
        return Ok(HttpResponse::BadRequest().json(serde_json::json!({
            "error": "Query cannot be empty"
        })));
    }
    if query.query.len() > 500 {
        return Ok(HttpResponse::BadRequest().json(serde_json::json!({
            "error": "Query too long (max 500 characters)"
        })));
    }

    let user_uuid = utils::parse_user_uuid(&claims.user_id)?;

    let embedding = match get_text_embedding(&query.query, &config).await {
        Ok(emb) => emb,
        Err(e) => {
            error!("Failed to get text embedding for video keyframe search: {}", e);
            return Ok(HttpResponse::InternalServerError().json(serde_json::json!({
                "error": "Failed to generate embedding for search query"
            })));
        }
    };

    let results = perform_keyframe_search(
        &embedding,
        &user_uuid,
        query.video_hash.as_ref(),
        query.min_similarity,
        query.limit as i64,
        query.offset as i64,
        &pool,
    ).await?;

    let total = results.len();
    info!("Video keyframe search for '{}' returned {} results (user {})", query.query, total, claims.user_id);

    Ok(HttpResponse::Ok().json(VideoKeyframeSearchResponse {
        results,
        total,
        query: query.query.clone(),
    }))
}

/// KNN search over per-keyframe video embeddings. Uses the HNSW index on
/// `video_keyframes.embedding` (ORDER BY embedding <=> $2 LIMIT) and joins to
/// `videos` to exclude soft-deleted media and fetch display metadata.
async fn perform_keyframe_search(
    embedding: &Vector,
    user_uuid: &uuid::Uuid,
    video_hash: Option<&String>,
    min_similarity: f32,
    limit: i64,
    offset: i64,
    pool: &web::Data<MainDbPool>,
) -> Result<Vec<VideoKeyframeResult>, actix_web::Error> {
    let client = utils::get_db_client(&pool.0).await?;

    // ef_search must be >= limit + offset for the HNSW KNN scan.
    let ef_search = std::cmp::max((limit + offset) * 2, 100);
    client.execute(&format!("SET hnsw.ef_search = {}", ef_search), &[]).await
        .unwrap_or(0);

    // $1=user_id, $2=embedding, $3=min_similarity, $4=limit, $5=offset, $6=video_hash (optional)
    let mut sql = String::from(
        "SELECT k.video_hash, k.timestamp_secs,
                1 - (k.embedding <=> $2) as similarity,
                v.name, v.created_at
         FROM video_keyframes k
         JOIN videos v ON v.hash = k.video_hash AND v.user_id = k.user_id
         WHERE k.user_id = $1
           AND v.deleted_at IS NULL
           AND (1 - (k.embedding <=> $2)) > $3"
    );
    if video_hash.is_some() {
        sql.push_str(" AND k.video_hash = $6");
    }
    sql.push_str(" ORDER BY k.embedding <=> $2 LIMIT $4 OFFSET $5");

    let mut params: Vec<&(dyn tokio_postgres::types::ToSql + Sync)> =
        vec![user_uuid, embedding, &min_similarity, &limit, &offset];
    if let Some(vh) = video_hash {
        params.push(vh);
    }

    let rows = client.query(&sql, &params).await.map_err(|e| {
        error!("Failed to execute video keyframe search query: {}", e);
        actix_web::error::ErrorInternalServerError("Video keyframe search query failed")
    })?;

    let results = rows
        .iter()
        .map(|row| {
            let vhash: String = row.get(0);
            VideoKeyframeResult {
                video_hash: vhash.clone(),
                timestamp_secs: row.get(1),
                similarity: row.get::<_, f64>(2) as f32,
                name: row.get(3),
                created_at: row.get::<_, chrono::DateTime<chrono::Utc>>(4).to_rfc3339(),
                thumbnail_url: Some(format!("/api/thumbnail/{}", vhash)),
            }
        })
        .collect();

    Ok(results)
}

/// Perform semantic search using embeddings.
/// Public so integration tests (tests/semantic_search_test.rs) can drive the SQL
/// directly with a supplied embedding, bypassing the gRPC text-embedding step.
#[allow(clippy::too_many_arguments)]
pub async fn perform_semantic_search(
    embedding: &Vector,
    user_uuid: &uuid::Uuid,
    device_filter: Option<&String>,
    starred_only: bool,
    min_similarity: f32,
    limit: i64,
    offset: i64,
    location_lat: Option<f64>,
    location_lon: Option<f64>,
    location_radius_km: Option<f64>,
    start_date: Option<&String>,
    end_date: Option<&String>,
    pool: &web::Data<MainDbPool>,
) -> Result<Vec<SearchResult>, actix_web::Error> {
    let client = utils::get_db_client(&pool.0).await?;

    // Build query with optional filters
    // Add similarity threshold to filter out irrelevant results
    info!("Using minimum similarity threshold: {:.2}", min_similarity);

    // Reserved params: $1=user_id, $2=embedding, $3=per_branch_limit, $4=final_limit, $5=offset
    let per_branch_limit = limit + offset;
    let mut param_count = 5;

    let device_param = if device_filter.is_some() {
        param_count += 1;
        Some(param_count)
    } else {
        None
    };

    let start_param = if start_date.is_some() {
        param_count += 1;
        Some(param_count)
    } else {
        None
    };

    let end_param = if end_date.is_some() {
        param_count += 1;
        Some(param_count)
    } else {
        None
    };

    // Add location filtering if coordinates are provided
    let has_location_filter = location_lat.is_some() && location_lon.is_some();
    let mut lon_param: Option<i64> = None;
    let mut lat_param: Option<i64> = None;
    let radius_meters = if has_location_filter {
        let radius_km = location_radius_km.unwrap_or(10.0);
        param_count += 1;
        lon_param = Some(param_count);
        param_count += 1;
        lat_param = Some(param_count);
        info!("Location filter: lat={:.4}, lon={:.4}, radius={:.1}km",
              location_lat.unwrap(), location_lon.unwrap(), radius_km);
        radius_km * 1000.0
    } else {
        0.0
    };

    // Build a WHERE clause for a given table alias. Param placeholders are
    // shared across both branches (same bound params), only the alias differs.
    let build_where = |alias: &str| -> String {
        let mut conds: Vec<String> = vec![
            format!("{}.embedding IS NOT NULL", alias),
            format!("{}.user_id = $1", alias),
            format!("{}.deleted_at IS NULL", alias),
            format!("(1 - ({}.embedding <=> $2)) > {}", alias, min_similarity),
        ];
        if let Some(p) = device_param {
            conds.push(format!("{}.deviceid = ${}", alias, p));
        }
        if starred_only {
            conds.push("s.hash IS NOT NULL".to_string());
        }
        if let Some(p) = start_param {
            conds.push(format!("{}.created_at >= ${}", alias, p));
        }
        if let Some(p) = end_param {
            conds.push(format!("{}.created_at < ${}", alias, p));
        }
        if let (Some(lon), Some(lat)) = (lon_param, lat_param) {
            conds.push(format!("{}.location IS NOT NULL", alias));
            conds.push(format!(
                "ST_DWithin({}.location, ST_MakePoint(${}, ${})::geography, {})",
                alias, lon, lat, radius_meters
            ));
        }
        conds.join(" AND ")
    };

    let build_distance = |alias: &str| -> String {
        if let (Some(lon), Some(lat)) = (lon_param, lat_param) {
            format!(
                "ST_Distance({}.location, ST_MakePoint(${}, ${})::geography) / 1000.0",
                alias, lon, lat
            )
        } else {
            "NULL::double precision".to_string()
        }
    };

    let img_where = build_where("i");
    let vid_where = build_where("v");
    let img_distance = build_distance("i");
    let vid_distance = build_distance("v");

    info!("Image WHERE clause: {}", img_where);
    info!("Video WHERE clause: {}", vid_where);

    // Each branch performs its own KNN (ORDER BY embedding <=> $2 LIMIT $3) so
    // pgvector's HNSW index is used. The outer query then merges and applies
    // the final LIMIT/OFFSET. Wrapping each arm in parentheses guarantees the
    // per-arm ORDER BY/LIMIT is respected by the planner.
    let sql = format!(
        "SELECT hash, name, description, place, created_at, similarity, starred, deviceid, distance_km, media_type FROM (
            (
                SELECT i.hash, i.name, i.description, i.place, i.created_at,
                       1 - (i.embedding <=> $2) as similarity,
                       CASE WHEN s.hash IS NOT NULL THEN true ELSE false END as starred,
                       i.deviceid,
                       {} as distance_km,
                       'image' as media_type,
                       i.embedding <=> $2 as dist
                FROM images i
                LEFT JOIN starred_images s ON i.hash = s.hash AND s.user_id = $1
                WHERE {}
                ORDER BY i.embedding <=> $2
                LIMIT $3
            )
            UNION ALL
            (
                SELECT v.hash, v.name, v.description, NULL::text as place, v.created_at,
                       1 - (v.embedding <=> $2) as similarity,
                       CASE WHEN s.hash IS NOT NULL THEN true ELSE false END as starred,
                       v.deviceid,
                       {} as distance_km,
                       'video' as media_type,
                       v.embedding <=> $2 as dist
                FROM videos v
                LEFT JOIN starred_videos s ON v.hash = s.hash AND s.user_id = $1
                WHERE {}
                ORDER BY v.embedding <=> $2
                LIMIT $3
            )
        ) combined
        ORDER BY dist ASC
        LIMIT $4 OFFSET $5",
        img_distance,
        img_where,
        vid_distance,
        vid_where
    );

    // hnsw.ef_search controls how many candidates HNSW considers at query time.
    // Default is 40 — must be >= per_branch_limit so each arm returns enough rows.
    // Use per_branch_limit * 2 as a buffer, minimum 100.
    let ef_search = std::cmp::max(per_branch_limit * 2, 100);
    client.execute(&format!("SET hnsw.ef_search = {}", ef_search), &[]).await
        .unwrap_or(0);

    info!("Executing semantic search query across images & videos (ef_search={})", ef_search);
    info!("Query params: user_id={}, limit={}, offset={}, device_filter={:?}",
          user_uuid, limit, offset, device_filter);

    // Parse date strings
    use chrono::{NaiveDate, DateTime, Utc, TimeZone};
    let start_datetime: Option<DateTime<Utc>> = start_date.and_then(|d| {
        NaiveDate::parse_from_str(d, "%Y-%m-%d")
            .ok()
            .and_then(|nd| nd.and_hms_opt(0, 0, 0))
            .map(|ndt| Utc.from_utc_datetime(&ndt))
    });
    let end_datetime: Option<DateTime<Utc>> = end_date.and_then(|d| {
        NaiveDate::parse_from_str(d, "%Y-%m-%d")
            .ok()
            .and_then(|nd| nd.and_hms_opt(23, 59, 59))
            .and_then(|ndt| ndt.checked_add_signed(chrono::Duration::seconds(1)))
            .map(|ndt| Utc.from_utc_datetime(&ndt))
    });

    let mut params: Vec<&(dyn tokio_postgres::types::ToSql + Sync)> = vec![user_uuid, embedding, &per_branch_limit, &limit, &offset];

    if let Some(device) = device_filter {
        params.push(device);
    }

    if let Some(ref sd) = start_datetime {
        params.push(sd);
    }
    if let Some(ref ed) = end_datetime {
        params.push(ed);
    }

    // Variables to hold location values
    let lat_val;
    let lon_val;

    if has_location_filter {
        lat_val = location_lat.unwrap();
        lon_val = location_lon.unwrap();
        params.push(&lon_val);
        params.push(&lat_val);
    }

    let rows = client.query(&sql, &params).await.map_err(|e| {
        error!("Failed to execute semantic search query: {}", e);
        actix_web::error::ErrorInternalServerError("Semantic search query failed")
    })?;

    let results: Vec<SearchResult> = rows
        .iter()
        .map(|row| {
            let hash: String = row.get(0);
            let similarity = row.get::<_, f64>(5) as f32;
            let distance_km: Option<f64> = row.get(8);
            let media_type: String = row.try_get(9).unwrap_or_else(|_| "image".to_string());
            SearchResult {
                hash: hash.clone(),
                name: row.get(1),
                description: row.get(2),
                place: row.get(3),
                created_at: row.get::<_, chrono::DateTime<chrono::Utc>>(4).to_rfc3339(),
                similarity,
                starred: row.get(6),
                device_id: row.get(7),
                distance_km: distance_km.map(|d| d as f32),
                thumbnail_url: Some(format!("/api/thumbnail/{}", hash)),
                media_type,
            }
        })
        .collect();

    info!("Found {} semantic search results", results.len());

    // Log top 5 results with similarity scores
    for (i, result) in results.iter().take(5).enumerate() {
        info!("  Result {}: {} - similarity: {:.4}, name: {}",
              i + 1, &result.hash[..16], result.similarity, result.name);
    }

    if results.is_empty() {
        info!("No results found with similarity > {:.2}", min_similarity);
    }

    Ok(results)
}

/// Get text embedding from AI gRPC service
async fn get_text_embedding(text: &str, config: &Config) -> Result<Vector, String> {
    info!("Requesting text embedding via gRPC at: {}", config.ai_grpc_url);

    let client = crate::ai_client::AiClient::shared(config);
    let embedding_vec = client.embed_text(text.to_string()).await?;

    if embedding_vec.len() != 1152 {
        return Err(format!("Invalid embedding dimension: expected 1152, got {}", embedding_vec.len()));
    }

    info!("Successfully received text embedding via gRPC (dimension: {})", embedding_vec.len());
    Ok(Vector::from(embedding_vec))
}

/// Get image embedding from AI gRPC service
pub async fn get_image_embedding(image_data: &[u8], config: &Config) -> Result<Vector, String> {
    let client = crate::ai_client::AiClient::shared(config);
    let embedding_vec = client.embed_image(image_data.to_vec()).await?;

    if embedding_vec.len() != 1152 {
        return Err(format!("Invalid embedding dimension: expected 1152, got {}", embedding_vec.len()));
    }

    Ok(Vector::from(embedding_vec))
}
