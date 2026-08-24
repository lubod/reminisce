use deadpool_postgres::{Pool, Runtime, PoolConfig, Timeouts};
use tokio_postgres::Config as PgConfig;
use std::str::FromStr;
use deadpool_postgres::Manager as PgManager;
use std::time::Duration;
use log::{error, info};

/// Configuration options for database connection pool
#[derive(Clone)]
pub struct DbPoolOptions {
    pub max_size: usize,
    pub timeout_secs: u64,
}

impl Default for DbPoolOptions {
    fn default() -> Self {
        Self {
            max_size: 16,
            timeout_secs: 30,
        }
    }
}

pub fn create_pool(postgres_url: &str) -> Result<Pool, Box<dyn std::error::Error>> {
    create_pool_with_options(postgres_url, DbPoolOptions::default(), false)
}

pub fn create_pool_with_options(
    postgres_url: &str,
    options: DbPoolOptions,
    use_tls: bool,
) -> Result<Pool, Box<dyn std::error::Error>> {
    let pg_config = PgConfig::from_str(postgres_url)?;

    // Configure pool with explicit settings
    let mut pool_config = PoolConfig::new(options.max_size);
    pool_config.timeouts = Timeouts {
        wait: Some(Duration::from_secs(options.timeout_secs)),
        create: Some(Duration::from_secs(options.timeout_secs)),
        recycle: Some(Duration::from_secs(options.timeout_secs)),
    };

    let pool = if use_tls {
        let tls_connector = native_tls::TlsConnector::builder().build()?;
        let connector = postgres_native_tls::MakeTlsConnector::new(tls_connector);
        let manager = PgManager::new(pg_config, connector);
        Pool::builder(manager)
            .config(pool_config)
            .runtime(Runtime::Tokio1)
            .build()?
    } else {
        let manager = PgManager::new(pg_config, tokio_postgres::NoTls);
        Pool::builder(manager)
            .config(pool_config)
            .runtime(Runtime::Tokio1)
            .build()?
    };

    info!(
        "Database connection pool configured: max={}, timeout={}s",
        options.max_size, options.timeout_secs
    );

    Ok(pool)
}

/// Split a SQL script into individual statements, splitting on semicolons while
/// respecting `$$`-quoted blocks, single-quoted string literals (including `''`
/// escapes), `--` line comments, and `/* ... */` block comments (nestable, as in
/// PostgreSQL). Comments are stripped from the output.
fn split_sql_statements(sql: &str) -> Vec<String> {
    let mut statements: Vec<String> = Vec::new();
    let mut current = String::new();
    let mut in_dollar = false;
    let mut in_single_quote = false;
    let mut block_comment_depth = 0usize;
    let chars: Vec<char> = sql.chars().collect();
    let mut i = 0usize;

    while i < chars.len() {
        let c = chars[i];

        // Inside a /* ... */ comment: track nesting, emit nothing.
        if block_comment_depth > 0 {
            if c == '/' && i + 1 < chars.len() && chars[i + 1] == '*' {
                block_comment_depth += 1;
                i += 2;
                continue;
            }
            if c == '*' && i + 1 < chars.len() && chars[i + 1] == '/' {
                block_comment_depth -= 1;
                i += 2;
                continue;
            }
            i += 1;
            continue;
        }

        // Line comment (outside quotes): drop through to end of line.
        if c == '-' && i + 1 < chars.len() && chars[i + 1] == '-' && !in_dollar && !in_single_quote {
            while i < chars.len() && chars[i] != '\n' {
                i += 1;
            }
            continue;
        }

        // Block comment open (outside quotes).
        if c == '/' && i + 1 < chars.len() && chars[i + 1] == '*' && !in_dollar && !in_single_quote {
            block_comment_depth = 1;
            i += 2;
            continue;
        }

        // Dollar-quote open/close ($$).
        if !in_dollar && c == '$' && i + 1 < chars.len() && chars[i + 1] == '$' {
            in_dollar = true;
            current.push_str("$$");
            i += 2;
            continue;
        }
        if in_dollar && c == '$' && i + 1 < chars.len() && chars[i + 1] == '$' {
            in_dollar = false;
            current.push_str("$$");
            i += 2;
            continue;
        }

        // Single-quote (open/close), honoring the '' escape.
        if !in_dollar && c == '\'' {
            if in_single_quote && i + 1 < chars.len() && chars[i + 1] == '\'' {
                current.push_str("''");
                i += 2;
                continue;
            }
            in_single_quote = !in_single_quote;
            current.push(c);
            i += 1;
            continue;
        }

        // Statement terminator (only outside quotes/comments).
        if c == ';' && !in_dollar && !in_single_quote {
            let statement = current.trim();
            if !statement.is_empty() {
                statements.push(statement.to_string());
            }
            current.clear();
            i += 1;
            continue;
        }

        current.push(c);
        i += 1;
    }

    // Keep any trailing statement without a final semicolon.
    let statement = current.trim();
    if !statement.is_empty() {
        statements.push(statement.to_string());
    }

    statements
}

