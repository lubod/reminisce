//! In-app observability API: logs, errors, alerts and GPU status.
//!
//! These back the new `/system` page in the client. Everything is served by the
//! app itself (ring buffer + rotating files + metric values) — no Loki,
//! Prometheus or Tempo required.

use std::collections::HashMap;

use actix_web::{get, web, HttpRequest, HttpResponse};
use serde::Serialize;

use crate::alerts;
use crate::config::Config;
use crate::db::MainDbPool;
use crate::logtail;
use crate::services::system_stats::SharedSystem;
use sysinfo::SystemExt;

#[derive(Serialize)]
struct LogsResponse {
    entries: Vec<logtail::LogEntry>,
    source: &'static str,
}

#[derive(Serialize)]
struct ErrorsResponse {
    entries: Vec<logtail::LogEntry>,
    count_5m: ErrorCounts,
}

#[derive(Serialize, Default)]
struct ErrorCounts {
    error: usize,
    warn: usize,
    panic: usize,
}

#[derive(Serialize)]
struct AlertsResponse {
    alerts: Vec<alerts::Alert>,
}

#[derive(Serialize)]
struct WorkerStats {
    id: String,
    name: String,
    count: u64,
    mean_ms: Option<f64>,
    p50_ms: Option<f64>,
    p90_ms: Option<f64>,
    p95_ms: Option<f64>,
    p99_ms: Option<f64>,
}

#[derive(Serialize, Default)]
struct HttpStatusBreakdown {
    http_2xx: u64,
    http_3xx: u64,
    http_4xx: u64,
    http_5xx: u64,
}

#[derive(Serialize)]
struct HttpStats {
    total: u64,
    per_second: f64,
    status: HttpStatusBreakdown,
    duration_ms: WorkerStats,
}

#[derive(Serialize)]
struct PipelineResponse {
    workers: Vec<WorkerStats>,
    http: HttpStats,
    db_query_ms: WorkerStats,
}

/// Estimate a histogram percentile (linear interpolation between buckets).
fn hist_percentile(h: &prometheus::proto::Histogram, q: f64) -> Option<f64> {
    let count = h.get_sample_count() as f64;
    if count <= 0.0 {
        return None;
    }
    let target = q * count;
    let mut cum = 0.0f64;
    let mut prev = 0.0f64;
    for b in h.get_bucket() {
        let ub = b.get_upper_bound();
        let bcum = b.get_cumulative_count() as f64;
        if bcum >= target {
            let within = bcum - cum;
            let f = if within > 0.0 { (target - cum) / within } else { 0.0 };
            return Some(prev + (ub - prev) * f);
        }
        cum = bcum;
        prev = ub;
    }
    Some(prev)
}

fn histogram_stats(fam: Option<&prometheus::proto::MetricFamily>, id: &str, name: &str) -> WorkerStats {
    let mut ws = WorkerStats {
        id: id.to_string(),
        name: name.to_string(),
        count: 0,
        mean_ms: None,
        p50_ms: None,
        p90_ms: None,
        p95_ms: None,
        p99_ms: None,
    };
    if let Some(f) = fam {
        if let Some(m) = f.get_metric().first() {
            if m.has_histogram() {
                let h = m.get_histogram();
                ws.count = h.get_sample_count();
                if ws.count > 0 {
                    ws.mean_ms = Some((h.get_sample_sum() / ws.count as f64) * 1000.0);
                    ws.p50_ms = hist_percentile(h, 0.50).map(|v| v * 1000.0);
                    ws.p90_ms = hist_percentile(h, 0.90).map(|v| v * 1000.0);
                    ws.p95_ms = hist_percentile(h, 0.95).map(|v| v * 1000.0);
                    ws.p99_ms = hist_percentile(h, 0.99).map(|v| v * 1000.0);
                }
            }
        }
    }
    ws
}

/// Total + status-code breakdown from `api_http_requests_total`.
fn http_status(fam: Option<&prometheus::proto::MetricFamily>) -> (u64, HttpStatusBreakdown) {
    let mut total = 0u64;
    let mut sb = HttpStatusBreakdown::default();
    if let Some(f) = fam {
        for m in f.get_metric() {
            let v = if m.has_counter() { m.get_counter().get_value() as u64 } else { 0 };
            total += v;
            // Status code is the numeric label (actix-web-prom), whatever its name.
            let mut status = String::new();
            for l in m.get_label() {
                let val = l.get_value();
                if val.bytes().all(|b| b.is_ascii_digit()) {
                    status = val.to_string();
                    break;
                }
            }
            match status.as_str() {
                s if s.starts_with("2") => sb.http_2xx += v,
                s if s.starts_with("3") => sb.http_3xx += v,
                s if s.starts_with("4") => sb.http_4xx += v,
                s if s.starts_with("5") => sb.http_5xx += v,
                _ => {}
            }
        }
    }
    (total, sb)
}

