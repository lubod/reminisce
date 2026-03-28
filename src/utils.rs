use actix_web::{ web, HttpRequest, HttpResponse };
use chrono::{ DateTime, NaiveDate, NaiveDateTime, Utc };

use jsonwebtoken::{ Algorithm, DecodingKey, Validation };
use log::{ error, info, warn };
use sysinfo::{ System, SystemExt };
use std::path::PathBuf;
use tokio::fs;

use crate::constants::media;
use crate::query_builder::MediaQueryBuilder;
use crate::services::{ auth::Claims, thumbnail::ThumbnailItem };
use crate::db::{GeotaggingDbPool, MainDbPool};

/// Get a DB client from the pool, returning 500 on failure.
pub async fn get_db_client(pool: &deadpool_postgres::Pool) -> Result<deadpool_postgres::Client, actix_web::Error> {
    pool.get().await.map_err(|e| {
        error!("Failed to get DB client: {}", e);
        actix_web::error::ErrorInternalServerError("Database connection failed")
    })
}

/// Parse a user_id string into a UUID, returning 400 on failure.
pub fn parse_user_uuid(user_id: &str) -> Result<uuid::Uuid, actix_web::Error> {
    uuid::Uuid::parse_str(user_id).map_err(|e| {
        error!("Failed to parse user_id as UUID: {}", e);
        actix_web::error::ErrorBadRequest("Invalid user ID")
    })
}

#[derive(serde::Serialize)]
pub struct ExistenceCheckResult {
    pub exists_for_deviceid: bool,
    pub exists_without_deviceid: bool,
}

/// Generates a subdirectory path from the first 2 characters of a hash
pub fn get_subdirectory_path(base_dir: &str, hash: &str) -> PathBuf {
    if hash.len() < 2 {
        return PathBuf::from(base_dir);
    }

    let sub_dir = &hash[..2];
    PathBuf::from(base_dir).join(sub_dir)
}

pub async fn get_load_average() -> f64 {
    let sys = System::new_all();
    sys.load_average().one
}

pub async fn get_gpu_load() -> u32 {
    match tokio::fs::read_to_string("/sys/class/drm/card0/device/gpu_busy_percent").await {
        Ok(s) => s.trim().parse().unwrap_or(0),
        Err(_) => 0,
    }
}

pub fn get_cpu_count() -> usize {
    let sys = System::new_all();
    sys.cpus().len()
}

pub fn adjust_batch_size(load_average: f64) -> i64 {
    if load_average > 3.0 {
        0
    } else if load_average > 2.0 {
        1
    } else if load_average > 1.0 {
        2
    } else {
        3
    }
}

/// Weighted concurrency limits for all worker types
/// Priority order: verification > thumbnail > embedding > face_detection > description
#[derive(Debug, Clone, Copy)]
pub struct WorkerConcurrencyLimits {
    pub verification: usize,    // I/O bound (BLAKE3 hashing)
    pub embedding: usize,       // High priority AI task
    pub face_detection: usize,  // Medium priority AI task
    pub description: usize,     // Lowest priority AI task (slow, 5min timeout)
    pub gpu_overloaded: bool,   // Whether GPU is currently at capacity
}

impl WorkerConcurrencyLimits {
    pub fn is_overloaded(&self) -> bool {
        self.verification == 0
    }
}

/// Calculate weighted concurrency limits based on system load, GPU load and CPU count
/// Priority: verification > embedding > face_detection > description
/// Uses ~70% of CPU capacity to leave headroom for system operations
pub fn calculate_worker_concurrency(load_average: f64, gpu_load: u32, cpu_count: usize) -> WorkerConcurrencyLimits {
    let normalized_load = load_average / (cpu_count as f64).max(1.0);
    let gpu_overloaded = gpu_load > 90;

    // Skip all processing if load is extremely high (> 120% usage)
    if normalized_load > 1.2 {
        info!("System overloaded ({:.0}% normalized), pausing all workers", normalized_load * 100.0);
        return WorkerConcurrencyLimits {
            verification: 0,
            embedding: 0,
            face_detection: 0,
            description: 0,
            gpu_overloaded,
        };
    }

    // Use 70% of CPU cores as base (better utilization than 50%)
    let base = ((cpu_count as f64) * 0.7).max(1.0);

    // Adjust based on current load
    let load_multiplier = if normalized_load > 0.9 {
        0.5   // High load (>90%): 50% capacity
    } else if normalized_load > 0.7 {
        0.75  // Medium load (>70%): 75% capacity
    } else if normalized_load > 0.5 {
        0.9   // Light load (>50%): 90% capacity
    } else {
        1.0   // Low load: full capacity
    };

    // Throttle AI tasks specifically if GPU is busy
    let ai_multiplier = if gpu_load > 80 {
        0.3
    } else if gpu_load > 50 {
        0.6
    } else {
        1.0
    };

    let available = (base * load_multiplier).max(1.0);

    // Weighted distribution based on priority and resource intensity:
    // - Verification: I/O bound, can run more (weight: 1.5x)
    // - Embedding: GPU/CPU moderate, high priority (weight: 1.0x)
    // - Face detection: GPU/CPU moderate (weight: 0.75x)
    // - Description: CPU heavy, slow, lowest priority (weight: 0.25x)

    let verification = (available * 1.5).ceil() as usize;
    let embedding = (available * ai_multiplier).ceil() as usize;
    let face_detection = (available * 0.75 * ai_multiplier).ceil() as usize;
    let description = (available * 0.25 * ai_multiplier).ceil().max(1.0) as usize;

    // Apply reasonable caps
    WorkerConcurrencyLimits {
        verification: verification.min(16).max(2),      // 2-16 concurrent
        embedding: embedding.min(10).max(1),            // 1-10 concurrent
        face_detection: face_detection.min(8).max(1),   // 1-8 concurrent
        description: description.min(4).max(1),         // 1-4 concurrent
        gpu_overloaded,
    }
}

