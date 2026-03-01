use actix_web::{ get, web, HttpResponse, HttpRequest };
use serde::Serialize;

use crate::config::Config;
use crate::db::MainDbPool;
use crate::utils;

use utoipa::ToSchema;

#[derive(Serialize, ToSchema)]
pub struct PoolStatsResponse {
    pub main_pool: PoolMetrics,
}

#[derive(Serialize, ToSchema)]
pub struct PoolMetrics {
    pub size: usize,
    pub available: isize,
    pub max_size: usize,
    pub utilization_percent: f32,
}

/// Get database connection pool statistics
/// Only accessible to admin users for monitoring purposes
#[utoipa::path(
    get,
    path = "/api/pool-stats",
    responses(
        (status = 200, description = "Pool statistics", body = PoolStatsResponse),
        (status = 401, description = "Unauthorized"),
        (status = 403, description = "Forbidden - Admin only"),
        (status = 500, description = "Internal server error")
    )
)]
#[get("/pool-stats")]
pub async fn get_pool_stats(
    req: HttpRequest,
    pool: web::Data<MainDbPool>,
    config: web::Data<Config>
) -> Result<HttpResponse, actix_web::Error> {
    let claims = match utils::authenticate_request(&req, "get_pool_stats", config.get_api_key()) {
        Ok(claims) => claims,
        Err(response) => return Ok(response),
    };

    // Only admin users can view pool stats
    if claims.role != "admin" {
        return Ok(HttpResponse::Forbidden().json(serde_json::json!({
            "status": "error",
            "message": "Admin access required"
        })));
    }

    // Get pool status
    let status = pool.0.status();
    let max_size = config.db_pool_max_size;
    let size = status.size;
    let available = status.available;
    // Calculate utilization: (total - available) / max * 100
    // available can be negative if there are more waiters than connections
    let in_use = if available < 0 {
        size
    } else {
        size.saturating_sub(available as usize)
    };
    let utilization = if max_size > 0 {
        (in_use as f32 / max_size as f32) * 100.0
    } else {
        0.0
    };

    log::debug!(
        "Pool stats requested - Size: {}, Available: {}, Max: {}, Utilization: {:.1}%",
        size, available, max_size, utilization
    );

    let stats = PoolStatsResponse {
        main_pool: PoolMetrics {
            size,
            available,
            max_size,
            utilization_percent: utilization,
        },
    };

    Ok(HttpResponse::Ok().json(stats))
}
