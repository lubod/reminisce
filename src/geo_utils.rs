use actix_web::web;
use log::{info, warn};

use crate::db::GeotaggingDbPool;

/// Extract GPS coordinates (latitude, longitude) from EXIF JSON.
/// Returns `(latitude, longitude)` as decimal degrees, or `None` if GPS data is absent.
pub fn extract_gps_coordinates(exif_json: &serde_json::Value) -> Option<(f64, f64)> {
    let gps_lat_str = exif_json.get("GPSLatitude")?.as_str()?;
    let gps_lon_str = exif_json.get("GPSLongitude")?.as_str()?;
    let gps_lat_ref = exif_json.get("GPSLatitudeRef")?.as_str()?;
    let gps_lon_ref = exif_json.get("GPSLongitudeRef")?.as_str()?;

    let latitude = parse_gps_coordinate(gps_lat_str, gps_lat_ref)?;
    let longitude = parse_gps_coordinate(gps_lon_str, gps_lon_ref)?;

    Some((latitude, longitude))
}

fn parse_gps_coordinate(coord_str: &str, reference: &str) -> Option<f64> {
    if coord_str.contains('/') && coord_str.contains(',') {
        let parts: Vec<&str> = coord_str.split(',').map(|s| s.trim()).collect();
        if parts.len() < 3 {
            return None;
        }
        let degrees = parse_rational(parts[0])?;
        let minutes = parse_rational(parts[1])?;
        let seconds_str = parts[2].split_whitespace().next()?;
        let seconds = parse_rational(seconds_str)?;
        let mut decimal = degrees + (minutes / 60.0) + (seconds / 3600.0);
        if reference == "S" || reference == "W" {
            decimal = -decimal;
        }
        return Some(decimal);
    }

    let cleaned = coord_str
        .replace("deg", "")
        .replace("min", "")
        .replace("sec", "")
        .replace('°', "")
        .replace('\'', "")
        .replace('"', "");

    let parts: Vec<f64> = cleaned
        .split_whitespace()
        .filter_map(|s| s.parse::<f64>().ok())
        .collect();

    if parts.len() < 2 {
        return None;
    }

    let degrees = parts[0];
    let minutes = parts[1];
    let seconds = parts.get(2).copied().unwrap_or(0.0);

    let mut decimal = degrees + (minutes / 60.0) + (seconds / 3600.0);
    if reference == "S" || reference == "W" {
        decimal = -decimal;
    }
    Some(decimal)
}

fn parse_rational(rational_str: &str) -> Option<f64> {
    let cleaned = rational_str.trim();
    if let Some(slash_pos) = cleaned.find('/') {
        let numerator: f64 = cleaned[..slash_pos].trim().parse().ok()?;
        let denominator: f64 = cleaned[slash_pos + 1..].trim().parse().ok()?;
        if denominator == 0.0 {
            return None;
        }
        Some(numerator / denominator)
    } else {
        cleaned.parse::<f64>().ok()
    }
}

/// Reverse geocode coordinates, trying local DB first then external Nominatim.
pub async fn reverse_geocode(
    latitude: f64,
    longitude: f64,
    geotagging_pool: &web::Data<GeotaggingDbPool>,
    enable_local: bool,
    enable_external_fallback: bool,
) -> Option<String> {
    if latitude.is_nan()
        || longitude.is_nan()
        || latitude < -90.0
        || latitude > 90.0
        || longitude < -180.0
        || longitude > 180.0
    {
        warn!(
            "Invalid coordinates for reverse geocoding: lat={}, lon={}",
            latitude, longitude
        );
        return None;
    }

    if enable_local {
        if let Some(place) = reverse_geocode_local(latitude, longitude, geotagging_pool).await {
            info!(
                "Local geocoding successful for ({}, {}): {}",
                latitude, longitude, place
            );
            return Some(place);
        }
        info!(
            "Local geocoding returned no result for ({}, {})",
            latitude, longitude
        );
    }

    if enable_external_fallback {
        info!("Trying external Nominatim API for ({}, {})", latitude, longitude);
        return reverse_geocode_external(latitude, longitude).await;
    }

    warn!(
        "Geocoding failed for ({}, {}): no local result and external fallback disabled",
        latitude, longitude
    );
    None
}

async fn reverse_geocode_local(
    latitude: f64,
    longitude: f64,
    geotagging_pool: &web::Data<GeotaggingDbPool>,
) -> Option<String> {
    let client = match geotagging_pool.0.get().await {
        Ok(c) => c,
        Err(e) => {
            warn!("Failed to get database client for local geocoding: {}", e);
            return None;
        }
    };

    let query = r#"
        SELECT name, admin_level, country_code
        FROM admin_boundaries
        WHERE ST_Contains(geometry, ST_SetSRID(ST_MakePoint($1, $2), 4326))
        ORDER BY admin_level DESC
    "#;

    match client.query(query, &[&longitude, &latitude]).await {
        Ok(rows) if !rows.is_empty() => {
            let mut place_parts: Vec<String> = Vec::new();
            for row in &rows {
                let name: String = row.get("name");
                let admin_level: i32 = row.get("admin_level");
                if !place_parts.contains(&name) {
                    place_parts.push(name);
                }
                info!(
                    "Found admin_level {} boundary: {}",
                    admin_level,
                    place_parts.last().unwrap()
                );
            }
            let place = place_parts.join(", ");
            info!(
                "Local geocoding successful for ({}, {}): {}",
                latitude, longitude, place
            );
            Some(place)
        }
        Ok(_) => {
            info!(
                "No matching boundary found for ({}, {}) in local database",
                latitude, longitude
            );
            None
        }
        Err(e) => {
            warn!(
                "Failed to query admin_boundaries for ({}, {}): {}",
                latitude, longitude, e
            );
            None
        }
    }
}

async fn reverse_geocode_external(latitude: f64, longitude: f64) -> Option<String> {
    let url = format!(
        "https://nominatim.openstreetmap.org/reverse?lat={}&lon={}&format=json&addressdetails=1&zoom=14",
        latitude, longitude
    );

    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(5))
        .user_agent("Reminisce/1.0")
        .build()
        .ok()?;

    match client.get(&url).send().await {
        Ok(response) if response.status().is_success() => {
            match response.json::<serde_json::Value>().await {
                Ok(json) => {
                    if let Some(display_name) =
                        json.get("display_name").and_then(|v| v.as_str())
                    {
                        info!(
                            "External geocoding successful for ({}, {}): {}",
                            latitude, longitude, display_name
                        );
                        return Some(display_name.to_string());
                    }
                    warn!(
                        "No display_name in Nominatim response for ({}, {})",
                        latitude, longitude
                    );
                }
                Err(e) => warn!(
                    "Failed to parse Nominatim response for ({}, {}): {}",
                    latitude, longitude, e
                ),
            }
        }
        Ok(response) => warn!(
            "Nominatim API returned status {} for ({}, {})",
            response.status(),
            latitude,
            longitude
        ),
        Err(e) => warn!(
            "Failed to call Nominatim API for ({}, {}): {}",
            latitude, longitude, e
        ),
    }

    None
}