/// Calculate optimal batch size for parallel processing
/// Since we can process multiple items concurrently, we want to fetch larger batches
/// Returns the number of items to fetch from the database
pub fn calculate_parallel_batch_size(concurrency: usize, load_average: f64, cpu_count: usize) -> i64 {
    let normalized_load = load_average / (cpu_count as f64).max(1.0);

    // Stop if load is too high (> 150%)
    if normalized_load > 1.5 {
        return 0;  // Skip processing
    }

    // Fetch 3-5x the concurrency limit to keep workers busy
    // This ensures there's always work available when a task completes
    let multiplier = if normalized_load > 1.0 {
        2  // High load: fetch 2x concurrency (smaller batches)
    } else if normalized_load > 0.7 {
        3  // Medium load: fetch 3x concurrency
    } else {
        5  // Low load: fetch 5x concurrency
    };

    ((concurrency * multiplier) as i64).max(3).min(50)  // Minimum 3, maximum 50 items per batch
}

// These functions will be replaced by accessing config directly in the handlers

/// Authenticates a request by checking for a valid JWT token or a valid secret cookie.
/// Logs the request and returns an Unauthorized response if authentication fails.
/// Ensures a user from JWT claims exists in the local database.
/// If the user doesn't exist, auto-provisions them from the JWT claims.
/// This allows the relay to be the single source of truth for auth.
pub async fn ensure_user_exists(
    client: &tokio_postgres::Client,
    claims: &Claims,
) -> Result<(), actix_web::Error> {
    let user_uuid = uuid::Uuid::parse_str(&claims.user_id).map_err(|e| {
        error!("Failed to parse user_id as UUID: {}", e);
        actix_web::error::ErrorBadRequest("Invalid user ID")
    })?;

    let exists = client
        .query_opt("SELECT 1 FROM users WHERE id = $1", &[&user_uuid])
        .await
        .map_err(|e| {
            error!("Failed to check user existence: {}", e);
            actix_web::error::ErrorInternalServerError("Database error")
        })?
        .is_some();

    if !exists {
        let email = if claims.email.is_empty() {
            format!("{}@relay", claims.username)
        } else {
            claims.email.clone()
        };
        info!("Auto-provisioning user from relay JWT: id={}, username={}, email={}, role={}", claims.user_id, claims.username, email, claims.role);
        client
            .execute(
                "INSERT INTO users (id, username, email, password_hash, role) VALUES ($1, $2, $3, $4, $5) ON CONFLICT (id) DO NOTHING",
                &[&user_uuid, &claims.username, &email, &"relay-managed", &claims.role],
            )
            .await
            .map_err(|e| {
                error!("Failed to auto-provision user: {}", e);
                actix_web::error::ErrorInternalServerError("Failed to create user")
            })?;
    }

    Ok(())
}

pub fn authenticate_request(
    req: &HttpRequest,
    handler_name: &str,
    api_secret_env: &str
) -> Result<Claims, HttpResponse> {
    if let Some(peer_addr) = req.peer_addr() {
        info!("{} request from: {}", handler_name, peer_addr);
    }

    let mut token = None;

    // 1. Try Authorization header
    if let Some(auth_header) = req.headers().get("Authorization") {
        if let Ok(auth_str) = auth_header.to_str() {
            if auth_str.starts_with("Bearer ") {
                token = Some(auth_str.trim_start_matches("Bearer ").to_string());
            }
        }
    }

    // 2. Try 'token' query parameter (useful for <img> tags)
    if token.is_none() {
        if let Ok(query) = web::Query::<std::collections::HashMap<String, String>>::from_query(req.query_string()) {
            if let Some(t) = query.get("token") {
                token = Some(t.clone());
            }
        }
    }

    if let Some(token_str) = token {
        let validation = Validation::new(Algorithm::HS512);
        match jsonwebtoken::decode::<Claims>(
            &token_str,
            &DecodingKey::from_secret(api_secret_env.as_ref()),
            &validation
        ) {
            Ok(token_data) => {
                log::debug!("JWT token validated successfully for {}.", handler_name);
                return Ok(token_data.claims);
            }
            Err(e) => {
                warn!("JWT validation failed for {}: {:?}", handler_name, e);
            }
        }
    }

    warn!("Authentication failed for {}: No valid JWT token found.", handler_name);
    Err(HttpResponse::Unauthorized().json("Authentication required"))
}

pub fn determine_image_type(image_name: &str) -> String {
    let lower_name = image_name.to_lowercase();
    if lower_name.contains("dcim/camera") {
        media::TYPE_CAMERA.to_string()
    } else if lower_name.contains("whatsapp") {
        media::TYPE_WHATSAPP.to_string()
    } else if lower_name.contains("screenshot") {
        media::TYPE_SCREENSHOT.to_string()
    } else {
        media::TYPE_OTHER.to_string()
    }
}

pub fn determine_video_type(video_name: &str) -> String {
    let lower_name = video_name.to_lowercase();
    if lower_name.contains("dcim/camera") || lower_name.contains("dji") {
        media::TYPE_CAMERA.to_string()
    } else if lower_name.contains("whatsapp") {
        media::TYPE_WHATSAPP.to_string()
    } else if lower_name.contains("screen") {
        media::TYPE_SCREEN_RECORDING.to_string()
    } else {
        media::TYPE_OTHER.to_string()
    }
}

