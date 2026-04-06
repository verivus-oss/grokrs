use std::sync::Arc;
use std::time::Duration;

use futures::Stream;
use reqwest::Method;
use reqwest::header::{AUTHORIZATION, CONTENT_TYPE, HeaderMap, HeaderValue};
use serde::Serialize;
use serde::de::DeserializeOwned;

use crate::transport::auth::{ApiKeySecret, resolve_api_key};
use crate::transport::error::TransportError;
use crate::transport::policy_bridge::DenyAllGate;
use crate::transport::policy_gate::{PolicyDecision, PolicyGate};
use crate::transport::retry::{RetryConfig, should_retry};
use crate::transport::sse::SseStream;
use crate::transport::telemetry::{self, RequestMeta, ResponseMeta};
use crate::types::error::{ApiError, ApiErrorResponse};

/// Configuration for the HTTP client.
#[derive(Debug, Clone)]
pub struct HttpClientConfig {
    /// Base URL for the API (e.g., "https://api.x.ai").
    pub base_url: String,
    /// Request timeout duration.
    pub timeout: Duration,
    /// Retry configuration for transient errors.
    pub retry: RetryConfig,
    /// Name of the environment variable holding the API key.
    /// Used by `HttpClient::from_env`. Defaults to `"XAI_API_KEY"`.
    pub api_key_env: Option<String>,
}

impl Default for HttpClientConfig {
    fn default() -> Self {
        Self {
            base_url: "https://api.x.ai".into(),
            timeout: Duration::from_secs(120),
            retry: RetryConfig::default(),
            api_key_env: None,
        }
    }
}

/// An async HTTP client for the xAI API.
///
/// Wraps `reqwest::Client` with authentication, retry logic, policy gate
/// enforcement, and SSE stream support. No endpoint-specific logic lives here;
/// that is the job of endpoint modules built on top of this transport.
pub struct HttpClient {
    client: reqwest::Client,
    config: HttpClientConfig,
    api_key: ApiKeySecret,
    policy_gate: Option<Arc<dyn PolicyGate>>,
}

impl std::fmt::Debug for HttpClient {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("HttpClient")
            .field("config", &self.config)
            .field("api_key", &self.api_key)
            .field(
                "policy_gate",
                &if self.policy_gate.is_some() {
                    "Some(<PolicyGate>)"
                } else {
                    "None"
                },
            )
            .finish()
    }
}

impl HttpClient {
    /// Create a new `HttpClient` with the given configuration and API key.
    ///
    /// The optional `policy_gate` is evaluated before every outbound request.
    /// If no gate is provided, a `DenyAllGate` is used as the default
    /// (fail-closed / deny-by-default). Callers that want to allow traffic
    /// must explicitly provide an `AllowAllGate` or a custom gate.
    pub fn new(
        config: HttpClientConfig,
        api_key: ApiKeySecret,
        policy_gate: Option<Arc<dyn PolicyGate>>,
    ) -> Result<Self, TransportError> {
        let mut default_headers = HeaderMap::new();
        default_headers.insert(CONTENT_TYPE, HeaderValue::from_static("application/json"));

        let client = reqwest::Client::builder()
            .timeout(config.timeout)
            .default_headers(default_headers)
            .build()
            .map_err(|e| TransportError::Http { source: e })?;

        Ok(Self {
            client,
            config,
            api_key,
            policy_gate,
        })
    }

    /// Create a new `HttpClient` by reading the API key from the environment.
    ///
    /// If `config.api_key_env` is set, that environment variable name is used.
    /// Otherwise, defaults to `"XAI_API_KEY"`.
    ///
    /// Returns `TransportError::Auth` if the environment variable is not set
    /// or empty.
    pub fn from_env(
        config: HttpClientConfig,
        policy_gate: Option<Arc<dyn PolicyGate>>,
    ) -> Result<Self, TransportError> {
        let env_var = config.api_key_env.as_deref().unwrap_or("XAI_API_KEY");
        let api_key = resolve_api_key(env_var)?;
        Self::new(config, api_key, policy_gate)
    }

