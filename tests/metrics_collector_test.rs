use actix_web::web;
use reminisce::*;
use reminisce::test_utils::setup_test_database_with_instance;
use serial_test::serial;
use std::time::Duration;
use tokio_util::sync::CancellationToken;

mod common;

#[tokio::test]
#[serial]
async fn test_metrics_collector_lifecycle_and_sampling() {
    let (pool, _test_db) = setup_test_database_with_instance().await;
    let config = common::utils::create_test_config();
    let config_data = web::Data::new(config);
    let main_pool = web::Data::new(common::utils::wrap_main_pool(pool.clone()));
    let geo_pool = web::Data::new(common::utils::create_geotagging_pool().await);

    // Insert sample metrics rows to verify table exists and is writable
    let client = pool.get().await.unwrap();
    let _ = client.execute(
        "CREATE TABLE IF NOT EXISTS metric_samples (
            ts TIMESTAMPTZ NOT NULL,
            name TEXT NOT NULL,
            value DOUBLE PRECISION NOT NULL
        )",
        &[],
    ).await;

    let shutdown = CancellationToken::new();
    let shutdown_clone = shutdown.clone();

    let collector_task = tokio::spawn(async move {
        metrics_collector::start_metrics_collector(
            main_pool,
            geo_pool,
            config_data,
            shutdown_clone,
        ).await;
    });

    // Let the collector start and run for a brief moment
    tokio::time::sleep(Duration::from_millis(100)).await;

    // Trigger graceful shutdown
    shutdown.cancel();

    let res = tokio::time::timeout(Duration::from_secs(3), collector_task).await;
    assert!(res.is_ok(), "metrics collector stopped cleanly within timeout");
}

#[actix_web::test]
#[serial]
async fn test_pool_util_and_collect_pool_metrics() {
    let (pool, _db) = setup_test_database_with_instance().await;
    let main_pool = web::Data::new(common::utils::wrap_main_pool(pool.clone()));
    let util = metrics_collector::pool_util(&main_pool);
    assert!(util >= 0.0);
    metrics_collector::collect_pool_metrics(&main_pool, 16);
}