pub async fn check_if_exists(
    hash: &str,
    device_id: &str,
    table: &str,
    pool: web::Data<MainDbPool>
) -> Result<ExistenceCheckResult, tokio_postgres::Error> {
    let client = pool.0.get().await.expect("Failed to get database client");

    // Use match to safely select table name instead of format!
    let query_string = match table {
        "images" => "
            SELECT
                EXISTS(SELECT 1 FROM images WHERE deviceid = $1 AND hash = $2 AND deleted_at IS NULL) as exists_for_deviceid,
                EXISTS(SELECT 1 FROM images WHERE hash = $2 AND verification_status = 1 AND deleted_at IS NULL) as exists_without_deviceid
        ",
        "videos" => "
            SELECT
                EXISTS(SELECT 1 FROM videos WHERE deviceid = $1 AND hash = $2 AND deleted_at IS NULL) as exists_for_deviceid,
                EXISTS(SELECT 1 FROM videos WHERE hash = $2 AND verification_status = 1 AND deleted_at IS NULL) as exists_without_deviceid
        ",
        _ => {
            warn!("Invalid table name provided to check_if_exists: {}", table);
            return Err(tokio_postgres::Error::__private_api_timeout());
        }
    };

    // Execute both checks in a single query
    let row = client.query_one(query_string, &[&device_id, &hash]).await?;
    let exists_for_deviceid: bool = row.get(0);
    let exists_without_deviceid: bool = row.get(1);

    Ok(ExistenceCheckResult {
        exists_for_deviceid,
        exists_without_deviceid,
    })
}

pub async fn list_thumbnails(
    user_id: &str,
    device_id: Option<&str>,
    table: &str,
    media_type: &str,
    offset: usize,
    limit: usize,
    starred_only: bool,
    start_date: Option<&str>,
    end_date: Option<&str>,
    location_lat: Option<f64>,
    location_lon: Option<f64>,
    location_radius_km: Option<f64>,
    label_id: Option<i32>,
    apply_user_id_filter: bool,
    pool: &web::Data<MainDbPool>,
) -> Result<Vec<ThumbnailItem>, Box<dyn std::error::Error>> {
    let client = pool.0.get().await.map_err(|e| {
        use log::error;
        error!("Failed to get database client for list_thumbnails: {}", e);
        Box::new(e) as Box<dyn std::error::Error>
    })?;

    let user_uuid = uuid::Uuid::parse_str(user_id).map_err(|e| {
        use log::error;
        error!("Failed to parse user_id as UUID: {}", e);
        Box::new(e) as Box<dyn std::error::Error>
    })?;

    // Helper closure to apply common filters
    let apply_filters = |builder: &mut MediaQueryBuilder| {
        // Always add user_id for starred images JOIN
        builder.with_user_id();

        // For non-admin users, filter by user_id for access control
        if apply_user_id_filter {
            builder.with_user_id_filter();
        }

        if device_id.is_some() {
            builder.with_device_id();
        }

        if media_type != media::TYPE_ALL {
            builder.with_media_type();
        }

        if starred_only {
            builder.with_starred_only();
        }

        if label_id.is_some() {
            builder.with_label_id();
        }

        if start_date.is_some() {
            builder.with_start_date();
        }

        if end_date.is_some() {
            builder.with_end_date();
        }
    };

    let query_string;
    let has_location_filter = location_lat.is_some() && location_lon.is_some();
    let limit_param;
    let offset_param;
    // Store param indices for location binding
    let mut lon_param_idx = None;
    let mut lat_param_idx = None;

    if table == "all" {
        // Build query for images
        let mut img_builder = MediaQueryBuilder::new("images");
        apply_filters(&mut img_builder);
        
        // Add location filters to image builder if needed
        if has_location_filter {
            let radius_km = location_radius_km.unwrap_or(10.0);
            let radius_meters = radius_km * 1000.0;
            let lon_param = img_builder.param_count() + 1;
            let lat_param = img_builder.param_count() + 2;
            img_builder.add_custom_condition("t.location IS NOT NULL".to_string());
            img_builder.add_custom_condition(format!(
                "ST_DWithin(t.location, ST_MakePoint(${}, ${})::geography, {})",
                lon_param, lat_param, radius_meters
            ));
            lon_param_idx = Some(lon_param);
            lat_param_idx = Some(lat_param);
        }

        // Build query for videos
        let mut vid_builder = MediaQueryBuilder::new("videos");
        apply_filters(&mut vid_builder);

        // Videos don't have location data, so if filtering by location, exclude them
        if has_location_filter {
            vid_builder.add_custom_condition("1 = 0".to_string());
        }

        // Get max param count to determine limit/offset params
        let max_param = img_builder.param_count() + (if has_location_filter { 2 } else { 0 });
        limit_param = max_param + 1;
        offset_param = max_param + 2;

        let img_body = img_builder.build_select_body(lon_param_idx, lat_param_idx);
        let vid_body = vid_builder.build_select_body(None, None); // Videos don't support location sorting yet

        query_string = format!(
            "{} UNION ALL {} ORDER BY created_at DESC, hash DESC LIMIT ${} OFFSET ${}",
            img_body, vid_body, limit_param, offset_param
        );
    } else {
        // Single table query
        let mut builder = MediaQueryBuilder::new(table);
        apply_filters(&mut builder);

        if has_location_filter && table == "images" {
            let radius_km = location_radius_km.unwrap_or(10.0);
            let radius_meters = radius_km * 1000.0;
            let lon_param = builder.param_count() + 1;
            let lat_param = builder.param_count() + 2;
            builder.add_custom_condition("t.location IS NOT NULL".to_string());
            builder.add_custom_condition(format!(
                "ST_DWithin(t.location, ST_MakePoint(${}, ${})::geography, {})",
                lon_param, lat_param, radius_meters
            ));
            lon_param_idx = Some(lon_param);
            lat_param_idx = Some(lat_param);
        }

        limit_param = builder.param_count() + 1 + (if has_location_filter && table == "images" { 2 } else { 0 });
        offset_param = builder.param_count() + 2 + (if has_location_filter && table == "images" { 2 } else { 0 });

        query_string = builder.build_select_query(limit_param, offset_param, lon_param_idx, lat_param_idx);
    }

    // Convert limit and offset to i64
    let limit_i64 = limit as i64;
    let offset_i64 = offset as i64;

    // Parse date strings to DateTime<Utc> if provided
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

    // Build parameter vector dynamically
    // We need to bind device_id value to keep it alive
    let device_id_value;
    let label_id_value;
    let mut params: Vec<&(dyn tokio_postgres::types::ToSql + Sync)> = vec![&user_uuid];

    if let Some(dev_id) = device_id {
        device_id_value = dev_id;
        params.push(&device_id_value as &(dyn tokio_postgres::types::ToSql + Sync));
    }

    if media_type != media::TYPE_ALL {
        params.push(&media_type as &(dyn tokio_postgres::types::ToSql + Sync));
    }

    if let Some(lbl_id) = label_id {
        label_id_value = lbl_id;
        params.push(&label_id_value as &(dyn tokio_postgres::types::ToSql + Sync));
    }

    if let Some(ref sd) = start_datetime {
        params.push(sd as &(dyn tokio_postgres::types::ToSql + Sync));
    }

    if let Some(ref ed) = end_datetime {
        params.push(ed as &(dyn tokio_postgres::types::ToSql + Sync));
    }

    // Bind location values outside the block so they live long enough
    let lat_value;
    let lon_value;

    // Add location parameters if provided (lon first, then lat to match SQL)
    // Only add if we actually used them in the query
    if lon_param_idx.is_some() {
        lat_value = location_lat.unwrap();
        lon_value = location_lon.unwrap();
        params.push(&lon_value as &(dyn tokio_postgres::types::ToSql + Sync));
        params.push(&lat_value as &(dyn tokio_postgres::types::ToSql + Sync));
    }

    params.push(&limit_i64 as &(dyn tokio_postgres::types::ToSql + Sync));
    params.push(&offset_i64 as &(dyn tokio_postgres::types::ToSql + Sync));

    // Execute query with dynamically built parameters
    let rows = client.query(&query_string, &params).await?;

    // Map rows to ThumbnailItem
    let thumbnails = rows
        .into_iter()
        .map(|row| {
            let distance_km: Option<f64> = row.get("distance_km");
            let media_type_val: Option<String> = row.try_get("media_type").ok();
            
            // If media_type column exists, use it. Otherwise infer from table (backward compatibility)
            let final_media_type = media_type_val.or_else(|| {
                if table == "images" { Some("image".to_string()) }
                else if table == "videos" { Some("video".to_string()) }
                else { None }
            });

            let hash: String = row.get("hash");
            ThumbnailItem {
                hash: hash.clone(),
                name: row.get("name"),
                created_at: row.get("created_at"),
                place: row.get("place"),
                device_id: row.get("deviceid"),
                starred: row.get("starred"),
                distance_km: distance_km.map(|d| d as f32),
                media_type: final_media_type,
                thumbnail_url: format!("/api/thumbnail/{}", hash),
                file_size_bytes: row.try_get("file_size_bytes").unwrap_or(None),
            }
        })
        .collect();

    Ok(thumbnails)
}