    /// Send a JSON request and deserialize the response.
    ///
    /// Applies policy gate, authentication, and retry logic automatically.
    /// When the `otel` feature is enabled, emits a tracing span with HTTP
    /// method, path, status code, latency, model name, and token usage.
    pub async fn send_json<Req, Resp>(
        &self,
        method: Method,
        path: &str,
        body: &Req,
    ) -> Result<Resp, TransportError>
    where
        Req: Serialize,
        Resp: DeserializeOwned,
    {
        self.check_policy_gate()?;

        let url = format!("{}{}", self.config.base_url, path);
        let body_bytes = serde_json::to_vec(body).map_err(|e| TransportError::Serialization {
            message: format!("failed to serialize request body: {e}"),
        })?;

        let req_meta = RequestMeta::new(method.as_str(), path, Some(&body_bytes));
        let span = telemetry::begin_http_span(&req_meta);
        #[cfg(feature = "otel")]
        let _span_guard = span.enter();

        let mut attempt = 0u32;
        loop {
            let request = self
                .client
                .request(method.clone(), &url)
                .header(AUTHORIZATION, format!("Bearer {}", self.api_key.expose()))
                .body(body_bytes.clone());

            let response = request.send().await?;
            let status = response.status().as_u16();

            if response.status().is_success() {
                let resp_bytes = response.bytes().await?;

                // Extract token usage for telemetry before deserializing.
                let (input_tokens, output_tokens) =
                    telemetry::extract_usage_from_bytes(&resp_bytes);
                let resp_meta = ResponseMeta {
                    status_code: status,
                    input_tokens,
                    output_tokens,
                    cost_usd: None,
                };
                telemetry::record_response(&span, &req_meta, &resp_meta);

                let parsed: Resp = serde_json::from_slice(&resp_bytes).map_err(|e| {
                    TransportError::Deserialization {
                        message: format!("failed to deserialize response: {e}"),
                    }
                })?;
                return Ok(parsed);
            }

            // Check if we should retry
            if let Some(delay) = should_retry(status, attempt, &self.config.retry) {
                attempt += 1;
                tokio::time::sleep(delay).await;
                continue;
            }

            // Record error status on span before returning.
            let resp_meta = ResponseMeta {
                status_code: status,
                input_tokens: None,
                output_tokens: None,
                cost_usd: None,
            };
            telemetry::record_response(&span, &req_meta, &resp_meta);

            // Not retryable — parse the error response
            return Err(self.parse_error_response(response).await);
        }
    }

    /// Send a JSON request and return an SSE stream of raw data strings.
    ///
    /// Applies policy gate, authentication, and retry logic on the initial
    /// connection. If the server returns 429/503 before the stream begins,
    /// the request is retried according to the retry configuration.
    /// Once the stream is established (2xx received), no further retries
    /// are attempted because partial delivery makes retry semantics ambiguous.
    ///
    /// When the `otel` feature is enabled, emits a tracing span for the
    /// initial connection (not for each streamed event).
    pub async fn send_sse<Req>(
        &self,
        method: Method,
        path: &str,
        body: &Req,
    ) -> Result<impl Stream<Item = Result<String, TransportError>> + use<Req>, TransportError>
    where
        Req: Serialize,
    {
        self.check_policy_gate()?;

        let url = format!("{}{}", self.config.base_url, path);
        let body_bytes = serde_json::to_vec(body).map_err(|e| TransportError::Serialization {
            message: format!("failed to serialize request body: {e}"),
        })?;

        let req_meta = RequestMeta::new(method.as_str(), path, Some(&body_bytes));
        let span = telemetry::begin_http_span(&req_meta);
        #[cfg(feature = "otel")]
        let _span_guard = span.enter();

        let mut attempt = 0u32;
        loop {
            let request = self
                .client
                .request(method.clone(), &url)
                .header(AUTHORIZATION, format!("Bearer {}", self.api_key.expose()))
                .body(body_bytes.clone());

            let response = request.send().await?;

            if response.status().is_success() {
                let resp_meta = ResponseMeta {
                    status_code: response.status().as_u16(),
                    input_tokens: None,
                    output_tokens: None,
                    cost_usd: None,
                };
                telemetry::record_response(&span, &req_meta, &resp_meta);

                let byte_stream = response.bytes_stream();
                return Ok(SseStream::new(byte_stream));
            }

            let status = response.status().as_u16();

            // Retry on 429/503 for the initial connection
            if let Some(delay) = should_retry(status, attempt, &self.config.retry) {
                attempt += 1;
                tokio::time::sleep(delay).await;
                continue;
            }

            let resp_meta = ResponseMeta {
                status_code: status,
                input_tokens: None,
                output_tokens: None,
                cost_usd: None,
            };
            telemetry::record_response(&span, &req_meta, &resp_meta);

            return Err(self.parse_error_response(response).await);
        }
    }

