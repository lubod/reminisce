use deadpool_postgres::{Pool, Runtime, PoolConfig, Timeouts};
use tokio_postgres::NoTls;
use tokio_postgres::Config as PgConfig;
use std::str::FromStr;
use deadpool_postgres::Manager as PgManager;
use std::time::Duration;
use log::{info, warn};

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

/// Execute a SQL script, splitting on semicolons while respecting $$-quoted blocks.
async fn exec_sql_script(client: &deadpool_postgres::Object, sql: &str, label: &str) -> (usize, usize) {
    let mut ok = 0usize;
    let mut errs = 0usize;
    let mut current = String::new();
    let mut in_dollar_quote = false;
    for line in sql.lines() {
        let dollar_count = line.matches("$$").count();
        if dollar_count % 2 != 0 {
            in_dollar_quote = !in_dollar_quote;
        }
        current.push_str(line);
        current.push('\n');
        if !in_dollar_quote && line.trim_end().ends_with(';') {
            let trimmed = current.trim().to_string();
            current.clear();
            if trimmed.is_empty() || trimmed.starts_with("--") {
                continue;
            }
            match client.execute(trimmed.as_str(), &[]).await {
                Ok(_) => ok += 1,
                Err(e) => {
                    warn!("[{}] statement warning: {}", label, e);
                    errs += 1;
                }
            }
        }
    }
    (ok, errs)
}

/// Run init.sql against the pool at startup.
/// All statements use IF NOT EXISTS / ADD COLUMN IF NOT EXISTS, so this is idempotent.
/// Then applies any numbered migrations from db/migrations/ that haven't run yet.
pub async fn run_migrations(pool: &Pool) -> Result<(), Box<dyn std::error::Error>> {
    let client = pool.get().await?;

    // --- Base schema (idempotent, runs every startup) ---
    let init_sql = include_str!("../db/init.sql");
    let (ok, errs) = exec_sql_script(&client, init_sql, "init.sql").await;
    if errs > 0 {
        warn!("DB init.sql: {} ok, {} warnings", ok, errs);
    } else {
        info!("DB init.sql: {} statements applied", ok);
    }

    // --- Versioned migrations (each runs exactly once) ---
    client.execute(
        "CREATE TABLE IF NOT EXISTS schema_migrations (
            version VARCHAR(255) PRIMARY KEY,
            applied_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
        )",
        &[],
    ).await?;

    // Migrations embedded at compile time: (version, sql)
    let migrations: &[(&str, &str)] = &[
        ("001", include_str!("../db/migrations/001_fix_partial_indexes_deleted_at.sql")),
        ("002", include_str!("../db/migrations/002_add_duplicate_pairs.sql")),
        ("003", include_str!("../db/migrations/003_add_orientation_column.sql")),
    ];

    for (version, sql) in migrations {
        let already_applied = client
            .query_opt("SELECT 1 FROM schema_migrations WHERE version = $1", &[version])
            .await?
            .is_some();

        if already_applied {
            continue;
        }

        info!("Applying migration {}...", version);
        let (ok, errs) = exec_sql_script(&client, sql, version).await;
        if errs > 0 {
            warn!("Migration {}: {} ok, {} warnings", version, ok, errs);
        } else {
            info!("Migration {}: {} statements applied", version, ok);
        }

        client.execute(
            "INSERT INTO schema_migrations (version) VALUES ($1)",
            &[version],
        ).await?;
    }

    Ok(())
}

// Wrapper types to distinguish between different database pools in dependency injection
#[derive(Clone)]
pub struct MainDbPool(pub Pool);

#[derive(Clone)]
pub struct GeotaggingDbPool(pub Pool);