pub async fn total_thumbnails(
    user_id: &str,
    device_id: Option<&str>,
    table: &str,
    media_type: &str,
    starred_only: bool,
    start_date: Option<&str>,
    end_date: Option<&str>,
    location_lat: Option<f64>,
    location_lon: Option<f64>,
    location_radius_km: Option<f64>,
    label_id: Option<i32>,
    apply_user_id_filter: bool,
    pool: &web::Data<MainDbPool>
) -> i64 {
    // Helper to handle errors and return 0 with logging
    let client = match pool.0.get().await {
        Ok(c) => c,
        Err(e) => {
            error!("Failed to get database client for total_thumbnails: {}", e);
            return 0;
        }
    };

    let user_uuid = match uuid::Uuid::parse_str(user_id) {
        Ok(uuid) => uuid,
        Err(e) => {
            error!("Failed to parse user_id as UUID in total_thumbnails: {}", e);
            return 0;
        }
    };

    // Helper closure to apply common filters
    let apply_filters = |builder: &mut MediaQueryBuilder| {
        builder.with_has_thumbnail();
        builder.with_user_id();

        // For non-admin users, filter by user_id for access control
        if apply_user_id_filter {
            builder.with_user_id_filter();
        }

        if device_id.is_some() {
            builder.with_device_id();
        }

        if media_type != media::TYPE_ALL {
            builder.with_media_type();
        }

        if starred_only {
            builder.with_starred_only();
        }

        if label_id.is_some() {
            builder.with_label_id();
        }

        if start_date.is_some() {
            builder.with_start_date();
        }

        if end_date.is_some() {
            builder.with_end_date();
        }
    };

    let query_string;
    let has_location_filter = location_lat.is_some() && location_lon.is_some();
    // Store param indices for location binding
    let mut lon_param_idx = None;

    if table == "all" {
        // Build query for images
        let mut img_builder = MediaQueryBuilder::new("images");
        apply_filters(&mut img_builder);

        // Add location filters to image builder if needed
        if has_location_filter {
            let radius_km = location_radius_km.unwrap_or(10.0);
            let radius_meters = radius_km * 1000.0;
            let lon_param = img_builder.param_count() + 1;
            let lat_param = img_builder.param_count() + 2;
            img_builder.add_custom_condition("t.location IS NOT NULL".to_string());
            img_builder.add_custom_condition(format!(
                "ST_DWithin(t.location, ST_MakePoint(${}, ${})::geography, {})",
                lon_param, lat_param, radius_meters
            ));
            lon_param_idx = Some(lon_param);
        }

        // Build query for videos
        let mut vid_builder = MediaQueryBuilder::new("videos");
        apply_filters(&mut vid_builder);

        // Videos don't have location data, so if filtering by location, exclude them
        if has_location_filter {
            vid_builder.add_custom_condition("1 = 0".to_string());
        }

        let img_count_query = img_builder.build_count_query(starred_only);
        let vid_count_query = vid_builder.build_count_query(starred_only);

        query_string = format!(
            "SELECT ({}) + ({})",
            img_count_query, vid_count_query
        );
    } else {
        let mut builder = MediaQueryBuilder::new(table);
        apply_filters(&mut builder);

        if has_location_filter && table == "images" {
            let radius_km = location_radius_km.unwrap_or(10.0);
            let radius_meters = radius_km * 1000.0;
            let lon_param = builder.param_count() + 1;
            let lat_param = builder.param_count() + 2;
            builder.add_custom_condition("t.location IS NOT NULL".to_string());
            builder.add_custom_condition(format!(
                "ST_DWithin(t.location, ST_MakePoint(${}, ${})::geography, {})",
                lon_param, lat_param, radius_meters
            ));
            lon_param_idx = Some(lon_param);
        }

        query_string = builder.build_count_query(starred_only);
    }

    // Parse date strings to DateTime<Utc> if provided
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

    // Build parameter vector dynamically
    // We need to bind device_id value to keep it alive
    let device_id_value;
    let label_id_value;
    let mut params: Vec<&(dyn tokio_postgres::types::ToSql + Sync)> = vec![&user_uuid];

    if let Some(dev_id) = device_id {
        device_id_value = dev_id;
        params.push(&device_id_value as &(dyn tokio_postgres::types::ToSql + Sync));
    }

    if media_type != media::TYPE_ALL {
        params.push(&media_type as &(dyn tokio_postgres::types::ToSql + Sync));
    }

    if let Some(lbl_id) = label_id {
        label_id_value = lbl_id;
        params.push(&label_id_value as &(dyn tokio_postgres::types::ToSql + Sync));
    }

    if let Some(ref sd) = start_datetime {
        params.push(sd as &(dyn tokio_postgres::types::ToSql + Sync));
    }

    if let Some(ref ed) = end_datetime {
        params.push(ed as &(dyn tokio_postgres::types::ToSql + Sync));
    }

    // Bind location values outside the block so they live long enough
    let lat_value;
    let lon_value;

    // Add location parameters if provided (lon first, then lat to match SQL)
    if lon_param_idx.is_some() {
        lat_value = location_lat.unwrap();
        lon_value = location_lon.unwrap();
        params.push(&lon_value as &(dyn tokio_postgres::types::ToSql + Sync));
        params.push(&lat_value as &(dyn tokio_postgres::types::ToSql + Sync));
    }

    // Execute query with dynamically built parameters
    let row = match client.query_one(&query_string, &params).await {
        Ok(row) => row,
        Err(e) => {
            error!("Failed to get total count: {}", e);
            return 0;
        }
    };

    let total: i64 = if table == "all" {
        // For sum query, result might be optional (though sum of counts is usually not null)
        row.get(0)
    } else {
        row.get(0)
    };

    // Log results
    if let Some(dev_id) = device_id {
        info!("Total thumbnails for device {}: {}", dev_id, total);
    } else {
        info!("Total thumbnails (all devices): {}", total);
    }

    total
}