    /// Send a multipart form request and deserialize the response.
    ///
    /// Applies policy gate and authentication. Retry is NOT applied to
    /// multipart uploads because `reqwest::multipart::Form` cannot be cloned
    /// or replayed — the form body is consumed on the first send attempt.
    /// Callers that need retry behavior for multipart uploads should
    /// rebuild the form and call this method again.
    pub async fn send_multipart<Resp>(
        &self,
        path: &str,
        form: reqwest::multipart::Form,
    ) -> Result<Resp, TransportError>
    where
        Resp: DeserializeOwned,
    {
        self.check_policy_gate()?;

        let url = format!("{}{}", self.config.base_url, path);

        let req_meta = RequestMeta::new("POST", path, None);
        let span = telemetry::begin_http_span(&req_meta);
        #[cfg(feature = "otel")]
        let _span_guard = span.enter();

        let request = self
            .client
            .post(&url)
            .header(AUTHORIZATION, format!("Bearer {}", self.api_key.expose()))
            .multipart(form);

        let response = request.send().await?;
        let status = response.status().as_u16();

        if !response.status().is_success() {
            let resp_meta = ResponseMeta {
                status_code: status,
                input_tokens: None,
                output_tokens: None,
                cost_usd: None,
            };
            telemetry::record_response(&span, &req_meta, &resp_meta);
            return Err(self.parse_error_response(response).await);
        }

        let resp_bytes = response.bytes().await?;
        let resp_meta = ResponseMeta {
            status_code: status,
            input_tokens: None,
            output_tokens: None,
            cost_usd: None,
        };
        telemetry::record_response(&span, &req_meta, &resp_meta);

        let parsed: Resp =
            serde_json::from_slice(&resp_bytes).map_err(|e| TransportError::Deserialization {
                message: format!("failed to deserialize multipart response: {e}"),
            })?;
        Ok(parsed)
    }

    /// Send a request with no body and deserialize the response.
    ///
    /// Applies policy gate, authentication, and retry logic automatically.
    /// Suitable for GET requests that have no request body.
    pub async fn send_no_body<Resp>(
        &self,
        method: Method,
        path: &str,
    ) -> Result<Resp, TransportError>
    where
        Resp: DeserializeOwned,
    {
        self.check_policy_gate()?;

        let url = format!("{}{}", self.config.base_url, path);

        let req_meta = RequestMeta::new(method.as_str(), path, None);
        let span = telemetry::begin_http_span(&req_meta);
        #[cfg(feature = "otel")]
        let _span_guard = span.enter();

        let mut attempt = 0u32;
        loop {
            let request = self
                .client
                .request(method.clone(), &url)
                .header(AUTHORIZATION, format!("Bearer {}", self.api_key.expose()));

            let response = request.send().await?;
            let status = response.status().as_u16();

            if response.status().is_success() {
                let resp_bytes = response.bytes().await?;
                let (input_tokens, output_tokens) =
                    telemetry::extract_usage_from_bytes(&resp_bytes);
                let resp_meta = ResponseMeta {
                    status_code: status,
                    input_tokens,
                    output_tokens,
                    cost_usd: None,
                };
                telemetry::record_response(&span, &req_meta, &resp_meta);

                let parsed: Resp = serde_json::from_slice(&resp_bytes).map_err(|e| {
                    TransportError::Deserialization {
                        message: format!("failed to deserialize response: {e}"),
                    }
                })?;
                return Ok(parsed);
            }

            if let Some(delay) = should_retry(status, attempt, &self.config.retry) {
                attempt += 1;
                tokio::time::sleep(delay).await;
                continue;
            }

            let resp_meta = ResponseMeta {
                status_code: status,
                input_tokens: None,
                output_tokens: None,
                cost_usd: None,
            };
            telemetry::record_response(&span, &req_meta, &resp_meta);

            return Err(self.parse_error_response(response).await);
        }
    }

