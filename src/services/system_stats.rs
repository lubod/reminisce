use actix_web::{get, web, HttpRequest, HttpResponse};
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;
use sysinfo::{System, SystemExt, CpuExt, DiskExt};
use std::sync::Arc;
use crate::config::Config;
use crate::utils;
use crate::db::MainDbPool;
use np2p::network::P2PService;

#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct SystemStatsResponse {
    pub cpu_usage_percent: f32,
    pub memory_used_gb: f32,
    pub memory_total_gb: f32,
    pub memory_usage_percent: f32,
    pub disk_used_gb: f32,
    pub disk_total_gb: f32,
    pub disk_usage_percent: f32,
    pub disk_available_gb: f32,
    pub uptime_seconds: u64,
    pub gpu_info: Option<String>,
    pub gpu_usage_percent: Option<f32>,
    pub gpu_memory_used_mb: Option<f32>,
    pub gpu_memory_total_mb: Option<f32>,
}

#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct P2PDaemonStatus {
    pub is_healthy: bool,
    pub node_id: String,
    pub active_peers: usize,
    pub blobs_stored: i64,
    pub bytes_stored: i64,
    pub bytes_uploaded: i64,
    pub files_uploaded: i64,
    pub p2p_peer_count: usize,
}

#[utoipa::path(
    get,
    path = "/api/system-stats",
    responses(
        (status = 200, description = "System resource statistics", body = SystemStatsResponse),
        (status = 401, description = "Unauthorized")
    ),
    tag = "System"
)]
#[get("/system-stats")]
pub async fn get_system_stats(
    req: HttpRequest,
    config: web::Data<Config>,
) -> Result<HttpResponse, actix_web::Error> {
    let _claims = match utils::authenticate_request(&req, "get_system_stats", config.get_api_key()) {
        Ok(claims) => claims,
        Err(response) => return Ok(response),
    };

    let mut sys = System::new_all();
    sys.refresh_all();

    // CPU Usage
    let cpu_usage = sys.global_cpu_info().cpu_usage();

    // Memory usage in GB
    let total_mem = sys.total_memory() as f32 / (1024.0 * 1024.0 * 1024.0);
    let used_mem = sys.used_memory() as f32 / (1024.0 * 1024.0 * 1024.0);
    let mem_percent = (used_mem / total_mem) * 100.0;

    // Disk usage (root partition)
    let mut total_disk = 0.0;
    let mut used_disk = 0.0;
    let mut available_disk = 0.0;

    for disk in sys.disks() {
        if disk.mount_point() == std::path::Path::new("/") || disk.mount_point().to_str().unwrap_or("").contains("data") {
            total_disk = disk.total_space() as f32 / (1024.0 * 1024.0 * 1024.0);
            available_disk = disk.available_space() as f32 / (1024.0 * 1024.0 * 1024.0);
            used_disk = total_disk - available_disk;
            break;
        }
    }
    
    let disk_percent = if total_disk > 0.0 { (used_disk / total_disk) * 100.0 } else { 0.0 };

    // GPU Load (AMD ROCm)
    let gpu_load = utils::get_gpu_load().await;
    
    let gpu_vram_total = match tokio::fs::read_to_string("/sys/class/drm/card0/device/mem_info_vram_total").await {
        Ok(s) => s.trim().parse::<f32>().unwrap_or(0.0) / (1024.0 * 1024.0), // Convert to MB
        Err(_) => 0.0,
    };
    
    let gpu_vram_used = match tokio::fs::read_to_string("/sys/class/drm/card0/device/mem_info_vram_used").await {
        Ok(s) => s.trim().parse::<f32>().unwrap_or(0.0) / (1024.0 * 1024.0), // Convert to MB
        Err(_) => 0.0,
    };

    let response = SystemStatsResponse {
        cpu_usage_percent: cpu_usage,
        memory_used_gb: used_mem,
        memory_total_gb: total_mem,
        memory_usage_percent: mem_percent,
        disk_used_gb: used_disk,
        disk_total_gb: total_disk,
        disk_usage_percent: disk_percent,
        disk_available_gb: available_disk,
        uptime_seconds: sys.uptime(),
        gpu_info: Some("AMD ROCm".to_string()),
        gpu_usage_percent: Some(gpu_load as f32),
        gpu_memory_used_mb: if gpu_vram_total > 0.0 { Some(gpu_vram_used) } else { None },
        gpu_memory_total_mb: if gpu_vram_total > 0.0 { Some(gpu_vram_total) } else { None },
    };

    Ok(HttpResponse::Ok().json(response))
}

#[utoipa::path(
    get,
    path = "/api/p2p-daemon-status",
    responses(
        (status = 200, description = "Status of the internal P2P engine", body = P2PDaemonStatus),
        (status = 401, description = "Unauthorized")
    ),
    tag = "System"
)]
#[get("/p2p-daemon-status")]
pub async fn get_p2p_daemon_status(
    req: HttpRequest,
    config: web::Data<Config>,
    p2p_service: web::Data<Arc<P2PService>>,
    pool: web::Data<MainDbPool>,
) -> Result<HttpResponse, actix_web::Error> {
    let _claims = match utils::authenticate_request(&req, "get_p2p_daemon_status", config.get_api_key()) {
        Ok(claims) => claims,
        Err(response) => return Ok(response),
    };

    let client = utils::get_db_client(&pool.0).await?;

    // Count peers
    let peer_count: i64 = client.query_one("SELECT COUNT(*) FROM p2p_nodes WHERE is_active = TRUE", &[]).await
        .map(|row| row.get(0)).unwrap_or(0);

    // Count shards
    let shard_count: i64 = client.query_one("SELECT COUNT(*) FROM p2p_shards", &[]).await
        .map(|row| row.get(0)).unwrap_or(0);

    Ok(HttpResponse::Ok().json(P2PDaemonStatus {
        is_healthy: true,
        node_id: hex::encode(p2p_service.identity().node_id()),
        active_peers: peer_count as usize,
        blobs_stored: shard_count,
        bytes_stored: 0, // In-memory tracking could be added
        bytes_uploaded: 0,
        files_uploaded: 0,
        p2p_peer_count: p2p_service.registry.len(),
    }))
}
