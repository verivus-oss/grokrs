//! OpenTelemetry initialization and CLI flag handling.
//!
//! This module provides the telemetry initialization logic for the grokrs CLI.
//! All functionality is gated behind the `otel` cargo feature flag. When the
//! feature is disabled, the public API compiles to no-ops with zero overhead.
//!
//! ## Configuration
//!
//! The OTLP exporter endpoint can be configured via:
//!
//! 1. `--otel-endpoint <URL>` CLI flag (highest priority)
//! 2. `GROKRS_OTEL_ENDPOINT` environment variable
//!
//! If neither is set and the `otel` feature is enabled, telemetry is silently
//! disabled (no spans are exported, but `tracing` macros remain zero-cost).
//!
//! ## Span hierarchy
//!
//! ```text
//! session > agent_iteration > tool_call > http_request
//! ```

/// Guard that shuts down the telemetry pipeline when dropped.
///
/// When the `otel` feature is disabled, this is a zero-size type.
pub struct TelemetryGuard {
    #[cfg(feature = "otel")]
    _inner: Option<TelemetryGuardInner>,
}

#[cfg(feature = "otel")]
struct TelemetryGuardInner {
    _provider: opentelemetry_sdk::trace::TracerProvider,
}

#[cfg(feature = "otel")]
impl Drop for TelemetryGuardInner {
    fn drop(&mut self) {
        // Flush remaining spans on shutdown.
        if let Err(e) = self._provider.shutdown() {
            eprintln!("[otel] failed to shutdown tracer provider: {e}");
        }
    }
}

/// Initialize the telemetry pipeline.
///
/// When the `otel` feature is enabled and an endpoint is provided (via CLI
/// flag or environment variable), this sets up:
///
/// 1. An OTLP gRPC exporter targeting the given endpoint.
/// 2. A `tracing-opentelemetry` layer that bridges `tracing` spans to OTEL.
/// 3. A `tracing-subscriber` registry that processes spans.
///
/// Returns a [`TelemetryGuard`] that flushes spans on drop.
///
/// When the `otel` feature is disabled, this is a no-op that returns
/// immediately.
///
/// # Arguments
///
/// * `endpoint` - Optional OTLP endpoint URL. If `None`, falls back to the
///   `GROKRS_OTEL_ENDPOINT` environment variable. If neither is set,
///   telemetry is silently disabled.
#[cfg(feature = "otel")]
pub fn init(endpoint: Option<&str>) -> TelemetryGuard {
    use opentelemetry::trace::TracerProvider as _;
    use opentelemetry_otlp::{SpanExporter, WithExportConfig};
    use opentelemetry_sdk::trace::TracerProvider;
    use tracing_subscriber::layer::SubscriberExt;
    use tracing_subscriber::util::SubscriberInitExt;

    let resolved_endpoint = endpoint
        .map(String::from)
        .or_else(|| std::env::var("GROKRS_OTEL_ENDPOINT").ok());

    let Some(endpoint_url) = resolved_endpoint else {
        // No endpoint configured; telemetry is disabled but tracing macros
        // still compile to no-ops (no subscriber installed for OTEL layer).
        return TelemetryGuard { _inner: None };
    };

    // Build the OTLP exporter.
    let exporter = match SpanExporter::builder()
        .with_tonic()
        .with_endpoint(&endpoint_url)
        .build()
    {
        Ok(e) => e,
        Err(err) => {
            eprintln!("[otel] failed to build OTLP exporter for '{endpoint_url}': {err}");
            return TelemetryGuard { _inner: None };
        }
    };

    // Build the tracer provider with a batch span processor.
    let provider = TracerProvider::builder()
        .with_batch_exporter(exporter, opentelemetry_sdk::runtime::Tokio)
        .build();

    let tracer = provider.tracer("grokrs");

    // Build the tracing-opentelemetry layer.
    let otel_layer = tracing_opentelemetry::layer().with_tracer(tracer);

    // Install the subscriber.
    let subscriber = tracing_subscriber::registry()
        .with(tracing_subscriber::EnvFilter::from_default_env())
        .with(otel_layer);

    if let Err(e) = subscriber.try_init() {
        eprintln!("[otel] failed to initialize tracing subscriber: {e}");
        return TelemetryGuard { _inner: None };
    }

    TelemetryGuard {
        _inner: Some(TelemetryGuardInner {
            _provider: provider,
        }),
    }
}

/// No-op initialization when `otel` feature is disabled.
#[cfg(not(feature = "otel"))]
pub fn init(_endpoint: Option<&str>) -> TelemetryGuard {
    TelemetryGuard {}
}

/// Resolve the OTLP endpoint from CLI flag and environment variable.
///
/// Priority: CLI flag > `GROKRS_OTEL_ENDPOINT` env var > None.
pub fn resolve_endpoint(cli_flag: Option<&str>) -> Option<String> {
    cli_flag
        .map(String::from)
        .or_else(|| std::env::var("GROKRS_OTEL_ENDPOINT").ok())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resolve_endpoint_prefers_cli_flag() {
        std::env::set_var("GROKRS_OTEL_ENDPOINT", "http://env:4317");
        let result = resolve_endpoint(Some("http://cli:4317"));
        std::env::remove_var("GROKRS_OTEL_ENDPOINT");
        assert_eq!(result, Some("http://cli:4317".to_string()));
    }

    #[test]
    fn resolve_endpoint_falls_back_to_env() {
        std::env::set_var("GROKRS_OTEL_ENDPOINT", "http://env:4317");
        let result = resolve_endpoint(None);
        std::env::remove_var("GROKRS_OTEL_ENDPOINT");
        assert_eq!(result, Some("http://env:4317".to_string()));
    }

    #[test]
    fn resolve_endpoint_returns_none_when_unset() {
        std::env::remove_var("GROKRS_OTEL_ENDPOINT");
        let result = resolve_endpoint(None);
        assert_eq!(result, None);
    }

    #[test]
    fn telemetry_guard_is_send_and_sync() {
        fn assert_send_sync<T: Send + Sync>() {}
        // TelemetryGuard must be Send+Sync so it can be held across await points.
        // This is a compile-time check.
        assert_send_sync::<TelemetryGuard>();
    }

    #[test]
    fn init_without_endpoint_returns_guard() {
        std::env::remove_var("GROKRS_OTEL_ENDPOINT");
        let _guard = init(None);
        // Should not panic; no exporter is configured.
    }
}
