//! In-app health alerts — the "alerting" half of the in-app observability page.
//!
//! Replaces the Prometheus `alert-rules.yml` set for this single, self-hosted
//! app. Everything is computed on demand from data the server already tracks:
//! the ring-buffer error rate, deadpool stats, Prometheus gauge/counter values
//! and `sysinfo` (CPU/memory/disk). No external system is queried.

use actix_web::web;
use serde::Serialize;
use sysinfo::{CpuExt, DiskExt, SystemExt};

use crate::db::MainDbPool;
use crate::logtail;
use crate::services::system_stats::SharedSystem;

#[derive(Debug, Clone, Serialize)]
pub struct Alert {
    pub id: String,
    pub severity: String, // "ok" | "warning" | "critical"
    pub status: String,   // "firing" | "ok"
    pub message: String,
    pub detail: String,
    pub value: String,
}

impl Alert {
    fn ok(id: &str, message: &str, detail: &str) -> Self {
        Self {
            id: id.to_string(),
            severity: "ok".into(),
            status: "ok".into(),
            message: message.to_string(),
            detail: detail.to_string(),
            value: String::new(),
        }
    }

    fn firing(id: &str, severity: &str, message: &str, detail: &str, value: &str) -> Self {
        Self {
            id: id.to_string(),
            severity: severity.into(),
            status: "firing".into(),
            message: message.to_string(),
            detail: detail.to_string(),
            value: value.to_string(),
        }
    }
}

