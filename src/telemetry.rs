//! Logging/telemetry setup.
//!
//! Lightweight by design: no OTLP/OpenTelemetry. `tracing` events go to
//!   - stdout (JSON, so `docker logs` / our rotating files are greppable), and
//!   - the in-process ring buffer (see `crate::logtail`), which backs the
//!     in-app `/api/admin/logs`, `/api/admin/errors` and `/api/admin/alerts`.
//!
//! When `config.log_dir` is set, events are additionally mirrored to
//! hourly-rotating JSON files (kept for `logtail::MAX_LOG_FILES`) that survive
//! restarts and are served by `GET /api/admin/logs?full=1`.

use crate::config::Config;
use crate::logtail;
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt};

pub fn init_telemetry(config: &Config) -> Result<(), Box<dyn std::error::Error>> {
    let env_filter = tracing_subscriber::EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info"));

    let store = logtail::ring();
    logtail::set_log_dir(config.log_dir.as_deref());
    logtail::install_panic_hook();

    // stdout + optionally the rotating file; both get JSON lines (Loki-style,
    // no Loki needed).
    let writer = logtail::file_writer(config.log_dir.is_none());

    let fmt_layer = tracing_subscriber::fmt::layer()
        .json()
        .flatten_event(true)
        .with_ansi(false)
        .with_writer(writer);

    // Bounded in-memory ring of structured events for the in-app views/alerts.
    let ring_layer = logtail::RingLayer::new(store);

    tracing_subscriber::registry()
        .with(env_filter)
        .with(fmt_layer)
        .with(ring_layer)
        .try_init()?;

    Ok(())
}