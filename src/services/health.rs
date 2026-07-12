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
    #[serde(skip_serializing_if = "Option::is_none")]
    pub database: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub geotagging_database: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ai_service: Option<String>,
    pub timestamp: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub backup: Option<serde_json::Value>,
}

#[utoipa::path(
    get,
    path = "/api/health",
    responses(
        (status = 200, description = "Service is healthy", body = HealthCheckResponse),
        (status = 503, description = "Service is unhealthy", body = HealthCheckResponse)
    ),
    tag = "Health"
)]
#[get("/health")]
pub async fn health_check(
    req: actix_web::HttpRequest,
    main_pool: web::Data<MainDbPool>,
    geo_pool: web::Data<GeotaggingDbPool>,
    config: web::Data<Config>,
) -> impl Responder {
    // Check if the requester is authenticated as admin
    let is_admin = match crate::auth_utils::authenticate_request(&req, "health_check", config.get_api_key()).await {
        Ok(claims) => claims.role == "admin",
        Err(_) => false,
    };

    let database = match main_pool.0.get().await {
        Ok(c) => match c.query_one("SELECT 1", &[]).await {
            Ok(_) => "connected".to_string(),
            Err(e) => {
                log::error!("Database health query failed: {}", e);
                "unhealthy".to_string()
            }
        },
        Err(e) => {
            log::error!("Database health connection failed: {}", e);
            "unhealthy".to_string()
        }
    };

    let geotagging_database = match geo_pool.0.get().await {
        Ok(c) => match c.query_one("SELECT 1", &[]).await {
            Ok(_) => "connected".to_string(),
            Err(e) => {
                log::error!("Geotagging database health query failed: {}", e);
                "unhealthy".to_string()
            }
        },
        Err(e) => {
            log::error!("Geotagging database health connection failed: {}", e);
            "unhealthy".to_string()
        }
    };

    let ai_url = format!("{}/health", config.embedding_service_url.trim_end_matches('/'));
    let ai_service = match tokio::time::timeout(
        std::time::Duration::from_secs(10),
        reqwest::get(&ai_url),
    ).await {
        Ok(Ok(r)) if r.status().is_success() => "connected".to_string(),
        Ok(Ok(r)) => {
            log::error!("AI service health check returned error: {}", r.status());
            "unhealthy".to_string()
        }
        Ok(Err(e)) => {
            log::error!("AI service health check connection failed: {}", e);
            "unhealthy".to_string()
        }
        Err(_) => {
            log::error!("AI service health check timeout");
            "timeout".to_string()
        }
    };

    let healthy = database == "connected"
        && geotagging_database == "connected"
        && ai_service == "connected";

    let body = if is_admin {
        HealthCheckResponse {
            status: if healthy { "healthy" } else { "unhealthy" }.to_string(),
            database: Some(database),
            geotagging_database: Some(geotagging_database),
            ai_service: Some(ai_service),
            timestamp: chrono::Utc::now().to_rfc3339(),
            backup: None,
        }
    } else {
        HealthCheckResponse {
            status: if healthy { "healthy" } else { "unhealthy" }.to_string(),
            database: None,
            geotagging_database: None,
            ai_service: None,
            timestamp: chrono::Utc::now().to_rfc3339(),
            backup: None,
        }
    };

    if healthy {
        HttpResponse::Ok().json(body)
    } else {
        HttpResponse::ServiceUnavailable().json(body)
    }
}