/// Execute each statement of a script sequentially. The first failing statement
/// aborts the script with a readable error (including the statement index) so a
/// partially-applied migration is never silently recorded as done.
async fn exec_sql_script(client: &deadpool_postgres::Object, sql: &str, label: &str) -> Result<usize, String> {
    let statements = split_sql_statements(sql);
    for (idx, statement) in statements.iter().enumerate() {
        client.execute(statement, &[]).await.map_err(|e| {
            format!(
                "[{}] statement {} failed: {}\nStatement: {}",
                label,
                idx + 1,
                e,
                statement
            )
        })?;
    }
    Ok(statements.len())
}

/// Execute a versioned migration inside a single transaction: the whole file is
/// applied atomically (`BEGIN; ...; COMMIT`) or rolled back entirely on the first
/// failing statement. The error message includes the failed statement index.
async fn exec_migration_script(client: &deadpool_postgres::Object, sql: &str, version: &str) -> Result<usize, String> {
    let statements = split_sql_statements(sql);

    client.execute("BEGIN", &[]).await.map_err(|e| {
        format!("[{}] BEGIN failed: {}", version, e)
    })?;

    for (idx, statement) in statements.iter().enumerate() {
        match client.execute(statement, &[]).await {
            Ok(_) => {}
            Err(e) => {
                let rollback_result = client.execute("ROLLBACK", &[]).await;
                return Err(format!(
                    "[{}] statement {} failed (transaction rolled back{}): {}\nStatement: {}",
                    version,
                    idx + 1,
                    match rollback_result {
                        Ok(_) => "".to_string(),
                        Err(re) => format!(", ROLLBACK also failed: {}", re),
                    },
                    e,
                    statement
                ));
            }
        }
    }

    client.execute("COMMIT", &[]).await.map_err(|e| {
        format!("[{}] COMMIT failed: {}", version, e)
    })?;

    Ok(statements.len())
}

/// Run init.sql against the pool at startup.
/// All statements use IF NOT EXISTS / ADD COLUMN IF NOT EXISTS, so this is idempotent.
/// Then applies any numbered migrations from db/migrations/ that haven't run yet.
pub async fn run_migrations(pool: &Pool) -> Result<(), Box<dyn std::error::Error>> {
    let init_sql = include_str!("../db/init.sql");
    run_migrations_with_schema(pool, init_sql).await
}

/// Same as run_migrations but allows providing a custom schema string (useful for tests).
pub async fn run_migrations_with_schema(pool: &Pool, init_sql: &str) -> Result<(), Box<dyn std::error::Error>> {
    let client = pool.get().await?;

    // --- Base schema (idempotent, runs every startup) ---
    // init.sql is deliberately lenient: everything is guarded (IF NOT EXISTS etc.)
    // and re-runs on every boot, so a failure here is logged, not aborting.
    match exec_sql_script(&client, init_sql, "init.sql").await {
        Ok(n) => info!("DB init.sql: {} statements applied", n),
        Err(msg) => {
            // A failed statement aborts the remaining script; surface how far it
            // got (the failing index is in `msg`) at ERROR level.
            let applied = msg
                .split("statement ")
                .nth(1)
                .and_then(|rest| rest.split(' ').next())
                .and_then(|idx| idx.parse::<usize>().ok())
                .map(|idx| idx.saturating_sub(1))
                .unwrap_or(0);
            error!(
                "DB init.sql error after {} statements applied (continuing, idempotent): {}",
                applied, msg
            );
        }
    }

    // Ensure dynamic sharding columns exist for backward compatibility
    let _ = client.execute("ALTER TABLE images ADD COLUMN IF NOT EXISTS p2p_data_shards INTEGER DEFAULT 3", &[]).await;
    let _ = client.execute("ALTER TABLE images ADD COLUMN IF NOT EXISTS p2p_parity_shards INTEGER DEFAULT 2", &[]).await;
    let _ = client.execute("ALTER TABLE videos ADD COLUMN IF NOT EXISTS p2p_data_shards INTEGER DEFAULT 3", &[]).await;
    let _ = client.execute("ALTER TABLE videos ADD COLUMN IF NOT EXISTS p2p_parity_shards INTEGER DEFAULT 2", &[]).await;

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
        ("004", include_str!("../db/migrations/004_multi_tenancy.sql")),
        ("005", include_str!("../db/migrations/005_add_segmented_sharding.sql")),
        ("006", include_str!("../db/migrations/006_backfill_orientation.sql")),
        ("007", include_str!("../db/migrations/007_add_orientation_detection.sql")),
        ("008", include_str!("../db/migrations/008_metric_series.sql")),
        ("009", include_str!("../db/migrations/009_replication_attempt.sql")),
        ("010", include_str!("../db/migrations/010_ai_poison.sql")),
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
        // A failed migration is FATAL: do not record the version and fail
        // startup — otherwise a broken upgrade is recorded as applied and the
        // schema is left inconsistent while the server keeps running. The whole
        // migration file runs in one transaction (BEGIN/COMMIT) and is rolled
        // back on the first failing statement.
        let n = exec_migration_script(&client, sql, version).await.map_err(|msg| {
            std::io::Error::other(format!("Database migration {} failed: {}", version, msg))
        })?;
        info!("Migration {}: {} statements applied", version, n);

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

