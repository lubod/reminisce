/// Database query instrumentation for performance monitoring
/// Logs slow queries and tracks query execution times

use tracing::{info, warn, instrument};
use std::time::Instant;
use tokio_postgres::Row;
use crate::metrics::{DB_QUERY_DURATION, SLOW_QUERIES_TOTAL};

/// Threshold in milliseconds for logging slow queries
const SLOW_QUERY_THRESHOLD_MS: u128 = 100;

/// Execute a query and log performance metrics
#[instrument(skip(client, params), fields(operation = %operation_name, elapsed_ms, query_preview, status))]
pub async fn instrumented_query(
    client: &tokio_postgres::Client,
    query: &str,
    params: &[&(dyn tokio_postgres::types::ToSql + Sync)],
    operation_name: &str,
) -> Result<Vec<Row>, tokio_postgres::Error> {
    let start = Instant::now();
    let result = client.query(query, params).await;
    let elapsed = start.elapsed();

    log_query_performance(operation_name, query, elapsed.as_millis(), result.is_ok());

    result
}

/// Execute a query_one and log performance metrics
#[instrument(skip(client, params), fields(operation = %operation_name, elapsed_ms, query_preview, status))]
pub async fn instrumented_query_one(
    client: &tokio_postgres::Client,
    query: &str,
    params: &[&(dyn tokio_postgres::types::ToSql + Sync)],
    operation_name: &str,
) -> Result<Row, tokio_postgres::Error> {
    let start = Instant::now();
    let result = client.query_one(query, params).await;
    let elapsed = start.elapsed();

    log_query_performance(operation_name, query, elapsed.as_millis(), result.is_ok());

    result
}

/// Execute a query_opt and log performance metrics
#[instrument(skip(client, params), fields(operation = %operation_name, elapsed_ms, query_preview, status))]
pub async fn instrumented_query_opt(
    client: &tokio_postgres::Client,
    query: &str,
    params: &[&(dyn tokio_postgres::types::ToSql + Sync)],
    operation_name: &str,
) -> Result<Option<Row>, tokio_postgres::Error> {
    let start = Instant::now();
    let result = client.query_opt(query, params).await;
    let elapsed = start.elapsed();

    log_query_performance(operation_name, query, elapsed.as_millis(), result.is_ok());

    result
}

/// Execute an execute statement and log performance metrics
#[instrument(skip(client, params), fields(operation = %operation_name, elapsed_ms, query_preview, status))]
pub async fn instrumented_execute(
    client: &tokio_postgres::Client,
    query: &str,
    params: &[&(dyn tokio_postgres::types::ToSql + Sync)],
    operation_name: &str,
) -> Result<u64, tokio_postgres::Error> {
    let start = Instant::now();
    let result = client.execute(query, params).await;
    let elapsed = start.elapsed();

    log_query_performance(operation_name, query, elapsed.as_millis(), result.is_ok());

    result
}

/// Execute a transaction query and log performance metrics
#[instrument(skip(transaction, params), fields(operation = %operation_name, elapsed_ms, query_preview, status))]
pub async fn instrumented_transaction_query(
    transaction: &tokio_postgres::Transaction<'_>,
    query: &str,
    params: &[&(dyn tokio_postgres::types::ToSql + Sync)],
    operation_name: &str,
) -> Result<Vec<Row>, tokio_postgres::Error> {
    let start = Instant::now();
    let result = transaction.query(query, params).await;
    let elapsed = start.elapsed();

    log_query_performance(operation_name, query, elapsed.as_millis(), result.is_ok());

    result
}

/// Execute a transaction query_opt and log performance metrics
#[instrument(skip(transaction, params), fields(operation = %operation_name, elapsed_ms, query_preview, status))]
pub async fn instrumented_transaction_query_opt(
    transaction: &tokio_postgres::Transaction<'_>,
    query: &str,
    params: &[&(dyn tokio_postgres::types::ToSql + Sync)],
    operation_name: &str,
) -> Result<Option<Row>, tokio_postgres::Error> {
    let start = Instant::now();
    let result = transaction.query_opt(query, params).await;
    let elapsed = start.elapsed();

    log_query_performance(operation_name, query, elapsed.as_millis(), result.is_ok());

    result
}

/// Execute a transaction execute and log performance metrics
#[instrument(skip(transaction, params), fields(operation = %operation_name, elapsed_ms, query_preview, status))]
pub async fn instrumented_transaction_execute(
    transaction: &tokio_postgres::Transaction<'_>,
    query: &str,
    params: &[&(dyn tokio_postgres::types::ToSql + Sync)],
    operation_name: &str,
) -> Result<u64, tokio_postgres::Error> {
    let start = Instant::now();
    let result = transaction.execute(query, params).await;
    let elapsed = start.elapsed();

    log_query_performance(operation_name, query, elapsed.as_millis(), result.is_ok());

    result
}

/// Log query performance metrics
fn log_query_performance(_operation: &str, query: &str, elapsed_ms: u128, success: bool) {
    // Extract first 100 chars of query for logging (remove extra whitespace)
    let query_preview = query
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .chars()
        .take(100)
        .collect::<String>();

    let status = if success { "OK" } else { "ERROR" };

    // Record metrics in Prometheus
    DB_QUERY_DURATION.observe(elapsed_ms as f64 / 1000.0); // Convert to seconds
    if elapsed_ms >= SLOW_QUERY_THRESHOLD_MS {
        SLOW_QUERIES_TOTAL.inc();
    }

    // Record fields in the current span for distributed tracing
    let span = tracing::Span::current();
    span.record("elapsed_ms", elapsed_ms);
    span.record("query_preview", query_preview.as_str());
    span.record("status", status);

    if elapsed_ms >= SLOW_QUERY_THRESHOLD_MS {
        warn!(
            elapsed_ms = %elapsed_ms,
            status = %status,
            query = %query_preview,
            "Slow query detected"
        );
    } else {
        info!(
            elapsed_ms = %elapsed_ms,
            status = %status,
            query = %query_preview,
            "Query executed"
        );
    }
}
