use actix_web::{ get, web, HttpResponse, HttpRequest };
use serde::Serialize;
use log::{error, info};

use crate::config::Config;
use crate::db::GeotaggingDbPool;
use crate::utils;

use utoipa::ToSchema;

#[derive(Serialize, ToSchema)]
pub struct GeoDbStatsResponse {
    pub total_boundaries: i64,
    pub countries: i64,           // admin_level = 2
    pub states_provinces: i64,     // admin_level = 4
    pub counties: i64,             // admin_level = 6
    pub cities: i64,               // admin_level = 8
    pub other_boundaries: i64,     // other admin levels
    pub unique_countries: i64,     // unique country codes
}

/// Get geotagging database statistics
/// Shows the size and composition of the reverse geocoding database
/// Only accessible to admin users
#[utoipa::path(
    get,
    path = "/api/geodb-stats",
    responses(
        (status = 200, description = "Geo database statistics", body = GeoDbStatsResponse),
        (status = 401, description = "Unauthorized"),
        (status = 403, description = "Forbidden - Admin only"),
        (status = 500, description = "Internal server error")
    )
)]
#[get("/geodb-stats")]
pub async fn get_geodb_stats(
    req: HttpRequest,
    pool: web::Data<GeotaggingDbPool>,
    config: web::Data<Config>
) -> Result<HttpResponse, actix_web::Error> {
    let claims = match utils::authenticate_request(&req, "get_geodb_stats", config.get_api_key()) {
        Ok(claims) => claims,
        Err(response) => return Ok(response),
    };

    // Only admin users can view geo database stats
    if claims.role != "admin" {
        return Ok(HttpResponse::Forbidden().json(serde_json::json!({
            "status": "error",
            "message": "Admin access required"
        })));
    }

    let client = utils::get_db_client(&pool.0).await?;

    // Combine all stats into a single query for better performance
    let query = "
        SELECT
            (SELECT COUNT(*) FROM admin_boundaries) as total_boundaries,
            (SELECT COUNT(*) FROM admin_boundaries WHERE admin_level = 2) as countries,
            (SELECT COUNT(*) FROM admin_boundaries WHERE admin_level = 4) as states_provinces,
            (SELECT COUNT(*) FROM admin_boundaries WHERE admin_level = 6) as counties,
            (SELECT COUNT(*) FROM admin_boundaries WHERE admin_level = 8) as cities,
            (SELECT COUNT(*) FROM admin_boundaries WHERE admin_level NOT IN (2, 4, 6, 8)) as other_boundaries,
            (SELECT COUNT(DISTINCT country_code) FROM admin_boundaries WHERE country_code IS NOT NULL) as unique_countries
    ";

    let row = client.query_one(query, &[]).await.map_err(|e| {
        error!("Failed to query geodb stats: {}", e);
        actix_web::error::ErrorInternalServerError("Failed to retrieve geo database statistics")
    })?;

    let stats = GeoDbStatsResponse {
        total_boundaries: row.get(0),
        countries: row.get(1),
        states_provinces: row.get(2),
        counties: row.get(3),
        cities: row.get(4),
        other_boundaries: row.get(5),
        unique_countries: row.get(6),
    };

    info!(
        "Geo database stats: {} total boundaries, {} unique countries",
        stats.total_boundaries, stats.unique_countries
    );

    Ok(HttpResponse::Ok().json(stats))
}
