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
    let claims = match utils::authenticate_request(&req, "search_images", config.get_api_key()) {
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

    let device_filter: Option<&String> = if claims.role == "admin" {
        query.device_id.as_ref()
    } else {
        None // Access control for non-admin is handled via user_id in the queries
    };

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

/// Perform semantic search using embeddings
async fn perform_semantic_search(
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

    let similarity_threshold = format!("(1 - (i.embedding <=> $2)) > {}", min_similarity);
    let mut conditions = vec![
        "i.embedding IS NOT NULL",
        "i.user_id = $1",
        "i.deleted_at IS NULL",
    ];
    let mut owned_conditions: Vec<String> = vec![similarity_threshold];
    let mut param_count = 4; // $1=user_id, $2=embedding, $3=limit, $4=offset

    if let Some(_) = device_filter {
        param_count += 1;
        owned_conditions.push(format!("i.deviceid = ${}", param_count));
    }

    if starred_only {
        conditions.push("s.hash IS NOT NULL");
    }

    // Add date filters
    if start_date.is_some() {
        param_count += 1;
        owned_conditions.push(format!("i.created_at >= ${}", param_count));
    }
    if end_date.is_some() {
        param_count += 1;
        owned_conditions.push(format!("i.created_at < ${}", param_count));
    }

    // Add location filtering if coordinates are provided
    let has_location_filter = location_lat.is_some() && location_lon.is_some();
    if has_location_filter {
        conditions.push("i.location IS NOT NULL");
        let radius_km = location_radius_km.unwrap_or(10.0);
        let radius_meters = radius_km * 1000.0;
        param_count += 1;
        let lon_param = param_count;
        param_count += 1;
        let lat_param = param_count;
        owned_conditions.push(format!(
            "ST_DWithin(i.location, ST_MakePoint(${}, ${})::geography, {})",
            lon_param, lat_param, radius_meters
        ));
        info!("Location filter: lat={:.4}, lon={:.4}, radius={:.1}km",
              location_lat.unwrap(), location_lon.unwrap(), radius_km);
    }

    // Combine both static and owned conditions
    let all_conditions: Vec<&str> = conditions.iter().map(|s| *s)
        .chain(owned_conditions.iter().map(|s| s.as_str()))
        .collect();

    let where_clause = all_conditions.join(" AND ");
    info!("WHERE clause conditions: {:?}", all_conditions);
    info!("Final WHERE clause: {}", where_clause);

    // Vector similarity search using cosine distance
    // The <=> operator calculates cosine distance (0 = identical, 2 = opposite)
    // similarity = 1 - distance gives us a 0-1 score
    let distance_calc = if has_location_filter {
        // Use parameter placeholders that will be filled in at query time
        // Note: We'll need to pass lon, lat in the correct order to match the params
        format!(
            "ST_Distance(i.location, ST_MakePoint(${}, ${})::geography) / 1000.0",
            param_count - 1, param_count  // lon_param, lat_param
        )
    } else {
        "NULL::double precision".to_string()
    };

    let sql = format!(
        "SELECT i.hash, i.name, i.description, i.place, i.created_at,
                1 - (i.embedding <=> $2) as similarity,
                CASE WHEN s.hash IS NOT NULL THEN true ELSE false END as starred,
                i.deviceid,
                {} as distance_km
         FROM images i
         LEFT JOIN starred_images s ON i.hash = s.hash AND s.user_id = $1
         WHERE {}
         ORDER BY i.embedding <=> $2
         LIMIT $3 OFFSET $4",
        distance_calc,
        where_clause
    );

    // hnsw.ef_search controls how many candidates HNSW considers at query time.
    // Default is 40 — must be >= (limit + offset) so OFFSET pagination works correctly.
    // Use (limit + offset) * 2 as a buffer, minimum 100.
    let ef_search = std::cmp::max((limit + offset) * 2, 100);
    client.execute(&format!("SET hnsw.ef_search = {}", ef_search), &[]).await
        .unwrap_or(0);

    info!("Executing semantic search query (ef_search={})", ef_search);
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

    let mut params: Vec<&(dyn tokio_postgres::types::ToSql + Sync)> = vec![user_uuid, embedding, &limit, &offset];

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
                media_type: "image".to_string(),
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

/// Get text embedding from CLIP service
async fn get_text_embedding(text: &str, config: &Config) -> Result<Vector, String> {
    use log::info;

    info!("Requesting text embedding from CLIP service at: {}", config.embedding_service_url);

    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(30))
        .build()
        .map_err(|e| format!("Failed to create HTTP client: {}", e))?;

    let url = format!("{}/embed/text", config.embedding_service_url);

    let response = client
        .post(&url)
        .json(&serde_json::json!({"text": text}))
        .send()
        .await
        .map_err(|e| format!("Failed to send request to CLIP service: {}", e))?;

    if !response.status().is_success() {
        return Err(format!("CLIP service returned error: {}", response.status()));
    }

    let data: serde_json::Value = response.json().await
        .map_err(|e| format!("Failed to parse response: {}", e))?;

    let embedding_vec: Vec<f32> = data["embedding"]
        .as_array()
        .ok_or("No embedding in response")?
        .iter()
        .map(|v| v.as_f64().unwrap_or(0.0) as f32)
        .collect();

    if embedding_vec.len() != 1152 {
        return Err(format!("Invalid embedding dimension: expected 1152, got {}", embedding_vec.len()));
    }

    info!("Successfully received embedding from CLIP service (dimension: {})", embedding_vec.len());

    Ok(Vector::from(embedding_vec))
}

/// Get image embedding from CLIP service
pub async fn get_image_embedding(image_data: &[u8], config: &Config) -> Result<Vector, String> {
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(60))
        .build()
        .map_err(|e| format!("Failed to create HTTP client: {}", e))?;

    let base64_image = base64::Engine::encode(&base64::engine::general_purpose::STANDARD, image_data);
    let url = format!("{}/embed/image", config.embedding_service_url);

    let response = client
        .post(&url)
        .json(&serde_json::json!({"image": base64_image}))
        .send()
        .await
        .map_err(|e| format!("Failed to send request to CLIP service: {}", e))?;

    if !response.status().is_success() {
        return Err(format!("CLIP service returned error: {}", response.status()));
    }

    let data: serde_json::Value = response.json().await
        .map_err(|e| format!("Failed to parse response: {}", e))?;

    let embedding_vec: Vec<f32> = data["embedding"]
        .as_array()
        .ok_or("No embedding in response")?
        .iter()
        .map(|v| v.as_f64().unwrap_or(0.0) as f32)
        .collect();

    if embedding_vec.len() != 1152 {
        return Err(format!("Invalid embedding dimension: expected 1152, got {}", embedding_vec.len()));
    }

    Ok(Vector::from(embedding_vec))
}
