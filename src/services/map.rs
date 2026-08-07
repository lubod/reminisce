//! Map view support: return geotagged media (images only — videos carry no
//! location) as lightweight map points for client-side clustering.

use actix_web::{ get, web, HttpResponse, HttpRequest };
use serde::{Deserialize, Serialize};
use log::{error, info};
use utoipa::ToSchema;

use crate::config::Config;
use crate::db::MainDbPool;
use crate::utils;

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
}

#[utoipa::path(
    get,
    path = "/map/media",
    params(
        ("starred_only" = Option<bool>, Query, description = "Only starred media"),
        ("label_id" = Option<i32>, Query, description = "Only media carrying this label"),
        ("start_date" = Option<String>, Query, description = "created_at >= date"),
        ("end_date" = Option<String>, Query, description = "created_at < date+1d"),
        ("device_id" = Option<String>, Query, description = "Only media from this device")
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

    let where_clause = conditions.join(" AND ");

    let sql = format!(
        "SELECT i.hash, ST_X(i.location::geometry) AS lon, ST_Y(i.location::geometry) AS lat, \
                i.created_at, i.place, \
                CASE WHEN s.hash IS NOT NULL THEN true ELSE false END AS starred, \
                i.deviceid, i.has_thumbnail \
         FROM images i \
         LEFT JOIN starred_images s ON i.hash = s.hash AND s.user_id = $1 \
         WHERE {} \
         ORDER BY i.created_at DESC",
        where_clause
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

    let rows = match client.query(&sql, &params).await {
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

    let total = points.len();
    info!("Returned {} map points for user {}", total, claims.user_id);

    Ok(HttpResponse::Ok().json(MapPointsResponse { points, total }))
}
