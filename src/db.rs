use deadpool_postgres::{Pool, Runtime, PoolConfig, Timeouts};
use tokio_postgres::NoTls;
use tokio_postgres::Config as PgConfig;
use std::str::FromStr;
use deadpool_postgres::Manager as PgManager;
use std::time::Duration;
use log::info;

/// Configuration options for database connection pool
#[derive(Clone)]
pub struct DbPoolOptions {
    pub max_size: usize,
    pub min_size: usize,
    pub timeout_secs: u64,
}

impl Default for DbPoolOptions {
    fn default() -> Self {
        Self {
            max_size: 16,
            min_size: 4,
            timeout_secs: 30,
        }
    }
}

pub fn create_pool(postgres_url: &str) -> Result<Pool, Box<dyn std::error::Error>> {
    create_pool_with_options(postgres_url, DbPoolOptions::default())
}

pub fn create_pool_with_options(
    postgres_url: &str,
    options: DbPoolOptions,
) -> Result<Pool, Box<dyn std::error::Error>> {
    let pg_config = PgConfig::from_str(&postgres_url)?;
    let manager = PgManager::new(pg_config, NoTls);

    // Configure pool with explicit settings
    let mut pool_config = PoolConfig::new(options.max_size);
    pool_config.timeouts = Timeouts {
        wait: Some(Duration::from_secs(options.timeout_secs)),
        create: Some(Duration::from_secs(options.timeout_secs)),
        recycle: Some(Duration::from_secs(options.timeout_secs)),
    };

    let pool = Pool::builder(manager)
        .config(pool_config)
        .runtime(Runtime::Tokio1)
        .build()?;

    info!(
        "Database connection pool configured: max={}, timeout={}s",
        options.max_size, options.timeout_secs
    );

    Ok(pool)
}

// Wrapper types to distinguish between different database pools in dependency injection
#[derive(Clone)]
pub struct MainDbPool(pub Pool);

#[derive(Clone)]
pub struct GeotaggingDbPool(pub Pool);

