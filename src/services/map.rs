//! Map view support: return geotagged media (images only — videos carry no
//! location) as lightweight map points for client-side clustering.
//!
//! The endpoint is paginated (`page`/`limit`, `limit` capped) so a large
//! geotagged library can never be returned in a single unbounded response;
//! `total` is the overall match count so clients can page through all points.

use actix_web::{ get, web, HttpResponse, HttpRequest };
use serde::{Deserialize, Serialize};
use log::{error, info};
use utoipa::ToSchema;

use crate::config::Config;
use crate::db::MainDbPool;
use crate::utils;

/// Upper bound on points returned in a single /map/media response.
const MAP_LIMIT_MAX: usize = 10_000;
/// Default page size if the client does not ask for one.
const MAP_DEFAULT_LIMIT: usize = 1_000;

#[derive(Serialize, Deserialize, ToSchema)]
pub struct MapPoint {
    pub hash: String,
    pub lon: f64,
    pub lat: f64,
    pub created_at: chrono::DateTime<chrono::Utc>,
    pub place: Option<String>,
    pub starred: bool,
    pub device_id: Option<String>,
    pub has_thumbnail: bool,
}

#[derive(Serialize, Deserialize, ToSchema)]
pub struct MapPointsResponse {
    pub points: Vec<MapPoint>,
    pub total: usize,
}

#[derive(Deserialize, ToSchema)]
pub struct MapQuery {
    /// Starred-only filter
    #[serde(default)]
    starred_only: bool,
    /// Optional label ID filter
    #[serde(default)]
    pub label_id: Option<i32>,
    /// Optional start date filter (YYYY-MM-DD)
    #[serde(default)]
    start_date: Option<String>,
    /// Optional end date filter (YYYY-MM-DD, inclusive)
    #[serde(default)]
    end_date: Option<String>,
    /// Optional device ID filter
    #[serde(default)]
    pub device_id: Option<String>,
    /// Page number, 1-based
    #[serde(default = "default_page")]
    pub page: usize,
    /// Points per page (capped at MAP_LIMIT_MAX)
    #[serde(default = "default_limit")]
    pub limit: usize,
    /// Keyset cursor: fetch points strictly older than this created_at (with
    /// `after_hash` as the tiebreaker). When set, `page` is ignored — this gives
    /// drift-free pagination under concurrent inserts (no duplicate/missed points).
    #[serde(default)]
    pub after_created_at: Option<chrono::DateTime<chrono::Utc>>,
    /// Keyset cursor tiebreaker: the hash of the last point returned on the
    /// previous page. Required when `after_created_at` is set.
    #[serde(default)]
    pub after_hash: Option<String>,
}

fn default_page() -> usize {
    1
}

fn default_limit() -> usize {
    MAP_DEFAULT_LIMIT
}