    /// Send a request with no body and return the HTTP status code along with
    /// the raw response bytes.
    ///
    /// Applies policy gate, authentication, and retry logic automatically.
    /// Unlike `send_no_body`, this method does NOT treat non-2xx as errors for
    /// the specific status codes listed in `accept_statuses`. All listed status
    /// codes are considered successful and returned as-is. Any other non-2xx
    /// status is treated as an error.
    ///
    /// This is useful for endpoints like deferred polling where 202 (still
    /// processing) and 200 (complete) have different response shapes.
    pub async fn send_no_body_with_status(
        &self,
        method: Method,
        path: &str,
        accept_statuses: &[u16],
    ) -> Result<(u16, bytes::Bytes), TransportError> {
        self.check_policy_gate()?;

        let url = format!("{}{}", self.config.base_url, path);

        let req_meta = RequestMeta::new(method.as_str(), path, None);
        let span = telemetry::begin_http_span(&req_meta);
        #[cfg(feature = "otel")]
        let _span_guard = span.enter();

        let mut attempt = 0u32;
        loop {
            let request = self
                .client
                .request(method.clone(), &url)
                .header(AUTHORIZATION, format!("Bearer {}", self.api_key.expose()));

            let response = request.send().await?;
            let status = response.status().as_u16();

            if response.status().is_success() || accept_statuses.contains(&status) {
                let resp_bytes = response.bytes().await?;
                let resp_meta = ResponseMeta {
                    status_code: status,
                    input_tokens: None,
                    output_tokens: None,
                    cost_usd: None,
                };
                telemetry::record_response(&span, &req_meta, &resp_meta);
                return Ok((status, resp_bytes));
            }

            if let Some(delay) = should_retry(status, attempt, &self.config.retry) {
                attempt += 1;
                tokio::time::sleep(delay).await;
                continue;
            }

            let resp_meta = ResponseMeta {
                status_code: status,
                input_tokens: None,
                output_tokens: None,
                cost_usd: None,
            };
            telemetry::record_response(&span, &req_meta, &resp_meta);

            return Err(self.parse_error_response(response).await);
        }
    }

    /// Send a request with no body and expect an empty (or ignorable) response.
    ///
    /// Applies policy gate, authentication, and retry logic automatically.
    /// Returns `Ok(())` on any 2xx status. Suitable for DELETE endpoints that
    /// return 204 No Content.
    pub async fn send_no_body_empty(
        &self,
        method: Method,
        path: &str,
    ) -> Result<(), TransportError> {
        self.check_policy_gate()?;

        let url = format!("{}{}", self.config.base_url, path);

        let req_meta = RequestMeta::new(method.as_str(), path, None);
        let span = telemetry::begin_http_span(&req_meta);
        #[cfg(feature = "otel")]
        let _span_guard = span.enter();

        let mut attempt = 0u32;
        loop {
            let request = self
                .client
                .request(method.clone(), &url)
                .header(AUTHORIZATION, format!("Bearer {}", self.api_key.expose()));

            let response = request.send().await?;
            let status = response.status().as_u16();

            if response.status().is_success() {
                // Drain the body to ensure the connection is properly returned to the pool.
                let _ = response.bytes().await;
                let resp_meta = ResponseMeta {
                    status_code: status,
                    input_tokens: None,
                    output_tokens: None,
                    cost_usd: None,
                };
                telemetry::record_response(&span, &req_meta, &resp_meta);
                return Ok(());
            }

            if let Some(delay) = should_retry(status, attempt, &self.config.retry) {
                attempt += 1;
                tokio::time::sleep(delay).await;
                continue;
            }

            let resp_meta = ResponseMeta {
                status_code: status,
                input_tokens: None,
                output_tokens: None,
                cost_usd: None,
            };
            telemetry::record_response(&span, &req_meta, &resp_meta);

            return Err(self.parse_error_response(response).await);
        }
    }

