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