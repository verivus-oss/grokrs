//! Telemetry helpers for HTTP request span instrumentation.
//!
//! This module provides span creation and attribute recording for HTTP requests
//! made through [`HttpClient`](super::client::HttpClient). All functionality is
//! gated behind the `otel` cargo feature flag. When the feature is disabled, no
//! spans are created and there is zero runtime overhead.
//!
//! ## Span hierarchy
//!
//! ```text
//! session > agent_iteration > tool_call > http_request
//! ```
//!
//! This module handles the `http_request` leaf span. Higher-level spans
//! (session, agent iteration, tool call) are created by their respective crates.
//!
//! ## Recorded attributes
//!
//! | Attribute           | Source                       |
//! |---------------------|------------------------------|
//! | `http.method`       | Request method               |
//! | `http.url`          | Full request URL (path only) |
//! | `http.status_code`  | Response status code         |
//! | `http.latency_ms`   | Wall-clock request duration  |
//! | `grokrs.model`      | Model name from request body |
//! | `grokrs.input_tokens`  | Token count from response |
//! | `grokrs.output_tokens` | Token count from response |
//! | `grokrs.cost_usd`      | Estimated cost              |

#[cfg(feature = "otel")]
use tracing::Span;

/// Metadata captured from the request before it is sent.
///
/// Constructed by [`RequestMeta::new`] and passed to [`record_response`]
/// after the response is received.
#[derive(Debug, Clone)]
pub struct RequestMeta {
    /// HTTP method (GET, POST, etc.).
    pub method: String,
    /// URL path (without base URL, e.g., "/v1/responses").
    pub path: String,
    /// Model name extracted from the request body, if present.
    pub model: Option<String>,
    /// Timestamp when the request was initiated.
    pub started_at: std::time::Instant,
}

impl RequestMeta {
    /// Create a new `RequestMeta` from request parameters.
    ///
    /// The `body_bytes` parameter is used to extract the model name when
    /// available. This is best-effort: if the body is not JSON or does not
    /// contain a `model` field, the model attribute is omitted.
    pub fn new(method: &str, path: &str, body_bytes: Option<&[u8]>) -> Self {
        let model = body_bytes.and_then(|bytes| {
            serde_json::from_slice::<serde_json::Value>(bytes)
                .ok()
                .and_then(|v| v.get("model")?.as_str().map(String::from))
        });

        Self {
            method: method.to_string(),
            path: path.to_string(),
            model,
            started_at: std::time::Instant::now(),
        }
    }
}

/// Response metadata used to finalize the span after the response is received.
#[derive(Debug, Clone)]
pub struct ResponseMeta {
    /// HTTP status code.
    pub status_code: u16,
    /// Token usage from the response, if available.
    pub input_tokens: Option<u64>,
    /// Token usage from the response, if available.
    pub output_tokens: Option<u64>,
    /// Estimated cost in USD, if calculable.
    pub cost_usd: Option<f64>,
}

/// Create a tracing span for an HTTP request and return it.
///
/// When the `otel` feature is disabled, this returns `None` and is optimized
/// away entirely.
#[cfg(feature = "otel")]
pub fn begin_http_span(meta: &RequestMeta) -> Span {
    let span = tracing::info_span!(
        "http_request",
        http.method = %meta.method,
        http.url = %meta.path,
        http.status_code = tracing::field::Empty,
        http.latency_ms = tracing::field::Empty,
        grokrs.model = tracing::field::Empty,
        grokrs.input_tokens = tracing::field::Empty,
        grokrs.output_tokens = tracing::field::Empty,
        grokrs.cost_usd = tracing::field::Empty,
    );

    if let Some(ref model) = meta.model {
        span.record("grokrs.model", model.as_str());
    }

    span
}

/// No-op version when otel is disabled.
#[cfg(not(feature = "otel"))]
pub fn begin_http_span(_meta: &RequestMeta) -> Option<()> {
    None
}

/// Record response attributes on the current span.
///
/// When the `otel` feature is disabled, this is a no-op.
#[cfg(feature = "otel")]
pub fn record_response(span: &Span, meta: &RequestMeta, response: &ResponseMeta) {
    let latency_ms = meta.started_at.elapsed().as_millis() as u64;

    span.record("http.status_code", response.status_code);
    span.record("http.latency_ms", latency_ms);

    if let Some(input_tokens) = response.input_tokens {
        span.record("grokrs.input_tokens", input_tokens);
    }
    if let Some(output_tokens) = response.output_tokens {
        span.record("grokrs.output_tokens", output_tokens);
    }
    if let Some(cost_usd) = response.cost_usd {
        span.record("grokrs.cost_usd", cost_usd);
    }
}

/// No-op version when otel is disabled.
#[cfg(not(feature = "otel"))]
pub fn record_response(_span: &Option<()>, _meta: &RequestMeta, _response: &ResponseMeta) {}

