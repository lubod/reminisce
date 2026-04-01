//! Test utilities for the Reminisce server
//! This module provides utilities for setting up fresh, isolated test databases
//!
//! Each test gets a completely fresh database that is automatically cleaned up

use deadpool_postgres::{Config, Pool, Runtime};
use tokio_postgres::NoTls;

/// A test database instance that provides a fresh, isolated database for each test
pub struct TestDatabase {
    pub pool: Pool,
    database_name: String,
    admin_config: String,
}

impl TestDatabase {
    /// Creates a new test database with a unique name
    ///
    /// This connects to the dev PostgreSQL server and creates a brand new database for the test.
    /// Configure via environment variables:
    /// - TEST_DATABASE_URL: Connection to postgres database (e.g., postgres://user:pass@localhost/postgres)
    /// - Or use: PGHOST, PGPORT, PGUSER, PGPASSWORD (defaults to localhost:5432, connects to 'postgres' database)
    pub async fn new() -> Result<Self, Box<dyn std::error::Error>> {
        // Generate a unique database name for this test
        let database_name = format!("test_{}", uuid::Uuid::new_v4().to_string().replace("-", ""));

        // Get connection to the postgres admin database
        let admin_url = std::env::var("TEST_DATABASE_URL")
            .unwrap_or_else(|_| {
                let host = std::env::var("PGHOST").unwrap_or_else(|_| "localhost".to_string());
                let port = std::env::var("PGPORT").unwrap_or_else(|_| "5432".to_string());
                let user = std::env::var("PGUSER").unwrap_or_else(|_| "postgres".to_string());
                let password = std::env::var("PGPASSWORD").unwrap_or_else(|_| "postgres".to_string());

                format!("postgres://{}:{}@{}:{}/postgres", user, password, host, port)
            });

        // Connect to postgres database to create the test database
        let admin_config: tokio_postgres::Config = admin_url.parse()
            .map_err(|e| format!("Failed to parse database URL: {}", e))?;

        let mut cfg = Config::new();
        cfg.host = admin_config.get_hosts().get(0).map(|h| {
            match h {
                tokio_postgres::config::Host::Tcp(s) => s.clone(),
                #[cfg(unix)]
                tokio_postgres::config::Host::Unix(p) => p.to_string_lossy().to_string(),
            }
        });
        cfg.port = admin_config.get_ports().get(0).copied().map(|p| p as u16);
        cfg.user = admin_config.get_user().map(|s| s.to_string());
        cfg.password = admin_config.get_password().map(|p| String::from_utf8_lossy(p).to_string());
        cfg.dbname = Some("postgres".to_string());

        let admin_pool = cfg.create_pool(Some(Runtime::Tokio1), NoTls)
            .map_err(|e| format!("Failed to create admin pool: {}", e))?;

        let admin_client = admin_pool.get().await
            .map_err(|e| format!("Failed to get admin client: {}", e))?;

        // Create the new test database
        admin_client.execute(&format!("CREATE DATABASE {}", database_name), &[]).await
            .map_err(|e| format!("Failed to create test database '{}': {}", database_name, e))?;

        // Now connect to the new test database
        let mut test_cfg = Config::new();
        test_cfg.host = cfg.host.clone();
        test_cfg.port = cfg.port;
        test_cfg.user = cfg.user.clone();
        test_cfg.password = cfg.password.clone();
        test_cfg.dbname = Some(database_name.clone());

        let pool = test_cfg.create_pool(Some(Runtime::Tokio1), NoTls)
            .map_err(|e| format!("Failed to create test pool: {}", e))?;

        let instance = Self {
            pool,
            database_name,
            admin_config: admin_url,
        };

        // Run schema migrations and seed the test user on the fresh database
        instance.run_migrations_with_seed().await?;

        // Ensure the database is fully ready by attempting a simple query
        tokio::time::sleep(std::time::Duration::from_millis(200)).await;

        Ok(instance)
    }

    /// Creates a fresh isolated test database with schema only — no seeded users.
    /// Use this for tests that require an empty users table (e.g. first-run setup tests).
    pub async fn new_empty() -> Result<Self, Box<dyn std::error::Error>> {
        let instance = Self::new().await?;
        // Remove the seeded test user so the DB is truly empty for setup flow tests
        let client = instance.pool.get().await?;
        client.execute("DELETE FROM users", &[]).await?;
        Ok(instance)
    }

    /// Runs database migrations for the test database
    async fn run_migrations(&self) -> Result<(), Box<dyn std::error::Error>> {
        let init_sql = include_str!("../db/init.sql");
        crate::db::run_migrations_with_schema(&self.pool, init_sql).await
    }

    /// Runs migrations AND seeds the fixed-UUID test user needed by most integration tests.
    async fn run_migrations_with_seed(&self) -> Result<(), Box<dyn std::error::Error>> {
        self.run_migrations().await?;
        let client = self.pool.get().await?;
        // password: "admin123"
        client.execute(
            "INSERT INTO users (id, username, email, password_hash, role) \
             VALUES ('550e8400-e29b-41d4-a716-446655440000', 'test-user', 'test@localhost', \
             '$argon2id$v=19$m=19456,t=2,p=1$ykODG4Kjv3ZOijtRLuNlFA$+6QnBbvOF+uWMm/po/O6mEZc9I9sZ/VBzi0fnp95ZnM', \
             'admin') ON CONFLICT (id) DO NOTHING",
            &[],
        ).await?;
        Ok(())
    }