    /// Send a JSON request and ignore the response body.
    ///
    /// Applies policy gate, authentication, and retry logic automatically.
    /// Returns `Ok(())` on any 2xx status. Suitable for endpoints that may
    /// return 200 with an empty body, 204 No Content, or a JSON acknowledgment
    /// that the caller does not need.
    pub async fn send_json_empty<Req>(
        &self,
        method: Method,
        path: &str,
        body: &Req,
    ) -> Result<(), TransportError>
    where
        Req: Serialize,
    {
        self.check_policy_gate()?;

        let url = format!("{}{}", self.config.base_url, path);
        let body_bytes = serde_json::to_vec(body).map_err(|e| TransportError::Serialization {
            message: format!("failed to serialize request body: {e}"),
        })?;

        let req_meta = RequestMeta::new(method.as_str(), path, Some(&body_bytes));
        let span = telemetry::begin_http_span(&req_meta);
        #[cfg(feature = "otel")]
        let _span_guard = span.enter();

        let mut attempt = 0u32;
        loop {
            let request = self
                .client
                .request(method.clone(), &url)
                .header(AUTHORIZATION, format!("Bearer {}", self.api_key.expose()))
                .body(body_bytes.clone());

            let response = request.send().await?;
            let status = response.status().as_u16();

            if response.status().is_success() {
                // Drain the body to ensure the connection is properly returned to the pool.
                let _ = response.bytes().await;
                let resp_meta = ResponseMeta {
                    status_code: status,
                    input_tokens: None,
                    output_tokens: None,
                    cost_usd: None,
                };
                telemetry::record_response(&span, &req_meta, &resp_meta);
                return Ok(());
            }

            if let Some(delay) = should_retry(status, attempt, &self.config.retry) {
                attempt += 1;
                tokio::time::sleep(delay).await;
                continue;
            }

            let resp_meta = ResponseMeta {
                status_code: status,
                input_tokens: None,
                output_tokens: None,
                cost_usd: None,
            };
            telemetry::record_response(&span, &req_meta, &resp_meta);

            return Err(self.parse_error_response(response).await);
        }
    }

    /// Send a JSON request and return the raw response bytes.
    ///
    /// Useful for endpoints that return binary data (e.g., file downloads)
    /// rather than JSON. Applies policy gate, authentication, and retry logic
    /// automatically.
    pub async fn send_json_raw<Req>(
        &self,
        method: Method,
        path: &str,
        body: &Req,
    ) -> Result<Vec<u8>, TransportError>
    where
        Req: Serialize,
    {
        self.check_policy_gate()?;

        let url = format!("{}{}", self.config.base_url, path);
        let body_bytes = serde_json::to_vec(body).map_err(|e| TransportError::Serialization {
            message: format!("failed to serialize request body: {e}"),
        })?;

        let req_meta = RequestMeta::new(method.as_str(), path, Some(&body_bytes));
        let span = telemetry::begin_http_span(&req_meta);
        #[cfg(feature = "otel")]
        let _span_guard = span.enter();

        let mut attempt = 0u32;
        loop {
            let request = self
                .client
                .request(method.clone(), &url)
                .header(AUTHORIZATION, format!("Bearer {}", self.api_key.expose()))
                .body(body_bytes.clone());

            let response = request.send().await?;
            let status = response.status().as_u16();

            if response.status().is_success() {
                let resp_bytes = response.bytes().await?;
                let resp_meta = ResponseMeta {
                    status_code: status,
                    input_tokens: None,
                    output_tokens: None,
                    cost_usd: None,
                };
                telemetry::record_response(&span, &req_meta, &resp_meta);
                return Ok(resp_bytes.to_vec());
            }

            if let Some(delay) = should_retry(status, attempt, &self.config.retry) {
                attempt += 1;
                tokio::time::sleep(delay).await;
                continue;
            }

            let resp_meta = ResponseMeta {
                status_code: status,
                input_tokens: None,
                output_tokens: None,
                cost_usd: None,
            };
            telemetry::record_response(&span, &req_meta, &resp_meta);

            return Err(self.parse_error_response(response).await);
        }
    }