#[get("/admin/pipeline")]
pub async fn get_admin_pipeline(
    req: HttpRequest,
    cfg: web::Data<Config>,
    sys: web::Data<SharedSystem>,
) -> Result<HttpResponse, actix_web::Error> {
    if let Err(resp) = require_admin(&req, &cfg, "get_admin_pipeline").await {
        return Ok(resp);
    }

    let gathered = prometheus::gather();
    let common = |name: &str| gathered.iter().find(|f| f.get_name() == name);

    let mut workers: Vec<WorkerStats> = Vec::with_capacity(6);
    for (name, id, label) in [
        ("embedding_duration_seconds", "embedding", "Embedding"),
        ("ai_description_duration_seconds", "description", "Description"),
        ("face_detection_duration_seconds", "face_detection", "Face detection"),
        ("face_clustering_duration_seconds", "face_clustering", "Face clustering"),
        ("backup_duration_seconds", "backup", "Media backup"),
        ("db_backup_duration_seconds", "db_backup", "DB backup"),
    ] {
        workers.push(histogram_stats(common(name), id, label));
    }

    let (total, status) = http_status(common("api_http_requests_total"));
    let duration = histogram_stats(common("api_http_requests_duration_seconds"), "http", "HTTP");
    let uptime = {
        let lock = sys.lock().unwrap();
        lock.uptime()
    }
    .max(1);
    let per_second = total as f64 / uptime as f64;

    let db_query_ms = histogram_stats(common("db_query_duration_seconds"), "db_query", "DB query");

    Ok(HttpResponse::Ok().json(PipelineResponse {
        workers,
        http: HttpStats {
            total,
            per_second,
            status,
            duration_ms: duration,
        },
        db_query_ms,
    }))
}

#[derive(Serialize)]
struct SeriesPoint {
    t: i64,
    v: f64,
}

#[derive(Serialize)]
struct Series {
    name: String,
    unit: &'static str,
    points: Vec<SeriesPoint>,
}

#[derive(Serialize)]
struct SeriesResponse {
    range: String,
    series: Vec<Series>,
}

fn series_unit(name: &str) -> &'static str {
    match name {
        "system_cpu_percent" | "system_mem_percent" | "system_disk_used_percent" | "db_pool_util_percent" => "%",
        "system_disk_free_gb" => "GB",
        "backup_peers_available" => "peers",
        "backlog_description" | "backlog_embedding" | "backlog_face" => "images",
        "ai_descriptions_per_hr" | "ai_embeddings_per_hr" | "ai_faces_per_hr" | "ai_errors_per_hr" | "http_requests_per_hr" => "/hr",
        _ => "ms",
    }
}

#[get("/admin/series")]
pub async fn get_admin_series(
    req: HttpRequest,
    cfg: web::Data<Config>,
    pool: web::Data<MainDbPool>,
    query: web::Query<HashMap<String, String>>,
) -> Result<HttpResponse, actix_web::Error> {
    if let Err(resp) = require_admin(&req, &cfg, "get_admin_series").await {
        return Ok(resp);
    }

    let range = query.get("range").map(|s| s.as_str()).unwrap_or("1d").to_string();
    let (lookback, bucket) = match range.as_str() {
        "30d" => ("720 hours", "3600 seconds"),
        "90d" => ("2160 hours", "21600 seconds"),
        _ => ("24 hours", "300 seconds"),
    };

    let q = "WITH s AS (
                SELECT date_bin($1::text::interval, ts, '2000-01-01 00:00:00+00') AS b,
                       name, avg(value) AS v
                FROM metric_samples
                WHERE ts > now() - $2::text::interval
                GROUP BY name, b
             )
             SELECT name, EXTRACT(EPOCH FROM b)::bigint, v FROM s ORDER BY name, b";

    let db = match pool.0.get().await {
        Ok(db) => db,
        Err(e) => return Ok(HttpResponse::ServiceUnavailable().json(serde_json::json!({"error": e.to_string()}))),
    };
    let points_by_name: HashMap<String, Vec<SeriesPoint>> =
        match db.query(q, &[&bucket, &lookback]).await {
        Ok(rows) => {
            let mut map: HashMap<String, Vec<SeriesPoint>> = HashMap::new();
            for row in rows {
                let name: String = row.get(0);
                let t: i64 = row.get(1);
                let v: f64 = row.get(2);
                map.entry(name).or_default().push(SeriesPoint { t, v });
            }
            map
        }
        Err(e) => return Ok(HttpResponse::ServiceUnavailable().json(serde_json::json!({"error": e.to_string()}))),
    };

    let mut series: Vec<Series> = points_by_name
        .into_iter()
        .map(|(name, mut points)| {
            points.sort_by_key(|p| p.t);
            let unit = series_unit(&name);
            Series { name, unit, points }
        })
        .collect();
    series.sort_by(|a, b| a.name.cmp(&b.name));

    Ok(HttpResponse::Ok().json(SeriesResponse { range, series }))
}

