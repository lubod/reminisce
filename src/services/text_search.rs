// Full-text and hybrid search. Both modes search images AND videos, support
// label + media_type + date/device/location filters, and return the raw fetch
// count so the search endpoint can report a pagination-correct total.

use actix_web::web;
use log::info;
use crate::db::MainDbPool;
use crate::services::embedding::SearchResult;
use crate::utils;

fn parse_datetime_pair(
    start_date: Option<&String>,
    end_date: Option<&String>,
) -> (
    Option<chrono::DateTime<chrono::Utc>>,
    Option<chrono::DateTime<chrono::Utc>>,
) {
    use chrono::{DateTime, NaiveDate, TimeZone, Utc};
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
    (start_datetime, end_datetime)
}

/// Shared WHERE-clause builder for a per-table arm. `threshold` already has the
/// correct alias baked in (e.g. `i.embedding` vs `v.embedding`).
#[allow(clippy::too_many_arguments)]
fn build_arm_where(
    threshold: &str,
    alias: &str,
    device_param: Option<i64>,
    starred_only: bool,
    label_param: Option<i64>,
    start_param: Option<i64>,
    end_param: Option<i64>,
    lon_param: Option<i64>,
    lat_param: Option<i64>,
    radius_meters: f64,
    label_table: &str,
    label_hash: &str,
    label_user: &str,
) -> String {
    let mut conds: Vec<String> = vec![
        format!("{}.user_id = $1", alias),
        format!("{}.deleted_at IS NULL", alias),
        threshold.to_string(),
    ];
    if let Some(p) = device_param {
        conds.push(format!("{}.deviceid = ${}", alias, p));
    }
    if starred_only {
        conds.push("s.hash IS NOT NULL".to_string());
    }
    if let Some(p) = label_param {
        conds.push(format!(
            "EXISTS (SELECT 1 FROM {} l WHERE l.{} = {}.hash AND l.{} = {}.user_id AND l.label_id = ${})",
            label_table, label_hash, alias, label_user, alias, p
        ));
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
}

fn build_distance_expr(
    lon_param: Option<i64>,
    lat_param: Option<i64>,
    alias: &str,
) -> String {
    if let (Some(lon), Some(lat)) = (lon_param, lat_param) {
        format!(
            "ST_Distance({}.location, ST_MakePoint(${}, ${})::geography) / 1000.0",
            alias, lon, lat
        )
    } else {
        "NULL::double precision".to_string()
    }
}

/// Perform full-text search on image (and video) descriptions/names.
#[allow(clippy::too_many_arguments)]
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
    label_id: Option<i32>,
    media_type: &str,
    pool: &web::Data<MainDbPool>,
) -> Result<(Vec<SearchResult>, usize), actix_web::Error> {
    let client = utils::get_db_client(&pool.0).await?;

    let include_image = media_type != "video";
    let include_video = media_type != "image";
    let per_branch_limit = limit + offset;
    // $1=user_id, $2=query_text, $3=per_branch_limit, $4=final_limit, $5=offset
    let mut param_count = 5;

    let device_param = if device_filter.is_some() {
        param_count += 1;
        Some(param_count)
    } else {
        None
    };

    let label_param = if label_id.is_some() {
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

    let has_location_filter = location_lat.is_some() && location_lon.is_some();
    let mut lon_param: Option<i64> = None;
    let mut lat_param: Option<i64> = None;
    let radius_meters = if has_location_filter {
        let radius_km = location_radius_km.unwrap_or(10.0);
        param_count += 1;
        lon_param = Some(param_count);
        param_count += 1;
        lat_param = Some(param_count);
        info!(
            "Location filter: lat={:.4}, lon={:.4}, radius={:.1}km",
            location_lat.unwrap(),
            location_lon.unwrap(),
            radius_km
        );
        radius_km * 1000.0
    } else {
        0.0
    };

    let base_desc_where = |alias: &str| -> String {
        format!(
            "{a}.description IS NOT NULL AND {a}.description != '' AND \
             to_tsvector('english', COALESCE({a}.description, '') || ' ' || COALESCE({a}.name, '')) \
             @@ plainto_tsquery('english', $2)",
            a = alias
        )
    };

    let anchor = |alias: &str, label_table: &str, label_hash: &str, label_user: &str| -> String {
        let mut conds: Vec<String> = vec![
            format!("{}.user_id = $1", alias),
            format!("{}.deleted_at IS NULL", alias),
            base_desc_where(alias),
        ];
        if let Some(p) = device_param {
            conds.push(format!("{}.deviceid = ${}", alias, p));
        }
        if starred_only {
            conds.push("s.hash IS NOT NULL".to_string());
        }
        if let Some(p) = label_param {
            conds.push(format!(
                "EXISTS (SELECT 1 FROM {} l WHERE l.{} = {}.hash AND l.{} = {}.user_id AND l.label_id = ${})",
                label_table, label_hash, alias, label_user, alias, p
            ));
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

    let img_where = anchor("i", "image_labels", "image_hash", "image_user_id");
    let vid_where = anchor("v", "video_labels", "video_hash", "video_user_id");
    let img_distance = build_distance_expr(lon_param, lat_param, "i");
    let vid_distance = build_distance_expr(lon_param, lat_param, "v");

    let ts_rank = |alias: &str| -> String {
        format!(
            "ts_rank(to_tsvector('english', COALESCE({}.description, '') || ' ' || COALESCE({}.name, '')), plainto_tsquery('english', $2))",
            alias, alias
        )
    };

    let img_arm = if include_image {
        format!(
            "(\n                SELECT i.hash, i.name, i.description, i.place, i.created_at,\n                       {} as similarity,\n                       CASE WHEN s.hash IS NOT NULL THEN true ELSE false END as starred,\n                       i.deviceid, {} as distance_km, 'image' as media_type\n                FROM images i\n                LEFT JOIN starred_images s ON i.hash = s.hash AND s.user_id = $1\n                WHERE {}\n                ORDER BY similarity DESC, i.created_at DESC\n                LIMIT $3\n            )",
            ts_rank("i"), img_distance, img_where
        )
    } else {
        String::new()
    };
    let vid_arm = if include_video {
        format!(
            "(\n                SELECT v.hash, v.name, v.description, NULL::text as place, v.created_at,\n                       {} as similarity,\n                       CASE WHEN s.hash IS NOT NULL THEN true ELSE false END as starred,\n                       v.deviceid, {} as distance_km, 'video' as media_type\n                FROM videos v\n                LEFT JOIN starred_videos s ON v.hash = s.hash AND s.user_id = $1\n                WHERE {}\n                ORDER BY similarity DESC, v.created_at DESC\n                LIMIT $3\n            )",
            ts_rank("v"), vid_distance, vid_where
        )
    } else {
        String::new()
    };

    let union_body = match (include_image, include_video) {
        (true, true) => format!("{} UNION ALL {}", img_arm, vid_arm),
        (true, false) => img_arm,
        (false, true) => vid_arm,
        (false, false) => unreachable!(),
    };

    let sql = format!(
        "SELECT hash, name, description, place, created_at, similarity, starred, deviceid, distance_km, media_type FROM (\n            {}\n        ) combined\n        ORDER BY similarity DESC, created_at DESC\n        LIMIT $4 OFFSET $5",
        union_body
    );

    info!("Executing text search query for: '{}'", query_text);

    let (start_datetime, end_datetime) = parse_datetime_pair(start_date, end_date);

    let mut params: Vec<&(dyn tokio_postgres::types::ToSql + Sync)> =
        vec![user_uuid, &query_text, &per_branch_limit, &limit, &offset];

    if let Some(device) = device_filter {
        params.push(device);
    }

    let label_id_val;
    if let Some(lbl) = label_id {
        label_id_val = lbl;
        params.push(&label_id_val);
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

    let fetched = results.len();
    info!(
        "Text search found {} results (raw fetched={}) for query: '{}'",
        results.len(),
        fetched,
        query_text
    );

    Ok((results, fetched))
}

/// Perform hybrid search combining semantic and text search across images & videos.
#[allow(clippy::too_many_arguments)]
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
    label_id: Option<i32>,
    media_type: &str,
    pool: &web::Data<MainDbPool>,
) -> Result<(Vec<SearchResult>, usize), actix_web::Error> {
    let client = utils::get_db_client(&pool.0).await?;

    let include_image = media_type != "video";
    let include_video = media_type != "image";
    let per_branch_limit = limit + offset;
    // $1=user_id, $2=embedding, $3=query_text, $4=per_branch_limit, $5=final_limit, $6=offset
    let mut param_count = 6;

    let device_param = if device_filter.is_some() {
        param_count += 1;
        Some(param_count)
    } else {
        None
    };

    let label_param = if label_id.is_some() {
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

    let has_location_filter = location_lat.is_some() && location_lon.is_some();
    let mut lon_param: Option<i64> = None;
    let mut lat_param: Option<i64> = None;
    let radius_meters = if has_location_filter {
        let radius_km = location_radius_km.unwrap_or(10.0);
        param_count += 1;
        lon_param = Some(param_count);
        param_count += 1;
        lat_param = Some(param_count);
        info!(
            "Location filter: lat={:.4}, lon={:.4}, radius={:.1}km",
            location_lat.unwrap(),
            location_lon.unwrap(),
            radius_km
        );
        radius_km * 1000.0
    } else {
        0.0
    };

    let img_threshold = format!("(1 - (i.embedding <=> $2)) > {}", min_similarity);
    let vid_threshold = format!("(1 - (v.embedding <=> $2)) > {}", min_similarity);

    let img_where = build_arm_where(
        &img_threshold,
        "i",
        device_param,
        starred_only,
        label_param,
        start_param,
        end_param,
        lon_param,
        lat_param,
        radius_meters,
        "image_labels",
        "image_hash",
        "image_user_id",
    );
    let vid_where = build_arm_where(
        &vid_threshold,
        "v",
        device_param,
        starred_only,
        label_param,
        start_param,
        end_param,
        lon_param,
        lat_param,
        radius_meters,
        "video_labels",
        "video_hash",
        "video_user_id",
    );

    let img_distance = build_distance_expr(lon_param, lat_param, "i");
    let vid_distance = build_distance_expr(lon_param, lat_param, "v");

    // Hybrid similarity: vector*0.7 + ts_rank*0.3
    let hybrid_score = |alias: &str| -> String {
        format!(
            "((1 - ({a}.embedding <=> $2)) * 0.7 + CASE WHEN {a}.description IS NOT NULL AND {a}.description != '' \
             THEN ts_rank(to_tsvector('english', COALESCE({a}.description, '') || ' ' || COALESCE({a}.name, '')), plainto_tsquery('english', $3)) * 0.3 ELSE 0 END)",
            a = alias
        )
    };

    let img_arm = if include_image {
        format!(
            "(\n                SELECT i.hash, i.name, i.description, i.place, i.created_at,\n                       {} as similarity,\n                       CASE WHEN s.hash IS NOT NULL THEN true ELSE false END as starred,\n                       i.deviceid, {} as distance_km, 'image' as media_type\n                FROM images i\n                LEFT JOIN starred_images s ON i.hash = s.hash AND s.user_id = $1\n                WHERE {}\n                ORDER BY similarity DESC\n                LIMIT $4\n            )",
            hybrid_score("i"), img_distance, img_where
        )
    } else {
        String::new()
    };
    let vid_arm = if include_video {
        format!(
            "(\n                SELECT v.hash, v.name, v.description, NULL::text as place, v.created_at,\n                       {} as similarity,\n                       CASE WHEN s.hash IS NOT NULL THEN true ELSE false END as starred,\n                       v.deviceid, {} as distance_km, 'video' as media_type\n                FROM videos v\n                LEFT JOIN starred_videos s ON v.hash = s.hash AND s.user_id = $1\n                WHERE {}\n                ORDER BY similarity DESC\n                LIMIT $4\n            )",
            hybrid_score("v"), vid_distance, vid_where
        )
    } else {
        String::new()
    };

    let union_body = match (include_image, include_video) {
        (true, true) => format!("{} UNION ALL {}", img_arm, vid_arm),
        (true, false) => img_arm,
        (false, true) => vid_arm,
        (false, false) => unreachable!(),
    };

    let sql = format!(
        "SELECT hash, name, description, place, created_at, similarity, starred, deviceid, distance_km, media_type FROM (\n            {}\n        ) combined\n        ORDER BY similarity DESC\n        LIMIT $5 OFFSET $6",
        union_body
    );

    let ef_search = std::cmp::max(per_branch_limit * 2, 100);
    client
        .execute(&format!("SET hnsw.ef_search = {}", ef_search), &[])
        .await
        .unwrap_or(0);

    info!("Executing hybrid search query for: '{}'", query_text);

    let (start_datetime, end_datetime) = parse_datetime_pair(start_date, end_date);

    let mut params: Vec<&(dyn tokio_postgres::types::ToSql + Sync)> = vec![
        user_uuid, embedding, &query_text, &per_branch_limit, &limit, &offset,
    ];

    if let Some(device) = device_filter {
        params.push(device);
    }

    let label_id_val;
    if let Some(lbl) = label_id {
        label_id_val = lbl;
        params.push(&label_id_val);
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

    let fetched = results.len();
    info!(
        "Hybrid search found {} results (raw fetched={}) for query: '{}'",
        results.len(),
        fetched,
        query_text
    );

    Ok((results, fetched))
}