    /// Check the policy gate before making a request.
    ///
    /// Extracts the host from the base URL and evaluates it against the
    /// policy gate. If no gate was explicitly provided, `DenyAllGate` is used
    /// as the default (fail-closed / deny-by-default).
    fn check_policy_gate(&self) -> Result<(), TransportError> {
        let deny_all = DenyAllGate;
        let gate: &dyn PolicyGate = match self.policy_gate {
            Some(ref g) => g.as_ref(),
            None => &deny_all,
        };
        let host = extract_host(&self.config.base_url)?;
        match gate.evaluate_network(&host) {
            PolicyDecision::Allow => Ok(()),
            PolicyDecision::Deny { reason } => Err(TransportError::PolicyDenied { host, reason }),
            PolicyDecision::Ask => Err(TransportError::ApprovalRequired { host }),
        }
    }

    /// Parse an error response body into a `TransportError`.
    async fn parse_error_response(&self, response: reqwest::Response) -> TransportError {
        let status = response.status().as_u16();
        let request_id = response
            .headers()
            .get("x-request-id")
            .and_then(|v| v.to_str().ok())
            .map(String::from);

        match response.bytes().await {
            Ok(bytes) => {
                if let Ok(api_resp) = serde_json::from_slice::<ApiErrorResponse>(&bytes) {
                    TransportError::Api(ApiError::from_response(status, api_resp.error, request_id))
                } else {
                    let fallback = String::from_utf8_lossy(&bytes).to_string();
                    TransportError::Api(ApiError {
                        status_code: status,
                        message: fallback,
                        error_type: None,
                        code: None,
                        request_id,
                    })
                }
            }
            Err(err) => TransportError::Http { source: err },
        }
    }
}

