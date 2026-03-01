use crate::config::Config;
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt};
use opentelemetry::KeyValue;
use opentelemetry_sdk::{trace, Resource};
use std::env;
use opentelemetry_otlp::WithExportConfig;

pub fn init_telemetry(config: &Config) -> Result<(), Box<dyn std::error::Error>> {
    // 1. Determine OTLP Endpoint (Config > Env > None)
    let otlp_endpoint = config.otlp_endpoint.clone()
        .or_else(|| env::var("OTEL_EXPORTER_OTLP_ENDPOINT").ok());

    // If we are sending traces (OTEL configured), we likely want JSON logs for Loki correlation.
    // Otherwise (local dev), keep it compact/pretty.
    let use_json_logs = otlp_endpoint.is_some();

    let env_filter = tracing_subscriber::EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info"));

    // 2. Optional: Setup OpenTelemetry exporter if endpoint is configured
    let telemetry_layer = if let Some(endpoint) = otlp_endpoint {
        let version = env::var("APP_VERSION").unwrap_or_else(|_| "0.1.0".to_string());
        let environment = config.environment.clone()
            .or_else(|| env::var("ENVIRONMENT").ok())
            .unwrap_or_else(|| "development".to_string());

        let resource = Resource::new(vec![
            KeyValue::new("service.name", "reminisce"),
            KeyValue::new("service.version", version),
            KeyValue::new("deployment.environment", environment),
        ]);

        let sample_rate = env::var("OTEL_TRACE_SAMPLE_RATE")
            .ok()
            .and_then(|s| s.parse::<f64>().ok())
            .unwrap_or(1.0)
            .clamp(0.0, 1.0);

        let sampler = trace::Sampler::ParentBased(Box::new(trace::Sampler::TraceIdRatioBased(sample_rate)));

        let exporter = opentelemetry_otlp::new_exporter()
            .tonic()
            .with_endpoint(endpoint);
        
        let tracer = opentelemetry_otlp::new_pipeline()
            .tracing()
            .with_exporter(exporter)
            .with_trace_config(
                trace::config()
                    .with_resource(resource)
                    .with_sampler(sampler),
            )
            .install_batch(opentelemetry_sdk::runtime::Tokio)?;

        Some(tracing_opentelemetry::layer().with_tracer(tracer))
    } else {
        None
    };

    // We compose the registry differently depending on formatting needs
    let registry = tracing_subscriber::registry()
        .with(env_filter)
        .with(telemetry_layer);

    if use_json_logs {
         // Use JSON formatting for better parsing by Loki/Promtail when in "Observability Mode"
         // This automatically includes span fields like trace_id if the OpenTelemetry layer is active
         registry.with(tracing_subscriber::fmt::layer().json().flatten_event(true)).try_init()?;
    } else {
         // Default compact human-readable for local dev without OTEL
         registry.with(tracing_subscriber::fmt::layer().compact().with_target(false)).try_init()?;
    }

    Ok(())
}
