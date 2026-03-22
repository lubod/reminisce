use actix_web::{ get, HttpResponse, Responder, web };
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;
use crate::db::{MainDbPool, GeotaggingDbPool};
use crate::config::Config;

#[utoipa::path(
    get,
    path = "/ping",
    responses((status = 200, description = "Ping successful", body = String))
)]
#[get("/ping")]
pub async fn ping() -> impl Responder {
    HttpResponse::Ok().body("OK")
}

#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct HealthCheckResponse {
    pub status: String,
    pub database: String,
    pub geotagging_database: String,
    pub ai_service: String,
    pub timestamp: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub backup: Option<serde_json::Value>,
}

#[utoipa::path(
    get,
    path = "/health",
    responses(
        (status = 200, description = "Service is healthy", body = HealthCheckResponse),
        (status = 503, description = "Service is unhealthy", body = HealthCheckResponse)
    ),
    tag = "Health"
)]
#[get("/health")]
pub async fn health_check(
    main_pool: web::Data<MainDbPool>,
    geo_pool: web::Data<GeotaggingDbPool>,
    config: web::Data<Config>,
) -> impl Responder {
    let database = match main_pool.0.get().await {
        Ok(c) => match c.query_one("SELECT 1", &[]).await {
            Ok(_) => "connected".to_string(),
            Err(e) => format!("query_failed: {}", e),
        },
        Err(e) => format!("connection_failed: {}", e),
    };

    let geotagging_database = match geo_pool.0.get().await {
        Ok(c) => match c.query_one("SELECT 1", &[]).await {
            Ok(_) => "connected".to_string(),
            Err(e) => format!("query_failed: {}", e),
        },
        Err(e) => format!("connection_failed: {}", e),
    };

    let ai_url = format!("{}/health", config.embedding_service_url.trim_end_matches('/'));
    let ai_service = match tokio::time::timeout(
        std::time::Duration::from_secs(3),
        reqwest::get(&ai_url),
    ).await {
        Ok(Ok(r)) if r.status().is_success() => "connected".to_string(),
        Ok(Ok(r)) => format!("error: {}", r.status()),
        Ok(Err(e)) => format!("connection_failed: {}", e),
        Err(_) => "timeout".to_string(),
    };

    let healthy = database == "connected"
        && geotagging_database == "connected"
        && ai_service == "connected";

    let body = HealthCheckResponse {
        status: if healthy { "healthy" } else { "unhealthy" }.to_string(),
        database,
        geotagging_database,
        ai_service,
        timestamp: chrono::Utc::now().to_rfc3339(),
        backup: None,
    };

    if healthy {
        HttpResponse::Ok().json(body)
    } else {
        HttpResponse::ServiceUnavailable().json(body)
    }
}