#[utoipa::path(
    get,
    path = "/map/media",
    params(
        ("starred_only" = Option<bool>, Query, description = "Only starred media"),
        ("label_id" = Option<i32>, Query, description = "Only media carrying this label"),
        ("start_date" = Option<String>, Query, description = "created_at >= date"),
        ("end_date" = Option<String>, Query, description = "created_at < date+1d"),
        ("device_id" = Option<String>, Query, description = "Only media from this device"),
        ("page" = Option<usize>, Query, description = "Page number, 1-based"),
        ("limit" = Option<usize>, Query, description = "Points per page (capped)"),
        ("after_created_at" = Option<String>, Query, description = "Keyset cursor: resume after this created_at (requires after_hash)"),
        ("after_hash" = Option<String>, Query, description = "Keyset cursor tiebreaker: hash of the last returned point")
    ),
    responses(
        (status = 200, description = "Geotagged media points", body = MapPointsResponse),
        (status = 401, description = "Unauthorized"),
        (status = 500, description = "Internal server error")
    ),
    tag = "Map"
)]
#[get("/map/media")]
pub async fn get_map_points(
    req: HttpRequest,
    query: web::Query<MapQuery>,
    pool: web::Data<MainDbPool>,
    config: web::Data<Config>,
) -> Result<HttpResponse, actix_web::Error> {
    let claims = match utils::authenticate_request(&req, "get_map_points", config.get_api_key()).await {
        Ok(claims) => claims,
        Err(response) => return Ok(response),
    };

    let client = utils::get_db_client(&pool.0).await?;

    use chrono::TimeZone;
    let start_datetime: Option<chrono::DateTime<chrono::Utc>> = query.start_date.as_deref().and_then(|d| {
        chrono::NaiveDate::parse_from_str(d, "%Y-%m-%d")
            .ok()
            .and_then(|nd| nd.and_hms_opt(0, 0, 0))
            .map(|ndt| chrono::Utc.from_utc_datetime(&ndt))
    });
    let end_datetime: Option<chrono::DateTime<chrono::Utc>> = query.end_date.as_deref().and_then(|d| {
        chrono::NaiveDate::parse_from_str(d, "%Y-%m-%d")
            .ok()
            .and_then(|nd| nd.and_hms_opt(23, 59, 59))
            .and_then(|ndt| ndt.checked_add_signed(chrono::Duration::seconds(1)))
            .map(|ndt| chrono::Utc.from_utc_datetime(&ndt))
    });

    let mut conditions: Vec<String> = vec![
        "i.user_id = $1".to_string(),
        "i.deleted_at IS NULL".to_string(),
        "i.location IS NOT NULL".to_string(),
    ];
    let mut param_count = 1;

    if query.device_id.is_some() {
        param_count += 1;
        conditions.push(format!("i.deviceid = ${}", param_count));
    }
    if query.label_id.is_some() {
        param_count += 1;
        conditions.push(format!(
            "EXISTS (SELECT 1 FROM image_labels l WHERE l.image_hash = i.hash AND l.image_user_id = i.user_id AND l.label_id = ${})",
            param_count
        ));
    }
    if query.starred_only {
        conditions.push("s.hash IS NOT NULL".to_string());
    }
    if start_datetime.is_some() {
        param_count += 1;
        conditions.push(format!("i.created_at >= ${}", param_count));
    }
    if end_datetime.is_some() {
        param_count += 1;
        conditions.push(format!("i.created_at < ${}", param_count));
    }

    // Keyset cursor: resume right after the last point from the previous page,
    // ordered by (created_at DESC, hash DESC). Fall back to OFFSET paging only
    // when no cursor is supplied (backward compatibility).
    let use_cursor = query.after_created_at.is_some();
    if use_cursor {
        if query.after_hash.is_none() {
            return Ok(HttpResponse::BadRequest().json(serde_json::json!({
                "error": "after_hash is required when after_created_at is set"
            })));
        }
        param_count += 2;
        let cursor_ts = format!("${}", param_count - 1);
        let cursor_hash = format!("${}", param_count);
        conditions.push(format!(
            "(i.created_at < {} OR (i.created_at = {} AND i.hash < {}))",
            cursor_ts, cursor_ts, cursor_hash
        ));
    }

    let where_clause = conditions.join(" AND ");

    // Total match count (bounded by the same filters) — enables pagination.
    let count_sql = format!(
        "SELECT COUNT(*) FROM ( \
            SELECT DISTINCT i.hash FROM images i \
            LEFT JOIN starred_images s ON i.hash = s.hash AND s.user_id = $1 \
            WHERE {} \
        ) t",
        where_clause
    );

    let select_order = if use_cursor {
        format!("ORDER BY i.created_at DESC, i.hash DESC LIMIT ${}", param_count + 1)
    } else {
        format!("ORDER BY i.created_at DESC, i.hash DESC LIMIT ${} OFFSET ${}", param_count + 1, param_count + 2)
    };
    let select_sql = format!(
        "SELECT i.hash, ST_X(i.location::geometry) AS lon, ST_Y(i.location::geometry) AS lat, \
                i.created_at, i.place, \
                CASE WHEN s.hash IS NOT NULL THEN true ELSE false END AS starred, \
                i.deviceid, i.has_thumbnail \
         FROM images i \
         LEFT JOIN starred_images s ON i.hash = s.hash AND s.user_id = $1 \
         WHERE {} \
         {}",
        where_clause, select_order
    );

    let user_uuid = utils::parse_user_uuid(&claims.user_id)?;

    let mut params: Vec<&(dyn tokio_postgres::types::ToSql + Sync)> = vec![&user_uuid];

    let device_id_value: String;
    if let Some(dev) = query.device_id.as_deref() {
        device_id_value = dev.to_string();
        params.push(&device_id_value);
    }

    let label_id_val;
    if let Some(label) = query.label_id {
        label_id_val = label;
        params.push(&label_id_val);
    }

    if let Some(ref sd) = start_datetime {
        params.push(sd);
    }
    if let Some(ref ed) = end_datetime {
        params.push(ed);
    }

    // Keyset cursor values go last among the WHERE params (after start/end dates)
    // so the COUNT query (which uses `params[..param_count]`) stays consistent.
    let after_created_at_dt;
    let after_hash_val;
    if use_cursor {
        after_created_at_dt = query.after_created_at.unwrap();
        after_hash_val = query.after_hash.as_deref().unwrap().to_string();
        params.push(&after_created_at_dt);
        params.push(&after_hash_val);
    }

    let total: i64 = match client.query(&count_sql, &params[..param_count]).await {
        Ok(rows) => rows[0].get::<_, i64>(0),
        Err(e) => {
            error!("Failed to count map points: {}", e);
            return Ok(HttpResponse::InternalServerError().json(serde_json::json!({
                "error": "Failed to load map points"
            })));
        }
    };

    let page = query.page.max(1);
    let limit = query.limit.clamp(1, MAP_LIMIT_MAX);
    let offset = (page - 1) * limit;

    let limit_i64 = limit as i64;
    let offset_i64 = offset as i64;
    params.push(&limit_i64);
    if !use_cursor {
        params.push(&offset_i64);
    }

    let rows = match client.query(&select_sql, &params).await {
        Ok(r) => r,
        Err(e) => {
            error!("Failed to query map points: {}", e);
            return Ok(HttpResponse::InternalServerError().json(serde_json::json!({
                "error": "Failed to load map points"
            })));
        }
    };

    let points: Vec<MapPoint> = rows
        .iter()
        .map(|row| MapPoint {
            hash: row.get(0),
            lon: row.get(1),
            lat: row.get(2),
            created_at: row.get(3),
            place: row.get(4),
            starred: row.get(5),
            device_id: row.get(6),
            has_thumbnail: row.get(7),
        })
        .collect();
    let total = total as usize;

    info!("Returned {} of {} map points for user {}", points.len(), total, claims.user_id);

    Ok(HttpResponse::Ok().json(MapPointsResponse { points, total }))
}