    /// Get a reference to the database pool
    pub fn pool(&self) -> &Pool {
        &self.pool
    }
}

impl Drop for TestDatabase {
    fn drop(&mut self) {
        // Drop the test database when done
        let db_name = self.database_name.clone();
        let admin_url = self.admin_config.clone();

        std::thread::spawn(move || {
            let rt = tokio::runtime::Runtime::new().unwrap();
            rt.block_on(async move {
                // Parse admin config
                if let Ok(admin_config) = admin_url.parse::<tokio_postgres::Config>() {
                    let mut cfg = Config::new();
                    cfg.host = admin_config.get_hosts().get(0).map(|h| {
                        match h {
                            tokio_postgres::config::Host::Tcp(s) => s.clone(),
                            #[cfg(unix)]
                            tokio_postgres::config::Host::Unix(p) => p.to_string_lossy().to_string(),
                        }
                    });
                    cfg.port = admin_config.get_ports().get(0).copied().map(|p| p as u16);
                    cfg.user = admin_config.get_user().map(|s| s.to_string());
                    cfg.password = admin_config.get_password().map(|p| String::from_utf8_lossy(p).to_string());
                    cfg.dbname = Some("postgres".to_string());

                    if let Ok(admin_pool) = cfg.create_pool(Some(Runtime::Tokio1), NoTls) {
                        if let Ok(client) = admin_pool.get().await {
                            // Terminate all connections to the test database
                            let _ = client.execute(
                                &format!(
                                    "SELECT pg_terminate_backend(pid) FROM pg_stat_activity WHERE datname = '{}' AND pid <> pg_backend_pid()",
                                    db_name
                                ),
                                &[]
                            ).await;

                            // Drop the test database
                            let _ = client.execute(&format!("DROP DATABASE IF EXISTS {}", db_name), &[]).await;
                        }
                    }
                }
            });
        });
    }
}

/// Helper function to create a test database pool for use in tests
/// This provides a quick way to set up a fresh, isolated database for each test
pub async fn setup_test_database() -> Pool {
    TestDatabase::new()
        .await
        .expect("Failed to create test database")
        .pool()
        .clone()
}

/// Creates a test database with schema only (no seeded users). Use for first-run setup tests.
pub async fn setup_empty_test_database_with_instance() -> (Pool, TestDatabase) {
    let test_db = TestDatabase::new_empty()
        .await
        .expect("Failed to create empty test database");
    let pool = test_db.pool().clone();
    (pool, test_db)
}

/// Helper function to create a test database instance that can be kept alive for the duration of a test
/// This is useful for integration tests where the database needs to live as long as web app is running
pub async fn setup_test_database_with_instance() -> (Pool, TestDatabase) {
    let test_db = TestDatabase::new()
        .await
        .expect("Failed to create test database");
    let pool = test_db.pool().clone();
    (pool, test_db)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_test_database_creation() {
        let test_db = TestDatabase::new().await.expect("Failed to create test database");
        let pool = test_db.pool();
        let client = pool.get().await.expect("Failed to get client");

        let test_user_id = uuid::Uuid::parse_str("550e8400-e29b-41d4-a716-446655440000").unwrap();

        // Test that we can insert and query data
        let result = client
            .execute(
                "INSERT INTO images (user_id, deviceid, hash, name, ext, type, has_thumbnail) VALUES ($1, $2, $3, $4, $5, $6, $7)",
                &[&test_user_id, &"test_device", &"test_hash", &"test_name.jpg", &"jpg", &"camera", &false],
            )
            .await;
        assert!(result.is_ok());

        let rows = client
            .query("SELECT hash, name FROM images WHERE user_id = $1 AND hash = $2", &[&test_user_id, &"test_hash"])
            .await
            .expect("Failed to query");

        assert_eq!(rows.len(), 1);
        let hash: &str = rows[0].get(0);
        let name: &str = rows[0].get(1);
        assert_eq!(hash, "test_hash");
        assert_eq!(name, "test_name.jpg");

        // No need to clean up - the database will be dropped automatically
    }

    #[tokio::test]
    async fn test_setup_test_database_helper() {
        let pool = setup_test_database().await;
        let client = pool.get().await.expect("Failed to get client");

        let test_user_id = uuid::Uuid::parse_str("550e8400-e29b-41d4-a716-446655440000").unwrap();

        // Test that we can insert and query data
        let result = client
            .execute(
                "INSERT INTO images (user_id, deviceid, hash, name, ext, type, has_thumbnail) VALUES ($1, $2, $3, $4, $5, $6, $7)",
                &[&test_user_id, &"test_device", &"test_hash", &"test_name.jpg", &"jpg", &"camera", &false],
            )
            .await;
        assert!(result.is_ok());

        let rows = client
            .query("SELECT hash, name FROM images WHERE user_id = $1 AND hash = $2", &[&test_user_id, &"test_hash"])
            .await
            .expect("Failed to query");

        assert_eq!(rows.len(), 1);
        let hash: &str = rows[0].get(0);
        let name: &str = rows[0].get(1);
        assert_eq!(hash, "test_hash");
        assert_eq!(name, "test_name.jpg");

        // No need to clean up - the database will be dropped automatically
    }
}