/// Helper: Parse YYYYMMDD_HHMMSS pattern after a prefix
fn try_parse_datetime_underscore(name: &str, start_pos: usize) -> Option<DateTime<Utc>> {
    // Need at least 15 chars: YYYYMMDD_HHMMSS
    if name.len() < start_pos + 15 {
        return None;
    }

    // Check for underscore separator at position 8
    if name.chars().nth(start_pos + 8) != Some('_') {
        return None;
    }

    let date_part = name.get(start_pos..start_pos + 8)?;
    let time_part = name.get(start_pos + 9..start_pos + 15)?;

    // Validate all digits
    if !date_part.chars().all(|c| c.is_ascii_digit()) ||
       !time_part.chars().all(|c| c.is_ascii_digit()) {
        return None;
    }

    let datetime_str = format!("{} {}", date_part, time_part);
    NaiveDateTime::parse_from_str(&datetime_str, "%Y%m%d %H%M%S")
        .ok()
        .map(|dt| DateTime::<Utc>::from_naive_utc_and_offset(dt, Utc))
}

/// Helper: Parse YYYYMMDD-WAXXXX pattern after a prefix (WhatsApp format)
fn try_parse_whatsapp_format(name: &str, start_pos: usize) -> Option<DateTime<Utc>> {
    let date_end = start_pos + 8;

    // Check for -WA suffix
    if name.len() < date_end + 7 || name.get(date_end..date_end + 3) != Some("-wa") {
        return None;
    }

    let date_part = name.get(start_pos..date_end)?;
    let millis_part = name.get(date_end + 3..date_end + 7)?;

    if !date_part.chars().all(|c| c.is_ascii_digit()) ||
       !millis_part.chars().all(|c| c.is_ascii_digit()) {
        return None;
    }

    let naive_date = NaiveDate::parse_from_str(date_part, "%Y%m%d").ok()?;
    let millis = millis_part.parse::<u32>().ok()?;
    let actual_millis = millis % 1000;

    Some(DateTime::<Utc>::from_naive_utc_and_offset(
        naive_date.and_hms_milli_opt(0, 0, 0, actual_millis)?,
        Utc
    ))
}

