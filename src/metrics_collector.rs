use std::collections::HashMap;
use std::time::{Duration, Instant};

use actix_web::web;
use sysinfo::{CpuExt, DiskExt, System, SystemExt};
use tokio::time;

use crate::db::{GeotaggingDbPool, MainDbPool};
use crate::metrics;

pub async fn start_metrics_collector(
    main_pool: web::Data<MainDbPool>,
    _geo_pool: web::Data<GeotaggingDbPool>, // Kept for future use
    config: web::Data<crate::config::Config>,
    shutdown_token: tokio_util::sync::CancellationToken,
) {
    let mut sampler = Sampler::new();
    let mut interval = time::interval(Duration::from_secs(15));

    loop {
        tokio::select! {
            _ = shutdown_token.cancelled() => {
                log::info!("Metrics collector stopping gracefully...");
                break;
            }
            _ = interval.tick() => {
                let now = Instant::now();
                let elapsed = sampler.last_tick.map(|t| now.duration_since(t).as_secs_f64()).unwrap_or(15.0).max(0.01);
                sampler.last_tick = Some(now);

                // 1. DB pool gauges (existing behavior)
                collect_pool_metrics(&main_pool, config.db_pool_max_size);

                // 2. System gauges (cpu/ram/disk) via sysinfo
                sampler.sys.refresh_cpu();
                sampler.sys.refresh_memory();
                sampler.sys.refresh_disks();
                let cpu = sampler.sys.global_cpu_info().cpu_usage();
                let mtotal = sampler.sys.total_memory() as f64;
                let mused = sampler.sys.used_memory() as f64;
                let mem_pct = if mtotal > 0.0 { (mused / mtotal) * 100.0 } else { 0.0 };
                let (disk_total, disk_used) = {
                    let mut td = 0.0f64;
                    let mut ud = 0.0f64;
                    for d in sampler.sys.disks() {
                        let mp = d.mount_point().to_str().unwrap_or("");
                        if mp == "/" || mp.contains("data") {
                            td = d.total_space() as f64;
                            ud = td - d.available_space() as f64;
                            break;
                        }
                    }
                    (td, ud)
                };
                let disk_pct = if disk_total > 0.0 { (disk_used / disk_total) * 100.0 } else { 0.0 };
                let disk_free_gb = (disk_total - disk_used) / (1024.0 * 1024.0 * 1024.0);

                // 3. Prometheus snapshot (counters, gauges, histogram percentiles)
                let families = prometheus::gather();
                let find = |name: &str| families.iter().find(|f| f.get_name() == name);

                let mut samples: Vec<(String, f64)> = vec![
                    ("system_cpu_percent".into(), cpu as f64),
                    ("system_mem_percent".into(), mem_pct),
                    ("system_disk_used_percent".into(), disk_pct),
                    ("system_disk_free_gb".into(), disk_free_gb),
                    ("db_pool_util_percent".into(), pool_util(&main_pool)),
                    (
                        "backup_peers_available".into(),
                        find("backup_peers_available")
                            .map(family_gauge)
                            .unwrap_or(0.0),
                    ),
                ];

                // 4. Per-hour rates from counter deltas
                for (series_name, counter_name) in [
                    ("ai_descriptions_per_hr", "ai_description_success_total"),
                    ("ai_embeddings_per_hr", "embedding_success_total"),
                    ("ai_faces_per_hr", "face_detection_success_total"),
                    ("ai_errors_per_hr", "application_errors_total"),
                    ("http_requests_per_hr", "api_http_requests_total"),
                ] {
                    let cur = find(counter_name).map(family_counter).unwrap_or(0.0);
                    let prev = sampler.last_counters.get(counter_name).copied().unwrap_or(cur);
                    let per_hr = ((cur - prev).max(0.0) / elapsed) * 3600.0;
                    samples.push((series_name.to_string(), per_hr));
                    sampler.last_counters.insert(counter_name.to_string(), cur);
                }

                // 5. p95 latencies (ms) from histograms
                let p95 = |nm: &str| {
                    find(nm)
                        .and_then(|f| f.get_metric().first())
                        .filter(|m| m.has_histogram())
                        .and_then(|m| hist_percentile_ms(m.get_histogram(), 0.95))
                };
                if let Some(v) = p95("ai_description_duration_seconds") {
                    samples.push(("ai_description_p95_ms".into(), v));
                }
                if let Some(v) = p95("embedding_duration_seconds") {
                    samples.push(("ai_embedding_p95_ms".into(), v));
                }
                if let Some(v) = p95("face_detection_duration_seconds") {
                    samples.push(("ai_face_p95_ms".into(), v));
                }
                if let Some(v) = p95("api_http_requests_duration_seconds") {
                    samples.push(("http_p95_ms".into(), v));
                }
                if let Some(v) = p95("db_query_duration_seconds") {
                    samples.push(("db_query_p95_ms".into(), v));
                }

                // 6. Pending backlogs (throttled to every 5 min)
                if sampler.last_backlog.map(|t| t.elapsed() >= Duration::from_secs(300)).unwrap_or(true) {
                    sampler.last_backlog = Some(now);
                    if let Ok(client) = main_pool.0.get().await {
                        let q = "SELECT
                            (SELECT count(*) FROM images WHERE verification_status=1 AND deleted_at IS NULL AND description IS NULL),
                            (SELECT count(*) FROM images WHERE verification_status=1 AND deleted_at IS NULL AND embedding IS NULL AND embedding_generated_at IS NULL),
                            (SELECT count(*) FROM images WHERE verification_status=1 AND deleted_at IS NULL AND face_detection_completed_at IS NULL)";
                        if let Ok(row) = client.query_one(q, &[]).await {
                            samples.push(("backlog_description".into(), row.get::<_, i64>(0) as f64));
                            samples.push(("backlog_embedding".into(), row.get::<_, i64>(1) as f64));
                            samples.push(("backlog_face".into(), row.get::<_, i64>(2) as f64));
                        }
                    }
                }

                // 7. Persist the sample batch
                if let Ok(db) = main_pool.0.get().await {
                    let names: Vec<String> = samples.iter().map(|(n, _)| n.clone()).collect();
                    let values: Vec<f64> = samples.iter().map(|(_, v)| *v).collect();
                    let _ = db
                        .execute(
                            "INSERT INTO metric_samples(ts,name,value) \
                             SELECT now(), v.name, v.value \
                             FROM unnest($1::text[], $2::float8[]) AS v(name,value)",
                            &[&names, &values],
                        )
                        .await;
                }

                // 8. Prune old samples (daily)
                if sampler.last_prune.map(|t| t.elapsed() >= Duration::from_secs(86400)).unwrap_or(true) {
                    sampler.last_prune = Some(now);
                    if let Ok(db) = main_pool.0.get().await {
                        let _ = db
                            .execute("DELETE FROM metric_samples WHERE ts < now() - interval '100 days'", &[])
                            .await;
                    }
                }
            }
        }
    }
}

