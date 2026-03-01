/// Full-text search functionality for image descriptions
/// Separated from embedding search for clarity

use actix_web::web;
use log::info;
use crate::db::MainDbPool;
use crate::services::embedding::SearchResult;
use crate::utils;

/// Perform full-text search on image descriptions
pub async fn search_by_text(
    query_text: &str,
    user_uuid: &uuid::Uuid,
    device_filter: Option<&String>,
    starred_only: bool,
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

    // Build dynamic WHERE conditions
    let mut conditions = vec![
        "i.description IS NOT NULL",
        "i.description != ''",
        "i.user_id = $1",
        "i.deleted_at IS NULL",
        "to_tsvector('english', COALESCE(i.description, '') || ' ' || COALESCE(i.name, '')) @@ plainto_tsquery('english', $2)",
    ];
    let mut owned_conditions: Vec<String> = vec![];
    let mut param_count = 4; // $1=user_id, $2=query, $3=limit, $4=offset

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

    // Full-text search query
    // ts_rank gives us a relevance score (higher = better match)
    let distance_calc = if has_location_filter {
        // Use parameter placeholders that match the location filter params
        format!(
            "ST_Distance(i.location, ST_MakePoint(${}, ${})::geography) / 1000.0",
            param_count - 1, param_count  // lon_param, lat_param
        )
    } else {
        "NULL::double precision".to_string()
    };

    let sql = format!(
        "SELECT i.hash, i.name, i.description, i.place, i.created_at,
                ts_rank(to_tsvector('english', COALESCE(i.description, '') || ' ' || COALESCE(i.name, '')),
                        plainto_tsquery('english', $2)) as similarity,
                CASE WHEN s.hash IS NOT NULL THEN true ELSE false END as starred,
                i.deviceid,
                {} as distance_km
         FROM images i
         LEFT JOIN starred_images s ON i.hash = s.hash AND s.user_id = $1
         WHERE {}
         ORDER BY similarity DESC, i.created_at DESC
         LIMIT $3 OFFSET $4",
        distance_calc,
        where_clause
    );

    info!("Executing text search query for: '{}'", query_text);

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

    let mut params: Vec<&(dyn tokio_postgres::types::ToSql + Sync)> = vec![user_uuid, &query_text, &limit, &offset];

    if let Some(device) = device_filter {
        params.push(device);
    }

    if let Some(ref sd) = start_datetime {
        params.push(sd);
    }
    if let Some(ref ed) = end_datetime {
        params.push(ed);
    }

    let lat_val;
    let lon_val;
    if has_location_filter {
        lat_val = location_lat.unwrap();
        lon_val = location_lon.unwrap();
        params.push(&lon_val);
        params.push(&lat_val);
    }

    let rows = client.query(&sql, &params).await.map_err(|e| {
        log::error!("Failed to execute text search query: {}", e);
        actix_web::error::ErrorInternalServerError("Text search query failed")
    })?;

    let results: Vec<SearchResult> = rows
        .iter()
        .map(|row| {
            let hash: String = row.get(0);
            let similarity = row.get::<_, f32>(5);
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

    info!("Text search found {} results for query: '{}'", results.len(), query_text);

    Ok(results)
}

/// Perform hybrid search combining semantic and text search
pub async fn search_hybrid(
    query_text: &str,
    embedding: &pgvector::Vector,
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

    // Build dynamic WHERE conditions
    let similarity_threshold = format!("(1 - (i.embedding <=> $2)) > {}", min_similarity);
    let mut conditions = vec![
        "i.user_id = $1",
        "i.deleted_at IS NULL",
    ];
    let mut owned_conditions: Vec<String> = vec![similarity_threshold];
    let mut param_count = 5; // $1=user_id, $2=embedding, $3=query_text, $4=limit, $5=offset

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

    // Hybrid search: combine vector similarity and text relevance
    // weighted_score = (vector_similarity * 0.7) + (text_relevance * 0.3)
    let distance_calc = if has_location_filter {
        // Use parameter placeholders that match the location filter params
        format!(
            "ST_Distance(i.location, ST_MakePoint(${}, ${})::geography) / 1000.0",
            param_count - 1, param_count  // lon_param, lat_param
        )
    } else {
        "NULL::double precision".to_string()
    };

    let sql = format!(
        "SELECT i.hash, i.name, i.description, i.place, i.created_at,
                ((1 - (i.embedding <=> $2)) * 0.7 +
                 CASE
                   WHEN i.description IS NOT NULL AND i.description != '' THEN
                     ts_rank(to_tsvector('english', COALESCE(i.description, '') || ' ' || COALESCE(i.name, '')),
                             plainto_tsquery('english', $3)) * 0.3
                   ELSE 0
                 END) as similarity,
                CASE WHEN s.hash IS NOT NULL THEN true ELSE false END as starred,
                i.deviceid,
                {} as distance_km
         FROM images i
         LEFT JOIN starred_images s ON i.hash = s.hash AND s.user_id = $1
         WHERE {}
         ORDER BY similarity DESC
         LIMIT $4 OFFSET $5",
        distance_calc,
        where_clause
    );

    info!("Executing hybrid search query for: '{}'", query_text);

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

    let mut params: Vec<&(dyn tokio_postgres::types::ToSql + Sync)> = vec![user_uuid, embedding, &query_text, &limit, &offset];

    if let Some(device) = device_filter {
        params.push(device);
    }

    if let Some(ref sd) = start_datetime {
        params.push(sd);
    }
    if let Some(ref ed) = end_datetime {
        params.push(ed);
    }

    let lat_val;
    let lon_val;
    if has_location_filter {
        lat_val = location_lat.unwrap();
        lon_val = location_lon.unwrap();
        params.push(&lon_val);
        params.push(&lat_val);
    }

    let rows = client.query(&sql, &params).await.map_err(|e| {
        log::error!("Failed to execute hybrid search query: {}", e);
        actix_web::error::ErrorInternalServerError("Hybrid search query failed")
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

    info!("Hybrid search found {} results for query: '{}'", results.len(), query_text);

    Ok(results)
}
