use actix_web::{ get, HttpResponse, Responder, web };
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;
use crate::db::MainDbPool;

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
) -> impl Responder {
    // Check database connectivity
    match main_pool.0.get().await {
        Ok(client) => {
            match client.query_one("SELECT 1", &[]).await {
                Ok(_) => HttpResponse::Ok().json(HealthCheckResponse {
                    status: "healthy".to_string(),
                    database: "connected".to_string(),
                    timestamp: chrono::Utc::now().to_rfc3339(),
                    backup: None, // Backup status available via dedicated endpoint
                }),
                Err(e) => HttpResponse::ServiceUnavailable().json(HealthCheckResponse {
                    status: "unhealthy".to_string(),
                    database: format!("query_failed: {}", e),
                    timestamp: chrono::Utc::now().to_rfc3339(),
                    backup: None,
                })
            }
        },
        Err(e) => HttpResponse::ServiceUnavailable().json(HealthCheckResponse {
            status: "unhealthy".to_string(),
            database: format!("connection_failed: {}", e),
            timestamp: chrono::Utc::now().to_rfc3339(),
            backup: None,
        })
    }
}