struct Sampler {
    sys: System,
    last_counters: HashMap<String, f64>,
    last_tick: Option<Instant>,
    last_backlog: Option<Instant>,
    last_prune: Option<Instant>,
}

impl Sampler {
    fn new() -> Self {
        Self {
            sys: System::new_all(),
            last_counters: HashMap::new(),
            last_tick: None,
            last_backlog: None,
            last_prune: None,
        }
    }
}

fn pool_util(pool: &web::Data<MainDbPool>) -> f64 {
    let status = pool.0.status();
    let size = status.size as f64;
    let available = status.available.max(0) as f64;
    if size > 0.0 {
        ((size - available) / size) * 100.0
    } else {
        0.0
    }
}

/// Sum all samples of a counter family (counters are labelled per endpoint/etc.).
fn family_counter(family: &prometheus::proto::MetricFamily) -> f64 {
    family
        .get_metric()
        .iter()
        .filter(|m| m.has_counter())
        .map(|m| m.get_counter().get_value())
        .sum()
}

fn family_gauge(family: &prometheus::proto::MetricFamily) -> f64 {
    family
        .get_metric()
        .first()
        .filter(|m| m.has_gauge())
        .map(|m| m.get_gauge().get_value())
        .unwrap_or(0.0)
}

/// Estimate a histogram percentile (linear interpolation) in milliseconds.
fn hist_percentile_ms(h: &prometheus::proto::Histogram, q: f64) -> Option<f64> {
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
            return Some((prev + (ub - prev) * f) * 1000.0);
        }
        cum = bcum;
        prev = ub;
    }
    Some(prev * 1000.0)
}

fn collect_pool_metrics(pool: &web::Data<MainDbPool>, max_size: usize) {
    let status = pool.0.status();
    let size = status.size;
    let available = status.available; // Available connections

    // "Active" (in use) = Size - Available.
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
        metrics::DB_POOL_UTILIZATION.set(util as i64); // Casting to i64 for IntGauge
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_family_counter_and_gauge() {
        let mut fam = prometheus::proto::MetricFamily::default();
        let mut m = prometheus::proto::Metric::default();
        let mut c = prometheus::proto::Counter::default();
        c.set_value(42.0);
        m.set_counter(c);
        fam.mut_metric().push(m);

        assert_eq!(family_counter(&fam), 42.0);

        let mut gfam = prometheus::proto::MetricFamily::default();
        let mut gm = prometheus::proto::Metric::default();
        let mut g = prometheus::proto::Gauge::default();
        g.set_value(123.5);
        gm.set_gauge(g);
        gfam.mut_metric().push(gm);

        assert_eq!(family_gauge(&gfam), 123.5);
    }

    #[test]
    fn test_hist_percentile_ms_calculation() {
        let mut h = prometheus::proto::Histogram::default();
        assert!(hist_percentile_ms(&h, 0.95).is_none());

        h.set_sample_count(100);
        h.set_sample_sum(5.0);

        let mut b1 = prometheus::proto::Bucket::default();
        b1.set_upper_bound(0.05);
        b1.set_cumulative_count(50);
        h.mut_bucket().push(b1);

        let mut b2 = prometheus::proto::Bucket::default();
        b2.set_upper_bound(0.10);
        b2.set_cumulative_count(95);
        h.mut_bucket().push(b2);

        let mut b3 = prometheus::proto::Bucket::default();
        b3.set_upper_bound(0.50);
        b3.set_cumulative_count(100);
        h.mut_bucket().push(b3);

        let p95 = hist_percentile_ms(&h, 0.95);
        assert!(p95.is_some());
        assert!((p95.unwrap() - 100.0).abs() < 1.0); // 0.10s = 100ms
    }

    #[test]
    fn test_sampler_struct_initialization() {
        let sampler = Sampler::new();
        assert!(sampler.last_counters.is_empty());
        assert!(sampler.last_tick.is_none());
        assert!(sampler.last_backlog.is_none());
        assert!(sampler.last_prune.is_none());
    }

    #[actix_web::test]
    async fn test_pool_util_and_collect_pool_metrics() {
        let (pool, _db) = crate::test_utils::setup_test_database_with_instance().await;
        let main_pool = web::Data::new(crate::db::MainDbPool(pool));
        let util = pool_util(&main_pool);
        assert!(util >= 0.0);
        collect_pool_metrics(&main_pool, 16);
    }
}