/// Extract token usage from a raw response byte slice.
///
/// Looks for the standard `usage.input_tokens` and `usage.output_tokens` fields
/// in the JSON response body. Returns `(None, None)` if not present or if the
/// response is not valid JSON.
pub fn extract_usage_from_bytes(bytes: &[u8]) -> (Option<u64>, Option<u64>) {
    let Ok(value) = serde_json::from_slice::<serde_json::Value>(bytes) else {
        return (None, None);
    };
    let usage = match value.get("usage") {
        Some(u) => u,
        None => return (None, None),
    };
    let input = usage
        .get("input_tokens")
        .or_else(|| usage.get("prompt_tokens"))
        .and_then(|v| v.as_u64());
    let output = usage
        .get("output_tokens")
        .or_else(|| usage.get("completion_tokens"))
        .and_then(|v| v.as_u64());
    (input, output)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn request_meta_extracts_model_from_body() {
        let body = br#"{"model": "grok-3", "input": "hello"}"#;
        let meta = RequestMeta::new("POST", "/v1/responses", Some(body));
        assert_eq!(meta.model, Some("grok-3".to_string()));
        assert_eq!(meta.method, "POST");
        assert_eq!(meta.path, "/v1/responses");
    }

    #[test]
    fn request_meta_handles_no_model() {
        let body = br#"{"input": "hello"}"#;
        let meta = RequestMeta::new("POST", "/v1/responses", Some(body));
        assert_eq!(meta.model, None);
    }

    #[test]
    fn request_meta_handles_no_body() {
        let meta = RequestMeta::new("GET", "/v1/models", None);
        assert_eq!(meta.model, None);
        assert_eq!(meta.method, "GET");
    }

    #[test]
    fn request_meta_handles_invalid_json_body() {
        let body = b"not json";
        let meta = RequestMeta::new("POST", "/v1/responses", Some(body));
        assert_eq!(meta.model, None);
    }

    #[test]
    fn extract_usage_from_valid_response() {
        let resp = br#"{"id":"resp_1","usage":{"input_tokens":100,"output_tokens":50}}"#;
        let (input, output) = extract_usage_from_bytes(resp);
        assert_eq!(input, Some(100));
        assert_eq!(output, Some(50));
    }

    #[test]
    fn extract_usage_from_chat_completion_response() {
        let resp = br#"{"id":"chatcmpl-1","usage":{"prompt_tokens":200,"completion_tokens":80}}"#;
        let (input, output) = extract_usage_from_bytes(resp);
        assert_eq!(input, Some(200));
        assert_eq!(output, Some(80));
    }

    #[test]
    fn extract_usage_from_response_without_usage() {
        let resp = br#"{"id":"resp_1","output":[]}"#;
        let (input, output) = extract_usage_from_bytes(resp);
        assert_eq!(input, None);
        assert_eq!(output, None);
    }

    #[test]
    fn extract_usage_from_invalid_json() {
        let resp = b"not json";
        let (input, output) = extract_usage_from_bytes(resp);
        assert_eq!(input, None);
        assert_eq!(output, None);
    }

    #[test]
    fn response_meta_debug_impl() {
        let meta = ResponseMeta {
            status_code: 200,
            input_tokens: Some(100),
            output_tokens: Some(50),
            cost_usd: Some(0.001),
        };
        let debug = format!("{meta:?}");
        assert!(debug.contains("200"));
        assert!(debug.contains("100"));
    }

    #[cfg(feature = "otel")]
    mod otel_tests {
        use super::*;

        #[test]
        fn begin_http_span_creates_span_with_attributes() {
            // Set up a subscriber so spans are actually processed
            let _guard = tracing_subscriber::fmt().with_test_writer().try_init();

            let meta = RequestMeta::new("POST", "/v1/responses", Some(br#"{"model":"grok-3"}"#));
            let span = begin_http_span(&meta);

            // Verify the span exists (we can't easily inspect attributes
            // without a custom subscriber, but we verify it doesn't panic)
            let _enter = span.enter();
        }

        #[test]
        fn record_response_sets_attributes() {
            let _guard = tracing_subscriber::fmt().with_test_writer().try_init();

            let req_meta =
                RequestMeta::new("POST", "/v1/responses", Some(br#"{"model":"grok-3"}"#));
            let span = begin_http_span(&req_meta);
            let _enter = span.enter();

            let resp_meta = ResponseMeta {
                status_code: 200,
                input_tokens: Some(150),
                output_tokens: Some(75),
                cost_usd: Some(0.002),
            };
            record_response(&span, &req_meta, &resp_meta);
            // No panic = success; attribute verification requires a custom Layer,
            // which is tested in the integration-level tests.
        }
    }

    #[cfg(not(feature = "otel"))]
    mod no_otel_tests {
        use super::*;

        #[test]
        fn begin_http_span_returns_none_without_otel() {
            let meta = RequestMeta::new("GET", "/v1/models", None);
            let result = begin_http_span(&meta);
            assert!(result.is_none());
        }

        #[test]
        fn record_response_is_noop_without_otel() {
            let req_meta = RequestMeta::new("GET", "/v1/models", None);
            let resp_meta = ResponseMeta {
                status_code: 200,
                input_tokens: None,
                output_tokens: None,
                cost_usd: None,
            };
            // Should compile and not panic
            record_response(&None, &req_meta, &resp_meta);
        }
    }
}
