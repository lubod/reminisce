use log::error;
use std::path::PathBuf;

pub use crate::system_utils::{
    WorkerConcurrencyLimits, get_load_average, get_gpu_load, get_cpu_count,
    adjust_batch_size, calculate_worker_concurrency, calculate_parallel_batch_size,
    run_worker_loop,
};
pub use crate::auth_utils::{parse_user_uuid, ensure_user_exists, authenticate_request};
pub use crate::geo_utils::{extract_gps_coordinates, reverse_geocode};
pub use crate::media_utils::{
    ExistenceCheckResult, get_subdirectory_path, check_if_exists,
    determine_image_type, determine_video_type,
    list_thumbnails, total_thumbnails,
    parse_date_from_image_name, parse_date_from_video_name,
    cleanup_temp_files, cleanup_temp_files_spawn,
};

/// Get a DB client from the pool, returning 500 on failure.
pub async fn get_db_client(pool: &deadpool_postgres::Pool) -> Result<deadpool_postgres::Client, actix_web::Error> {
    pool.get().await.map_err(|e| {
        error!("Failed to get DB client: {}", e);
        actix_web::error::ErrorInternalServerError("Database connection failed")
    })
}

/// Helper to dump DB to a file
pub fn perform_db_dump(config: &crate::config::Config) -> Result<PathBuf, String> {
    let database_url = config.database_url.as_ref().ok_or("Database URL not configured")?;

    let password = url::Url::parse(database_url)
        .ok()
        .and_then(|url| url.password().map(|p| p.to_string()))
        .unwrap_or_else(|| "postgres".to_string());

    let output_path = PathBuf::from(format!("db_dump_temp_{}.sql", chrono::Utc::now().timestamp()));

    let file = std::fs::File::create(&output_path).map_err(|e| e.to_string())?;

    let mut command = std::process::Command::new("pg_dump");
    command
        .arg("--format=plain")
        .env("PGPASSWORD", password)
        .arg(database_url)
        .stdout(file);

    match command.status() {
        Ok(status) if status.success() => Ok(output_path),
        Ok(_) => Err("pg_dump failed".to_string()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Err("pg_dump missing".to_string()),
        Err(e) => Err(e.to_string()),
    }
}

/// Parse a peer address string into a SocketAddr.
/// Handles both "ip:port" and bare "ip" (defaults to port 5050).
pub fn parse_peer_addr(peer: &str) -> Result<std::net::SocketAddr, String> {
    if let Ok(addr) = peer.parse::<std::net::SocketAddr>() {
        return Ok(addr);
    }
    format!("{}:5050", peer)
        .parse::<std::net::SocketAddr>()
        .map_err(|e| format!("Invalid peer address '{}': {}", peer, e))
}