/// Helper: Parse YYYYMMDD pattern (date only, no time)
fn try_parse_date_only(name: &str, start_pos: usize) -> Option<DateTime<Utc>> {
    let date_part = name.get(start_pos..start_pos + 8)?;

    if !date_part.chars().all(|c| c.is_ascii_digit()) {
        return None;
    }

    NaiveDate::parse_from_str(date_part, "%Y%m%d")
        .ok()
        .map(|date| DateTime::<Utc>::from_naive_utc_and_offset(
            date.and_hms_opt(0, 0, 0).unwrap(),
            Utc
        ))
}

pub fn parse_date_from_image_name(image_name: &str) -> Option<DateTime<Utc>> {
    let lower_name = image_name.to_lowercase();

    // Try IMG_YYYYMMDD_HHMMSS pattern (Camera images)
    if let Some(pos) = lower_name.find("img_") {
        if let Some(dt) = try_parse_datetime_underscore(&lower_name, pos + 4) {
            return Some(dt);
        }
    }

    // Try IMG-YYYYMMDD-WAXXXX pattern (WhatsApp)
    if let Some(pos) = lower_name.find("img-") {
        if let Some(dt) = try_parse_whatsapp_format(&lower_name, pos + 4) {
            return Some(dt);
        }
        // Fallback to IMG-YYYYMMDD (date only)
        if let Some(dt) = try_parse_date_only(&lower_name, pos + 4) {
            return Some(dt);
        }
    }

    None
}

pub fn parse_date_from_video_name(video_name: &str) -> Option<DateTime<Utc>> {
    let lower_name = video_name.to_lowercase();

    // Try DJI_YYYYMMDD_HHMMSS pattern (DJI drone videos)
    if let Some(pos) = lower_name.find("dji_") {
        if let Some(dt) = try_parse_datetime_underscore(&lower_name, pos + 4) {
            return Some(dt);
        }
    }

    // Try SL_MO_VID_YYYYMMDD_HHMMSS pattern (Samsung slow motion)
    if let Some(pos) = lower_name.find("sl_mo_vid_") {
        if let Some(dt) = try_parse_datetime_underscore(&lower_name, pos + 10) {
            return Some(dt);
        }
    }

    // Try VID_YYYYMMDD_HHMMSS pattern (Camera videos)
    if let Some(pos) = lower_name.find("vid_") {
        if let Some(dt) = try_parse_datetime_underscore(&lower_name, pos + 4) {
            return Some(dt);
        }
    }

    // Try VID-YYYYMMDD-WAXXXX pattern (WhatsApp videos)
    if let Some(pos) = lower_name.find("vid-") {
        if let Some(dt) = try_parse_whatsapp_format(&lower_name, pos + 4) {
            return Some(dt);
        }
        // Fallback to VID-YYYYMMDD (date only)
        if let Some(dt) = try_parse_date_only(&lower_name, pos + 4) {
            return Some(dt);
        }
    }

    None
}

/// Clean up temporary files if they exist
pub async fn cleanup_temp_files(
    image_temp_path: Option<PathBuf>,
    thumbnail_temp_path: Option<PathBuf>
) {
    if let Some(path) = image_temp_path {
        if path.exists() {
            if let Err(e) = fs::remove_file(&path).await {
                log::warn!("Failed to remove temporary image file {:?}: {}", path, e);
            }
        }
    }

    if let Some(path) = thumbnail_temp_path {
        if path.exists() {
            if let Err(e) = fs::remove_file(&path).await {
                log::warn!("Failed to remove temporary thumbnail file {:?}: {}", path, e);
            }
        }
    }
}

/// Helper function to clean up temporary files in error contexts without blocking
pub fn cleanup_temp_files_spawn(
    image_temp_path: Option<PathBuf>,
    thumbnail_temp_path: Option<PathBuf>
) {
    tokio::spawn(async move {
        if let Some(path) = image_temp_path {
            if path.exists() {
                if let Err(e) = fs::remove_file(&path).await {
                    log::warn!("Failed to remove temporary image file {:?}: {}", path, e);
                }
            }
        }

        if let Some(path) = thumbnail_temp_path {
            if path.exists() {
                if let Err(e) = fs::remove_file(&path).await {
                    log::warn!("Failed to remove temporary thumbnail file {:?}: {}", path, e);
                }
            }
        }
    });
}

/// Extract GPS coordinates (latitude, longitude) from EXIF JSON
/// Returns (latitude, longitude) as decimal degrees, or None if GPS data is not available
pub fn extract_gps_coordinates(exif_json: &serde_json::Value) -> Option<(f64, f64)> {
    // Extract GPS components from EXIF
    let gps_lat_str = exif_json.get("GPSLatitude")?.as_str()?;
    let gps_lon_str = exif_json.get("GPSLongitude")?.as_str()?;
    let gps_lat_ref = exif_json.get("GPSLatitudeRef")?.as_str()?;
    let gps_lon_ref = exif_json.get("GPSLongitudeRef")?.as_str()?;

    // Parse GPS coordinates (format: "52 deg 31 min 1.20 sec" or similar)
    let latitude = parse_gps_coordinate(gps_lat_str, gps_lat_ref)?;
    let longitude = parse_gps_coordinate(gps_lon_str, gps_lon_ref)?;

    Some((latitude, longitude))
}