/// Evaluates the alert set. `pool` and `sys` come from the request's web::Data.
pub fn compute_alerts(pool: &web::Data<MainDbPool>, sys: &SharedSystem) -> Vec<Alert> {
    let mut alerts: Vec<Alert> = Vec::new();

    // --- Error rate (from the in-process ring buffer) ---
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let error_5m = if let Ok(store) = logtail::ring().lock() {
        store.count_since(now.saturating_sub(300), logtail::level_from_str("error"))
    } else {
        0
    };
    if error_5m > 20 {
        alerts.push(Alert::firing(
            "HighErrorRate",
            "warning",
            "High error rate",
            &format!("{} ERROR/PANIC events in the last 5 minutes", error_5m),
            &error_5m.to_string(),
        ));
    } else if error_5m > 0 {
        alerts.push(Alert::ok(
            "HighErrorRate",
            "Error rate normal",
            &format!("{} ERROR/PANIC events in the last 5 minutes", error_5m),
        ));
    } else {
        alerts.push(Alert::ok("HighErrorRate", "Error rate normal", "No errors in the last 5 minutes"));
    }

    // --- Database pool (deadpool stats) ---
    let status = pool.0.status();
    let size = status.size as f64;
    let available = status.available.max(0) as f64;
    let utilization = if size > 0.0 { ((size - available) / size) * 100.0 } else { 0.0 };
    if utilization >= 95.0 {
        alerts.push(Alert::firing(
            "DatabaseConnectionPoolExhausted",
            "critical",
            "Database connection pool exhausted",
            &format!("{:.0}% of {} connections in use", utilization, size),
            &format!("{:.0}%", utilization),
        ));
    } else if utilization >= 80.0 {
        alerts.push(Alert::firing(
            "DatabaseConnectionPoolNearLimit",
            "warning",
            "Database connection pool near limit",
            &format!("{:.0}% of {} connections in use", utilization, size),
            &format!("{:.0}%", utilization),
        ));
    } else {
        alerts.push(Alert::ok(
            "DatabaseConnectionPoolNearLimit",
            "Database pool healthy",
            &format!("{:.0}% of {} connections in use", utilization, size),
        ));
    }

    // --- Backup peers (prometheus gauge) ---
    match metric("backup_peers_available") {
        Some(peers) if peers <= 0.0 => {
            alerts.push(Alert::firing(
                "BackupPeersUnavailable",
                "warning",
                "No backup peers available",
                "0 peers reachable for media backup — uploads will queue",
                "0",
            ));
        }
        Some(peers) => {
            alerts.push(Alert::ok(
                "BackupPeersUnavailable",
                "Backup peers available",
                &format!("{:.0} peer(s) reachable", peers),
            ));
        }
        None => {
            alerts.push(Alert::ok("BackupPeersUnavailable", "Backup peers — n/a", "No peer gauge sampled yet"));
        }
    }

    // --- Backup silently failing (counters) ---
    let failures = counter_delta("backup_failures_total", metric("backup_failures_total").unwrap_or(0.0));
    let successes = counter_delta("backup_success_total", metric("backup_success_total").unwrap_or(0.0));
    if failures > 5.0 && failures > successes {
        alerts.push(Alert::firing(
            "BackupFailingSilently",
            "critical",
            "Media backup is failing",
            &format!("{} failures vs {} successes", failures, successes),
            &format!("{:.0}/{:.0}", failures, successes),
        ));
    } else if failures > 0.0 {
        alerts.push(Alert::firing(
            "BackupFailingSilently",
            "ok",
            "Media backup stabilizing",
            &format!("{} failures, {} successes", failures, successes),
            &format!("{:.0}/{:.0}", failures, successes),
        ));
    } else {
        alerts.push(Alert::ok("BackupFailingSilently", "Media backup OK", "No backup failures recorded"));
    }

    // --- Database backups (counters) ---
    let db_ok_now = metric("db_backup_success_total");
    let db_fail_now = metric("db_backup_failures_total");
    if db_ok_now.is_none() && db_fail_now.is_none() {
        alerts.push(Alert::ok("DatabaseBackupStatus", "DB backups — n/a", "No DB backup yet recorded"));
    } else {
        let db_ok_d = counter_delta("db_backup_success_total", db_ok_now.unwrap_or(0.0));
        let db_fail_d = counter_delta("db_backup_failures_total", db_fail_now.unwrap_or(0.0));
        if db_fail_d > 0.0 && db_fail_d >= db_ok_d {
            alerts.push(Alert::firing(
                "DatabaseBackupStatus",
                "warning",
                "Database backups failing",
                &format!("{} failures vs {} successes in the last window", db_fail_d, db_ok_d),
                &format!("{:.0}/{:.0}", db_fail_d, db_ok_d),
            ));
        } else {
            alerts.push(Alert::ok(
                "DatabaseBackupStatus",
                "Database backups OK",
                &format!("{} successful DB backups in the last window", db_ok_d),
            ));
        }
    }

    // --- System: CPU / memory / disk (sysinfo, refreshed every 15s) ---
    let lock = sys.lock().unwrap();
    let cpu = lock.global_cpu_info().cpu_usage();
    let mem_total = lock.total_memory() as f64;
    let mem_used = lock.used_memory() as f64;
    let mem_pct = if mem_total > 0.0 { (mem_used / mem_total) * 100.0 } else { 0.0 };
    let (disk_total, disk_used) = {
        let mut td = 0.0f64;
        let mut ud = 0.0f64;
        for disk in lock.disks() {
            let mp = disk.mount_point().to_str().unwrap_or("");
            if mp == "/" || mp.contains("data") {
                td = disk.total_space() as f64;
                ud = td - disk.available_space() as f64;
                break;
            }
        }
        (td, ud)
    };
    let disk_pct = if disk_total > 0.0 { (disk_used / disk_total) * 100.0 } else { 0.0 };
    drop(lock);

    if cpu > 90.0 {
        alerts.push(Alert::firing(
            "NodeHighCPU",
            "warning",
            "CPU usage high",
            &format!("{:.0}% CPU", cpu),
            &format!("{:.0}%", cpu),
        ));
    } else {
        alerts.push(Alert::ok("NodeHighCPU", "CPU usage normal", &format!("{:.0}% CPU", cpu)));
    }

    if mem_pct > 90.0 {
        alerts.push(Alert::firing(
            "NodeHighMemory",
            "warning",
            "Memory usage high",
            &format!("{:.0}% of memory used", mem_pct),
            &format!("{:.0}%", mem_pct),
        ));
    } else {
        alerts.push(Alert::ok("NodeHighMemory", "Memory usage normal", &format!("{:.0}% used", mem_pct)));
    }

    if disk_pct >= 90.0 {
        alerts.push(Alert::firing(
            "NodeCriticalDiskSpace",
            "critical",
            "Critical disk space",
            &format!("{:.0}% disk used", disk_pct),
            &format!("{:.0}%", disk_pct),
        ));
    } else if disk_pct >= 80.0 {
        alerts.push(Alert::firing(
            "NodeLowDiskSpace",
            "warning",
            "Low disk space",
            &format!("{:.0}% disk used", disk_pct),
            &format!("{:.0}%", disk_pct),
        ));
    } else {
        alerts.push(Alert::ok("NodeLowDiskSpace", "Disk space OK", &format!("{:.0}% used", disk_pct)));
    }

    // --- Authentication (counters) ---
    let logins = metric("user_logins_total").unwrap_or(0.0);
    let login_failures = counter_delta("user_login_failures_total", metric("user_login_failures_total").unwrap_or(0.0));
    let logins = logins.max(login_failures); // keep comparison meaningful on first sample
    if login_failures > 20.0 && login_failures > logins {
        alerts.push(Alert::firing(
            "HighLoginFailureRate",
            "warning",
            "Repeated failed logins",
            &format!("{} login failures vs {} logins (possible brute-force)", login_failures, logins),
            &format!("{:.0}", login_failures),
        ));
    } else {
        alerts.push(Alert::ok("HighLoginFailureRate", "Logins OK", &format!("{:.0} failures", login_failures)));
    }

    alerts
}


/// Delta of a cumulative counter since the previous evaluation window.
/// Alerts must react to what happened RECENTLY, not to lifetime totals: a
/// counter that once crossed a threshold would otherwise fire forever.
fn counter_delta(name: &'static str, current: f64) -> f64 {
    use std::collections::HashMap;
    use std::sync::Mutex;
    static PREV: std::sync::OnceLock<Mutex<HashMap<&'static str, f64>>> = std::sync::OnceLock::new();
    let map = PREV.get_or_init(|| Mutex::new(HashMap::new()));
    let mut g = map.lock().unwrap_or_else(|e| e.into_inner());
    // Leak-free: names are a small closed set (static strings).
    let prev = g.insert(name, current).unwrap_or(current);
    (current - prev).max(0.0)
}

/// Returns the first sample value for a registered Prometheus metric by name.
fn metric(name: &str) -> Option<f64> {
    for family in prometheus::gather() {
        if family.get_name() != name {
            continue;
        }
        if let Some(m) = family.get_metric().first() {
            if m.has_gauge() {
                return Some(m.get_gauge().get_value());
            }
            if m.has_counter() {
                return Some(m.get_counter().get_value());
            }
            return Some(0.0);
        }
    }
    None
}