//! Tracing initialization shared by all Rust services.
//!
//! Honesty about observability: log output is fully wired (`LOG_LEVEL`,
//! overridable per-service with `RUST_LOG`). OTLP export is *not* wired yet —
//! when `OTEL_EXPORTER_OTLP_ENDPOINT` is configured we warn instead of
//! silently dropping spans.

use tracing_subscriber::EnvFilter;

/// Initialize global tracing. Call exactly once per process, before any
/// `tracing` event that must be captured.
pub fn init(log_level: &str, otel_endpoint: Option<&str>) {
    let filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new(log_level));

    tracing_subscriber::fmt()
        .with_env_filter(filter)
        .with_target(false)
        .init();

    if otel_endpoint.is_some() {
        tracing::warn!(
            "OTEL_EXPORTER_OTLP_ENDPOINT is set, but no OTLP exporter is configured yet; \
             spans and metrics will not be exported"
        );
    }
}