/// Parse GPS coordinate string and convert to decimal degrees
/// Format examples:
/// - "52 deg 31 min 1.20 sec" or "52° 31' 1.20\""
/// - "37/1, 25/1, 1919/100 N" (rational fraction format)
fn parse_gps_coordinate(coord_str: &str, reference: &str) -> Option<f64> {
    // Check if this is rational fraction format (e.g., "37/1, 25/1, 1919/100 N")
    if coord_str.contains('/') && coord_str.contains(',') {
        // Split by comma and parse each rational number
        let parts: Vec<&str> = coord_str.split(',').map(|s| s.trim()).collect();

        if parts.len() < 3 {
            return None;
        }

        // Parse each fraction (degrees/minutes/seconds)
        let degrees = parse_rational(parts[0])?;
        let minutes = parse_rational(parts[1])?;

        // Last part might contain the reference letter (N/S/E/W), remove it
        let seconds_str = parts[2].split_whitespace().next()?;
        let seconds = parse_rational(seconds_str)?;

        // Convert to decimal degrees
        let mut decimal = degrees + (minutes / 60.0) + (seconds / 3600.0);

        // Apply sign based on hemisphere
        if reference == "S" || reference == "W" {
            decimal = -decimal;
        }

        return Some(decimal);
    }

    // Otherwise, handle traditional format: "52 deg 31 min 1.20 sec"
    let cleaned = coord_str
        .replace("deg", "")
        .replace("min", "")
        .replace("sec", "")
        .replace("°", "")
        .replace("'", "")
        .replace("\"", "");

    let parts: Vec<&str> = cleaned.split_whitespace().collect();

    if parts.len() < 3 {
        return None;
    }

    // Parse degrees, minutes, and seconds
    let degrees: f64 = parts[0].parse().ok()?;
    let minutes: f64 = parts[1].parse().ok()?;
    let seconds: f64 = parts[2].parse().ok()?;

    // Convert to decimal degrees
    let mut decimal = degrees + (minutes / 60.0) + (seconds / 3600.0);

    // Apply sign based on hemisphere
    if reference == "S" || reference == "W" {
        decimal = -decimal;
    }

    Some(decimal)
}

/// Parse a rational number string (e.g., "37/1" -> 37.0, "1919/100" -> 19.19)
fn parse_rational(rational_str: &str) -> Option<f64> {
    let parts: Vec<&str> = rational_str.split('/').collect();
    if parts.len() != 2 {
        return None;
    }

    let numerator: f64 = parts[0].trim().parse().ok()?;
    let denominator: f64 = parts[1].trim().parse().ok()?;

    if denominator == 0.0 {
        return None;
    }

    Some(numerator / denominator)
}

/// Performs reverse geocoding to get place name from latitude and longitude coordinates
/// First tries local PostGIS database lookup, then falls back to external Nominatim API
/// Returns None if geocoding fails or coordinates are invalid
pub async fn reverse_geocode(
    latitude: f64,
    longitude: f64,
    geotagging_pool: &web::Data<GeotaggingDbPool>,
    enable_local: bool,
    enable_external_fallback: bool,
) -> Option<String> {
    // Validate coordinates
    if latitude.is_nan() || longitude.is_nan() ||
       latitude < -90.0 || latitude > 90.0 ||
       longitude < -180.0 || longitude > 180.0 {
        warn!("Invalid coordinates for reverse geocoding: lat={}, lon={}", latitude, longitude);
        return None;
    }

    // Try local geocoding first
    if enable_local {
        if let Some(place) = reverse_geocode_local(latitude, longitude, geotagging_pool).await {
            info!("Local geocoding successful for ({}, {}): {}", latitude, longitude, place);
            return Some(place);
        }
        info!("Local geocoding returned no result for ({}, {})", latitude, longitude);
    }

    // Fall back to external Nominatim API
    if enable_external_fallback {
        info!("Trying external Nominatim API for ({}, {})", latitude, longitude);
        return reverse_geocode_external(latitude, longitude).await;
    }

    warn!("Geocoding failed for ({}, {}): no local result and external fallback disabled", latitude, longitude);
    None
}

