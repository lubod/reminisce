use actix_web::{get, web, HttpRequest, HttpResponse};
use log::{info, warn};
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

use crate::config::Config;
use crate::db::GeotaggingDbPool;
use crate::utils;

/// Result from geocoding a place name to coordinates
#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct GeocodeResult {
    /// Place name (e.g., "London", "Paris")
    pub name: String,
    /// Latitude in decimal degrees
    pub latitude: f64,
    /// Longitude in decimal degrees
    pub longitude: f64,
    /// OSM admin level (2=country, 4=state, 6=county, 8=city)
    pub admin_level: i32,
    /// ISO country code (e.g., "GB", "FR")
    pub country_code: Option<String>,
    /// Full display name with hierarchy (e.g., "London, England, GB")
    pub display_name: String,
}

/// Query parameters for place search
#[derive(Deserialize, ToSchema)]
pub struct PlaceSearchQuery {
    /// Search query string (minimum 2 characters)
    pub query: String,
    /// Maximum number of results to return (default: 10)
    #[serde(default = "default_limit")]
    pub limit: i64,
}

fn default_limit() -> i64 {
    20
}

/// Forward geocoding: Convert place name to coordinates
/// Searches the admin_boundaries table for matching place names
pub async fn geocode_place_name(
    place_query: &str,
    geotagging_pool: &web::Data<GeotaggingDbPool>,
    limit: i64,
) -> Result<Vec<GeocodeResult>, String> {
    // Validate input
    if place_query.trim().len() < 2 {
        return Err("Search query must be at least 2 characters".to_string());
    }

    let client = match geotagging_pool.0.get().await {
        Ok(client) => client,
        Err(e) => {
            warn!("Failed to get database client for geocoding: {}", e);
            return Err(format!("Database connection error: {}", e));
        }
    };

    // Search admin_boundaries table for matching place names
    // Use ILIKE for case-insensitive partial matching
    // Calculate centroid of boundary polygons to get representative coordinates
    // Admin levels: 2=Country, 4=State/Province, 6=County/District, 8=City, 10=Locality
    let query = r#"
        SELECT
            b.name,
            ST_Y(ST_Centroid(b.geometry::geometry)) as latitude,
            ST_X(ST_Centroid(b.geometry::geometry)) as longitude,
            b.admin_level,
            b.country_code,
            (
                SELECT string_agg(p_name, ', ')
                FROM (
                    SELECT name as p_name
                    FROM admin_boundaries
                    WHERE ST_Intersects(geometry, ST_Centroid(b.geometry))
                    ORDER BY admin_level DESC
                ) p
            ) as display_name
        FROM admin_boundaries b
        WHERE
            b.name ILIKE $1
        ORDER BY
            LENGTH(b.name) ASC,  -- Prioritize exact/short matches ("Paris" before "Paristown")
            b.admin_level ASC    -- Prioritize higher levels (Country 2 > State 4 > City 8 > Village 10)
        LIMIT $2
    "#;

    // Add wildcards for partial matching
    let search_pattern = format!("%{}%", place_query.trim());

    match client
        .query(query, &[&search_pattern, &limit])
        .await
    {
        Ok(rows) => {
            let results: Vec<GeocodeResult> = rows
                .iter()
                .map(|row| {
                    let name: String = row.get("name");
                    let latitude: f64 = row.get("latitude");
                    let longitude: f64 = row.get("longitude");
                    let admin_level: i32 = row.get("admin_level");
                    let country_code: Option<String> = row.get("country_code");
                    let display_name: String = row.get::<_, Option<String>>("display_name")
                        .unwrap_or_else(|| name.clone());

                    GeocodeResult {
                        name,
                        latitude,
                        longitude,
                        admin_level,
                        country_code,
                        display_name,
                    }
                })
                .collect();

            info!(
                "Geocoding query '{}' returned {} results",
                place_query,
                results.len()
            );
            Ok(results)
        }
        Err(e) => {
            warn!("Failed to query admin_boundaries for '{}': {}", place_query, e);
            Err(format!("Database query error: {}", e))
        }
    }
}

/// API endpoint for place name autocomplete
/// GET /api/search/places?query={text}&limit={num}
#[utoipa::path(
    get,
    path = "/api/search/places",
    params(
        ("query" = String, Query, description = "Place name to search for"),
        ("limit" = Option<i64>, Query, description = "Maximum number of results (default: 10)")
    ),
    responses(
        (status = 200, description = "List of matching places", body = Vec<GeocodeResult>),
        (status = 400, description = "Invalid query (too short)"),
        (status = 401, description = "Unauthorized"),
        (status = 500, description = "Internal server error")
    ),
    security(
        ("bearer" = [])
    ),
    tag = "search"
)]
#[get("/search/places")]
pub async fn search_places(
    req: HttpRequest,
    query: web::Query<PlaceSearchQuery>,
    geotagging_pool: web::Data<GeotaggingDbPool>,
    config: web::Data<Config>,
) -> Result<HttpResponse, actix_web::Error> {
    // Authenticate request
    let _claims = match utils::authenticate_request(&req, "search_places", config.get_api_key()) {
        Ok(claims) => claims,
        Err(response) => return Ok(response),
    };

    // Validate query length
    if query.query.trim().len() < 2 {
        return Ok(HttpResponse::BadRequest().json(serde_json::json!({
            "error": "Search query must be at least 2 characters"
        })));
    }

    // Perform geocoding
    match geocode_place_name(&query.query, &geotagging_pool, query.limit).await {
        Ok(results) => Ok(HttpResponse::Ok().json(results)),
        Err(e) => Ok(HttpResponse::InternalServerError().json(serde_json::json!({
            "error": e
        }))),
    }
}