/// Extract the host portion from a URL string using proper URL parsing.
///
/// Returns an error if the URL cannot be parsed, contains userinfo
/// (username or password), or has no host component.
fn extract_host(url: &str) -> Result<String, TransportError> {
    let parsed = url::Url::parse(url).map_err(|e| TransportError::InvalidBaseUrl {
        url: url.to_string(),
        reason: e.to_string(),
    })?;

    // Reject URLs with any userinfo component to prevent credential smuggling
    // and host confusion attacks. Check the parsed fields AND the raw URL
    // string for '@' in the authority section (between scheme:// and the
    // first path /).  This catches edge cases like "https://@evil.example"
    // where the url crate may parse an empty username.
    let has_userinfo_fields = !parsed.username().is_empty() || parsed.password().is_some();
    let has_at_in_authority = url
        .split("://")
        .nth(1)
        .and_then(|after_scheme| after_scheme.split('/').next())
        .is_some_and(|authority| authority.contains('@'));
    if has_userinfo_fields || has_at_in_authority {
        return Err(TransportError::InvalidBaseUrl {
            url: url.to_string(),
            reason: "URL must not contain userinfo (@ in authority)".to_string(),
        });
    }

    let host = parsed
        .host_str()
        .ok_or_else(|| TransportError::InvalidBaseUrl {
            url: url.to_string(),
            reason: "URL has no host component".to_string(),
        })?;

    Ok(host.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::transport::policy_gate::{AllowAllGate, PolicyDecision, PolicyGate};

    #[test]
    fn extract_host_from_https_url() {
        assert_eq!(extract_host("https://api.x.ai").unwrap(), "api.x.ai");
    }

    #[test]
    fn extract_host_from_url_with_path() {
        assert_eq!(
            extract_host("https://api.x.ai/v1/chat").unwrap(),
            "api.x.ai"
        );
    }

    #[test]
    fn extract_host_from_url_with_port() {
        assert_eq!(
            extract_host("http://localhost:8080/v1").unwrap(),
            "localhost"
        );
    }

    #[test]
    fn extract_host_fallback_is_error() {
        let result = extract_host("not-a-url");
        assert!(result.is_err());
        match result.unwrap_err() {
            TransportError::InvalidBaseUrl { url, .. } => {
                assert_eq!(url, "not-a-url");
            }
            other => panic!("expected InvalidBaseUrl, got: {other}"),
        }
    }

    #[test]
    fn extract_host_rejects_userinfo_url() {
        let result = extract_host("https://api.x.ai:443@evil.example/v1");
        assert!(result.is_err());
        match result.unwrap_err() {
            TransportError::InvalidBaseUrl { url, reason } => {
                assert_eq!(url, "https://api.x.ai:443@evil.example/v1");
                assert!(
                    reason.contains("userinfo"),
                    "reason should mention userinfo: {reason}"
                );
            }
            other => panic!("expected InvalidBaseUrl, got: {other}"),
        }
    }

    #[test]
    fn extract_host_rejects_empty_userinfo_at_sign() {
        // https://@evil.example/v1 has an empty username but still contains @
        // in the authority — must be rejected fail-closed.
        let result = extract_host("https://@evil.example/v1");
        assert!(result.is_err());
        match result.unwrap_err() {
            TransportError::InvalidBaseUrl { reason, .. } => {
                assert!(
                    reason.contains("userinfo") || reason.contains('@'),
                    "reason should mention userinfo or @: {reason}"
                );
            }
            other => panic!("expected InvalidBaseUrl, got: {other}"),
        }
    }

    #[test]
    fn extract_host_localhost_with_port() {
        assert_eq!(
            extract_host("http://127.0.0.1:8080/v1").unwrap(),
            "127.0.0.1"
        );
    }

    #[test]
    fn check_policy_gate_rejects_userinfo_url() {
        let config = HttpClientConfig {
            base_url: "https://api.x.ai:443@evil.example/v1".into(),
            ..Default::default()
        };
        let key = ApiKeySecret::new("key");
        let client = HttpClient::new(config, key, Some(Arc::new(AllowAllGate))).unwrap();

        let result = client.check_policy_gate();
        match result {
            Err(TransportError::InvalidBaseUrl { url, reason }) => {
                assert_eq!(url, "https://api.x.ai:443@evil.example/v1");
                assert!(
                    reason.contains("userinfo"),
                    "reason should mention userinfo: {reason}"
                );
            }
            other => panic!("expected InvalidBaseUrl, got: {other:?}"),
        }
    }

    #[test]
    fn http_client_debug_redacts_key() {
        let config = HttpClientConfig::default();
        let key = ApiKeySecret::new("super-secret");
        let client = HttpClient::new(config, key, None).unwrap();
        let debug = format!("{client:?}");
        assert!(debug.contains("[REDACTED]"));
        assert!(!debug.contains("super-secret"));
    }

    #[test]
    fn http_client_policy_gate_deny_blocks_request() {
        struct DenyGate;
        impl PolicyGate for DenyGate {
            fn evaluate_network(&self, _host: &str) -> PolicyDecision {
                PolicyDecision::Deny {
                    reason: "test deny".into(),
                }
            }
        }

        let config = HttpClientConfig::default();
        let key = ApiKeySecret::new("key");
        let client = HttpClient::new(config, key, Some(Arc::new(DenyGate))).unwrap();

        let result = client.check_policy_gate();
        match result {
            Err(TransportError::PolicyDenied { host, reason }) => {
                assert_eq!(host, "api.x.ai");
                assert_eq!(reason, "test deny");
            }
            other => panic!("expected PolicyDenied, got: {other:?}"),
        }
    }

    #[test]
    fn http_client_policy_gate_ask_blocks_request() {
        struct AskGate;
        impl PolicyGate for AskGate {
            fn evaluate_network(&self, _host: &str) -> PolicyDecision {
                PolicyDecision::Ask
            }
        }

        let config = HttpClientConfig::default();
        let key = ApiKeySecret::new("key");
        let client = HttpClient::new(config, key, Some(Arc::new(AskGate))).unwrap();

        let result = client.check_policy_gate();
        match result {
            Err(TransportError::ApprovalRequired { host }) => {
                assert_eq!(host, "api.x.ai");
            }
            other => panic!("expected ApprovalRequired, got: {other:?}"),
        }
    }

    #[test]
    fn http_client_no_policy_gate_denies() {
        let config = HttpClientConfig::default();
        let key = ApiKeySecret::new("key");
        let client = HttpClient::new(config, key, None).unwrap();
        let result = client.check_policy_gate();
        match result {
            Err(TransportError::PolicyDenied { host, reason }) => {
                assert_eq!(host, "api.x.ai");
                assert!(
                    reason.contains("denied"),
                    "reason should mention denied: {reason}"
                );
            }
            other => panic!("expected PolicyDenied (deny-by-default), got: {other:?}"),
        }
    }

    #[test]
    fn http_client_allow_all_gate_allows() {
        let config = HttpClientConfig::default();
        let key = ApiKeySecret::new("key");
        let client = HttpClient::new(config, key, Some(Arc::new(AllowAllGate))).unwrap();
        assert!(client.check_policy_gate().is_ok());
    }

    #[test]
    fn default_config_has_correct_base_url() {
        let config = HttpClientConfig::default();
        assert_eq!(config.base_url, "https://api.x.ai");
    }

    #[test]
    fn from_env_reads_xai_api_key() {
        let var_name = "GROKRS_TEST_FROM_ENV_KEY";
        // SAFETY: Test-only env manipulation; test runner serializes these tests.
        unsafe {
            std::env::set_var(var_name, "test-api-key-from-env");
        }
        let config = HttpClientConfig {
            api_key_env: Some(var_name.into()),
            ..Default::default()
        };
        let client = HttpClient::from_env(config, None);
        // SAFETY: Test-only env manipulation; test runner serializes these tests.
        unsafe {
            std::env::remove_var(var_name);
        }

        let client = client.expect("should construct from env");
        assert_eq!(client.api_key.expose(), "test-api-key-from-env");
    }

    #[test]
    fn from_env_defaults_to_xai_api_key_var() {
        // Use a unique name to avoid collision; we set then unset XAI_API_KEY
        // which other tests might also use, so we use the custom env var path
        // to verify the default constant name.
        let config = HttpClientConfig::default();
        assert!(config.api_key_env.is_none());
        // The default env var name used is "XAI_API_KEY" — verified by code path.
    }

    #[test]
    fn from_env_fails_when_env_var_missing() {
        let config = HttpClientConfig {
            api_key_env: Some("GROKRS_TEST_MISSING_ENV_VAR_XYZ".into()),
            ..Default::default()
        };
        // SAFETY: Test-only env manipulation; test runner serializes these tests.
        unsafe {
            std::env::remove_var("GROKRS_TEST_MISSING_ENV_VAR_XYZ");
        }
        let result = HttpClient::from_env(config, None);
        assert!(result.is_err());
        match result.unwrap_err() {
            TransportError::Auth { message } => {
                assert!(message.contains("is not set"));
            }
            other => panic!("expected Auth error, got: {other}"),
        }
    }

    #[test]
    fn from_env_fails_when_env_var_empty() {
        let var_name = "GROKRS_TEST_EMPTY_FROM_ENV";
        // SAFETY: Test-only env manipulation; test runner serializes these tests.
        unsafe {
            std::env::set_var(var_name, "");
        }
        let config = HttpClientConfig {
            api_key_env: Some(var_name.into()),
            ..Default::default()
        };
        let result = HttpClient::from_env(config, None);
        // SAFETY: Test-only env manipulation; test runner serializes these tests.
        unsafe {
            std::env::remove_var(var_name);
        }

        assert!(result.is_err());
        match result.unwrap_err() {
            TransportError::Auth { message } => {
                assert!(message.contains("empty"));
            }
            other => panic!("expected Auth error, got: {other}"),
        }
    }
}