/// Performs local reverse geocoding using admin_boundaries table in PostgreSQL
/// Retrieves all administrative levels containing the coordinates and builds a complete address
async fn reverse_geocode_local(
    latitude: f64,
    longitude: f64,
    geotagging_pool: &web::Data<GeotaggingDbPool>,
) -> Option<String> {
    let client = match geotagging_pool.0.get().await {
        Ok(client) => client,
        Err(e) => {
            warn!("Failed to get database client for local geocoding: {}", e);
            return None;
        }
    };

    // Query for all administrative boundaries containing the point
    // Order by admin_level DESC to get most specific first
    // Hierarchy: locality (10) -> city (8) -> county/district (6) -> state/province (4) -> country (2)
    let query = r#"
        SELECT name, admin_level, country_code
        FROM admin_boundaries
        WHERE ST_Contains(geometry, ST_SetSRID(ST_MakePoint($1, $2), 4326))
        ORDER BY admin_level DESC
    "#;

    match client.query(query, &[&longitude, &latitude]).await {
        Ok(rows) if !rows.is_empty() => {
            // Extract names from all administrative levels
            let mut place_parts: Vec<String> = Vec::new();

            for row in &rows {
                let name: String = row.get("name");
                let admin_level: i32 = row.get("admin_level");

                // Add the name to our parts list
                // Skip duplicate names (sometimes boundaries overlap with same name)
                if !place_parts.contains(&name) {
                    place_parts.push(name);
                }

                info!("Found admin_level {} boundary: {}", admin_level, place_parts.last().unwrap());
            }

            // Build complete address from all levels
            // Format: "Village/City, District/Region, Country" or similar
            let place = place_parts.join(", ");

            info!("Local geocoding successful for ({}, {}): {}", latitude, longitude, place);
            Some(place)
        }
        Ok(_) => {
            info!("No matching boundary found for ({}, {}) in local database", latitude, longitude);
            None
        }
        Err(e) => {
            warn!("Failed to query admin_boundaries for ({}, {}): {}", latitude, longitude, e);
            None
        }
    }
}

/// Performs reverse geocoding using external Nominatim (OpenStreetMap) API
async fn reverse_geocode_external(latitude: f64, longitude: f64) -> Option<String> {
    // Build Nominatim API URL
    let url = format!(
        "https://nominatim.openstreetmap.org/reverse?lat={}&lon={}&format=json&addressdetails=1&zoom=14",
        latitude, longitude
    );

    // Make HTTP request with timeout
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(5))
        .user_agent("Reminisce/1.0") // Nominatim requires a user agent
        .build()
        .ok()?;

    match client.get(&url).send().await {
        Ok(response) => {
            if response.status().is_success() {
                match response.json::<serde_json::Value>().await {
                    Ok(json) => {
                        // Try to extract a meaningful place name
                        // Priority: display_name > address components
                        if let Some(display_name) = json.get("display_name").and_then(|v| v.as_str()) {
                            info!("External geocoding successful for ({}, {}): {}", latitude, longitude, display_name);
                            return Some(display_name.to_string());
                        }
                        warn!("No display_name found in Nominatim response for ({}, {})", latitude, longitude);
                    }
                    Err(e) => {
                        warn!("Failed to parse Nominatim response for ({}, {}): {}", latitude, longitude, e);
                    }
                }
            } else {
                warn!("Nominatim API returned status {} for ({}, {})", response.status(), latitude, longitude);
            }
        }
        Err(e) => {
            warn!("Failed to call Nominatim API for ({}, {}): {}", latitude, longitude, e);
        }
    }

    None
}

/// Helper to dump DB to a file
pub fn perform_db_dump(config: &crate::config::Config) -> Result<PathBuf, String> {
    let database_url = config.database_url.as_ref().ok_or("Database URL not configured")?;

    // Extract password from database URL
    let password = url::Url::parse(database_url)
        .ok()
        .and_then(|url| url.password().map(|p| p.to_string()))
        .unwrap_or_else(|| "postgres".to_string());

    let output_path = PathBuf::from(format!("db_dump_temp_{}.sql", chrono::Utc::now().timestamp()));
    
    let file = std::fs::File::create(&output_path).map_err(|e| e.to_string())?;

    let mut command = std::process::Command::new("pg_dump");
    command
        .arg("--format=plain")
        .env("PGPASSWORD", password)
        .arg(database_url)
        .stdout(file); // Direct to file

    match command.status() {
        Ok(status) if status.success() => Ok(output_path),
        Ok(_) => Err("pg_dump failed".to_string()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Err("pg_dump missing".to_string()),
        Err(e) => Err(e.to_string()),
    }
}

/// Parse a peer address string into a SocketAddr.
/// Handles both "ip:port" and bare "ip" (defaults to port 5050).
pub fn parse_peer_addr(peer: &str) -> Result<std::net::SocketAddr, String> {
    if let Ok(addr) = peer.parse::<std::net::SocketAddr>() {
        return Ok(addr);
    }
    format!("{}:5050", peer).parse::<std::net::SocketAddr>()
        .map_err(|e| format!("Invalid peer address '{}': {}", peer, e))
}

/// Generic helper for adaptive worker loops
///
/// Implements exponential backoff when idle, and "fast lane" (min_interval) when work is found.
///
/// # Arguments
/// * `name` - Worker name for logging
/// * `min_interval` - Interval when active (or after finding work)
/// * `max_interval` - Maximum interval when idle
/// * `task` - Closure that returns `Ok(true)` if work was done, `Ok(false)` if idle, or `Err`
pub async fn run_worker_loop<F, Fut>(
    name: &str,
    min_interval: std::time::Duration,
    max_interval: std::time::Duration,
    mut task: F,
) where
    F: FnMut() -> Fut,
    Fut: std::future::Future<Output = Result<bool, String>>,
{
    let mut current_interval = min_interval;

    loop {
        // Run the task
        match task().await {
            Ok(did_work) => {
                if did_work {
                    // Fast lane: Work was found, reset to minimum interval
                    // If we just processed a full batch, we might want to run again immediately (0s),
                    // but min_interval gives breathing room.
                    current_interval = min_interval;
                } else {
                    // No work: Backoff exponentially
                    current_interval = (current_interval * 2).min(max_interval);
                }
            }
            Err(e) => {
                error!("Worker '{}' failed: {}", name, e);
                // On error, also backoff to avoid hammering a broken system
                current_interval = (current_interval * 2).min(max_interval);
            }
        }

        // Sleep for the determined interval
        tokio::time::sleep(current_interval).await;
    }
}