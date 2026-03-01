use std::time::Duration;
use actix_web::web;
use tokio::time;

use crate::db::{MainDbPool, GeotaggingDbPool};
use crate::metrics;

pub async fn start_metrics_collector(
    main_pool: web::Data<MainDbPool>,
    _geo_pool: web::Data<GeotaggingDbPool>, // Kept for future use
    config: web::Data<crate::config::Config>,
) {
    let mut interval = time::interval(Duration::from_secs(15));

    loop {
        interval.tick().await;

        // 1. Collect DB Pool Metrics
        collect_pool_metrics(&main_pool, config.db_pool_max_size);
    }
}

fn collect_pool_metrics(pool: &web::Data<MainDbPool>, max_size: usize) {
    let status = pool.0.status();
    let size = status.size;
    let available = status.available; // Available connections
    
    // "Active" (in use) = Size - Available. 
    // Note: deadpool 'available' can be negative if we are waiting for connections, 
    // but for graphing we usually want "in use" bounded by size.
    // If available < 0, it means we have a backlog, so effectively all 'size' are in use + waiters.
    // We'll treat "active" as actual connections doing work.
    let active = if available < 0 {
        size
    } else {
        size.saturating_sub(available as usize)
    };

    metrics::DB_POOL_SIZE.set(size as i64);
    metrics::DB_POOL_AVAILABLE.set(available as i64); // Gauge allows negative
    metrics::DB_POOL_ACTIVE.set(active as i64);
    metrics::DB_POOL_MAX_SIZE.set(max_size as i64);
    
    // Utilization %
    if max_size > 0 {
        let util = (active as f64 / max_size as f64) * 100.0;
        metrics::DB_POOL_UTILIZATION.set(util as i64); // Casting to i64 for IntGauge, or we change to Gauge (float)
    }
}