/// Authenticate + require an admin role (mirrors `get_system_stats`).
async fn require_admin(
    req: &HttpRequest,
    cfg: &web::Data<Config>,
    handler: &str,
) -> Result<(), HttpResponse> {
    let claims = match crate::utils::authenticate_request(req, handler, cfg.get_api_key()).await {
        Ok(claims) => claims,
        Err(resp) => return Err(resp),
    };
    if claims.role != "admin" {
        return Err(HttpResponse::Forbidden().json(serde_json::json!({
            "error": "Forbidden: Admin role required"
        })));
    }
    Ok(())
}

#[get("/admin/logs")]
pub async fn get_admin_logs(
    req: HttpRequest,
    cfg: web::Data<Config>,
    query: web::Query<HashMap<String, String>>,
) -> Result<HttpResponse, actix_web::Error> {
    if let Err(resp) = require_admin(&req, &cfg, "get_admin_logs").await {
        return Ok(resp);
    }

    let min_level = logtail::level_from_str(query.get("level").map(|s| s.as_str()).unwrap_or("info"));
    let limit = query
        .get("limit")
        .and_then(|s| s.parse::<usize>().ok())
        .unwrap_or(200)
        .min(2000);
    let full = query.get("full").map(|s| s == "1").unwrap_or(false)
        || query.get("full").map(|s| s == "true").unwrap_or(false);

    let entries = if full {
        logtail::read_file_history(min_level, limit)
    } else if let Ok(store) = logtail::ring().lock() {
        store.query(min_level, limit)
    } else {
        Vec::new()
    };
    let source = if full { "file" } else { "ring" };

    Ok(HttpResponse::Ok().json(LogsResponse { entries, source }))
}

#[get("/admin/errors")]
pub async fn get_admin_errors(
    req: HttpRequest,
    cfg: web::Data<Config>,
    query: web::Query<HashMap<String, String>>,
) -> Result<HttpResponse, actix_web::Error> {
    if let Err(resp) = require_admin(&req, &cfg, "get_admin_errors").await {
        return Ok(resp);
    }

    let limit = query
        .get("limit")
        .and_then(|s| s.parse::<usize>().ok())
        .unwrap_or(100)
        .min(1000);

    let store = logtail::ring();
    let (entries, counts) = if let Ok(store) = store.lock() {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0);
        let since = now.saturating_sub(300);
        let mut counts = ErrorCounts::default();
        for e in store.query(logtail::level_from_str("warn"), usize::MAX) {
            if e.timestamp >= since {
                match e.level.as_str() {
                    "PANIC" => counts.panic += 1,
                    "ERROR" => counts.error += 1,
                    "WARN" | "WARNING" => counts.warn += 1,
                    _ => {}
                }
            }
        }
        (store.query(logtail::level_from_str("error"), limit), counts)
    } else {
        (Vec::new(), ErrorCounts::default())
    };

    Ok(HttpResponse::Ok().json(ErrorsResponse { entries, count_5m: counts }))
}

#[get("/admin/alerts")]
pub async fn get_admin_alerts(
    req: HttpRequest,
    cfg: web::Data<Config>,
    pool: web::Data<MainDbPool>,
    sys: web::Data<SharedSystem>,
) -> Result<HttpResponse, actix_web::Error> {
    if let Err(resp) = require_admin(&req, &cfg, "get_admin_alerts").await {
        return Ok(resp);
    }
    Ok(HttpResponse::Ok().json(AlertsResponse {
        alerts: alerts::compute_alerts(&pool, &sys),
    }))
}

#[get("/admin/gpu")]
pub async fn get_admin_gpu(
    req: HttpRequest,
    cfg: web::Data<Config>,
) -> Result<HttpResponse, actix_web::Error> {
    if let Err(resp) = require_admin(&req, &cfg, "get_admin_gpu").await {
        return Ok(resp);
    }

    let base = match cfg.ai_http_url.as_deref() {
        Some(url) if !url.is_empty() => url.trim_end_matches('/').to_string(),
        _ => return Ok(HttpResponse::Ok().json(serde_json::json!({ "available": false, "reason": "ai_http_url unset" }))),
    };

    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(4))
        .build()
        .unwrap_or_default();

    match client.get(format!("{}/gpu-metrics", base)).send().await {
        Ok(resp) => match resp.json::<serde_json::Value>().await {
            Ok(v) => Ok(HttpResponse::Ok().json(v)),
            Err(_) => Ok(HttpResponse::Ok().json(serde_json::json!({
                "available": false, "reason": "bad response"
            }))),
        },
        Err(_) => Ok(HttpResponse::Ok().json(serde_json::json!({
            "available": false, "reason": "ai-server unreachable"
        }))),
    }
}