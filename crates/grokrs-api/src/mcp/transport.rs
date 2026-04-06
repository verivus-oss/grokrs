//! Streamable HTTP transport for the MCP protocol.
//!
//! Implements the Streamable HTTP transport from the MCP 2025-03-26 spec.
//! The transport sends JSON-RPC requests over HTTP POST and reads the
//! response body as a JSON-RPC response. SSE-based streaming is not used
//! for the initial implementation — each request-response is a single
//! HTTP round-trip, which is sufficient for `initialize`, `tools/list`,
//! and `tools/call`.

use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use reqwest::header::{HeaderValue, ACCEPT, CONTENT_TYPE};

use super::types::{JsonRpcRequest, JsonRpcResponse};

/// Errors that can occur during MCP transport operations.
#[derive(Debug)]
pub enum McpTransportError {
    /// The HTTP request failed at the network/TLS level.
    Http(reqwest::Error),
    /// The server returned a non-2xx status code.
    HttpStatus { status: u16, body: String },
    /// Failed to parse the response body as JSON.
    JsonParse {
        source: serde_json::Error,
        body: String,
    },
    /// The server URL is invalid.
    InvalidUrl(String),
    /// Connection timeout.
    Timeout,
}

impl std::fmt::Display for McpTransportError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            McpTransportError::Http(e) => write!(f, "MCP HTTP error: {e}"),
            McpTransportError::HttpStatus { status, body } => {
                write!(f, "MCP server returned HTTP {status}: {body}")
            }
            McpTransportError::JsonParse { source, body } => {
                write!(
                    f,
                    "MCP response JSON parse error: {source} (body: {})",
                    truncate_for_display(body, 200)
                )
            }
            McpTransportError::InvalidUrl(url) => write!(f, "invalid MCP server URL: {url}"),
            McpTransportError::Timeout => write!(f, "MCP server connection timed out"),
        }
    }
}

impl std::error::Error for McpTransportError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            McpTransportError::Http(e) => Some(e),
            McpTransportError::JsonParse { source, .. } => Some(source),
            _ => None,
        }
    }
}

/// UTF-8 safe truncation for display purposes.
fn truncate_for_display(s: &str, max_bytes: usize) -> &str {
    if s.len() <= max_bytes {
        return s;
    }
    let mut i = max_bytes;
    while i > 0 && !s.is_char_boundary(i) {
        i -= 1;
    }
    &s[..i]
}

/// Configuration for the MCP HTTP transport.
#[derive(Debug, Clone)]
pub struct McpTransportConfig {
    /// The MCP server's base URL (e.g., `http://localhost:8080`).
    pub server_url: String,
    /// HTTP request timeout. Default: 30 seconds.
    pub timeout: Duration,
    /// Optional MCP session ID for session affinity.
    pub session_id: Option<String>,
}

impl McpTransportConfig {
    /// Create a new transport config with the given server URL and default settings.
    pub fn new(server_url: impl Into<String>) -> Self {
        Self {
            server_url: server_url.into(),
            timeout: Duration::from_secs(30),
            session_id: None,
        }
    }

    /// Set the request timeout.
    pub fn with_timeout(mut self, timeout: Duration) -> Self {
        self.timeout = timeout;
        self
    }
}

/// Streamable HTTP transport for MCP protocol communication.
///
/// Sends JSON-RPC 2.0 requests as HTTP POST with `Content-Type: application/json`
/// and `Accept: application/json, text/event-stream` headers. Reads the response
/// body as a single JSON-RPC response.
///
/// Thread-safe: the internal request ID counter is atomic.
pub struct McpTransport {
    client: reqwest::Client,
    config: McpTransportConfig,
    /// Monotonically increasing request ID counter.
    next_id: AtomicU64,
    /// MCP session ID, set after successful `initialize`.
    session_id: std::sync::RwLock<Option<String>>,
}

impl McpTransport {
    /// Create a new MCP transport with the given configuration.
    pub fn new(config: McpTransportConfig) -> Result<Self, McpTransportError> {
        // Validate the URL.
        let _ = url::Url::parse(&config.server_url)
            .map_err(|_| McpTransportError::InvalidUrl(config.server_url.clone()))?;

        let client = reqwest::Client::builder()
            .timeout(config.timeout)
            .build()
            .map_err(McpTransportError::Http)?;

        Ok(Self {
            client,
            config,
            next_id: AtomicU64::new(1),
            session_id: std::sync::RwLock::new(None),
        })
    }

    /// Return the server URL.
    pub fn server_url(&self) -> &str {
        &self.config.server_url
    }

    /// Extract the host portion of the server URL.
    ///
    /// Used to generate `NetworkConnect` effects for the policy engine.
    pub fn server_host(&self) -> String {
        url::Url::parse(&self.config.server_url)
            .ok()
            .and_then(|u| u.host_str().map(|h| h.to_owned()))
            .unwrap_or_else(|| self.config.server_url.clone())
    }

    /// Allocate the next request ID.
    fn next_request_id(&self) -> u64 {
        self.next_id.fetch_add(1, Ordering::Relaxed)
    }

    /// Store the MCP session ID (received from `initialize` response headers).
    pub fn set_session_id(&self, id: String) {
        let mut guard = self.session_id.write().expect("session_id lock poisoned");
        *guard = Some(id);
    }

    /// Send a JSON-RPC request and receive the response.
    ///
    /// This is the core transport method. Higher-level methods like
    /// `send_initialize`, `send_tools_list`, etc. are built on top.
    pub async fn send(
        &self,
        method: &str,
        params: Option<serde_json::Value>,
    ) -> Result<JsonRpcResponse, McpTransportError> {
        let id = self.next_request_id();
        let request = JsonRpcRequest::new(id, method, params);

        let mut http_req = self
            .client
            .post(&self.config.server_url)
            .header(CONTENT_TYPE, HeaderValue::from_static("application/json"))
            .header(
                ACCEPT,
                HeaderValue::from_static("application/json, text/event-stream"),
            )
            .json(&request);

        // Include session ID if we have one.
        {
            let guard = self.session_id.read().expect("session_id lock poisoned");
            if let Some(ref sid) = *guard {
                if let Ok(val) = HeaderValue::from_str(sid) {
                    http_req = http_req.header("Mcp-Session-Id", val);
                }
            }
        }

        let response = http_req.send().await.map_err(|e| {
            if e.is_timeout() {
                McpTransportError::Timeout
            } else {
                McpTransportError::Http(e)
            }
        })?;

        // Capture session ID from response headers if present.
        if let Some(sid_header) = response.headers().get("mcp-session-id") {
            if let Ok(sid) = sid_header.to_str() {
                self.set_session_id(sid.to_owned());
            }
        }

        let status = response.status().as_u16();
        let body = response.text().await.map_err(McpTransportError::Http)?;

        if !(200..300).contains(&status) {
            return Err(McpTransportError::HttpStatus { status, body });
        }

        serde_json::from_str::<JsonRpcResponse>(&body)
            .map_err(|source| McpTransportError::JsonParse { source, body })
    }
}

impl std::fmt::Debug for McpTransport {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("McpTransport")
            .field("server_url", &self.config.server_url)
            .field("timeout", &self.config.timeout)
            .field("next_id", &self.next_id.load(Ordering::Relaxed))
            .finish()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn transport_config_new_sets_defaults() {
        let config = McpTransportConfig::new("http://localhost:8080");
        assert_eq!(config.server_url, "http://localhost:8080");
        assert_eq!(config.timeout, Duration::from_secs(30));
        assert!(config.session_id.is_none());
    }

    #[test]
    fn transport_config_with_timeout() {
        let config =
            McpTransportConfig::new("http://localhost:8080").with_timeout(Duration::from_secs(60));
        assert_eq!(config.timeout, Duration::from_secs(60));
    }

    #[test]
    fn transport_rejects_invalid_url() {
        let config = McpTransportConfig::new("not a url");
        let result = McpTransport::new(config);
        assert!(result.is_err());
        let err = result.unwrap_err();
        assert!(matches!(err, McpTransportError::InvalidUrl(_)));
    }

    #[test]
    fn transport_accepts_valid_url() {
        let config = McpTransportConfig::new("http://localhost:8080/mcp");
        let transport = McpTransport::new(config).unwrap();
        assert_eq!(transport.server_url(), "http://localhost:8080/mcp");
    }

    #[test]
    fn server_host_extracts_hostname() {
        let config = McpTransportConfig::new("http://localhost:8080/mcp");
        let transport = McpTransport::new(config).unwrap();
        assert_eq!(transport.server_host(), "localhost");
    }

    #[test]
    fn server_host_extracts_domain() {
        let config = McpTransportConfig::new("https://tools.example.com:9090/v1/mcp");
        let transport = McpTransport::new(config).unwrap();
        assert_eq!(transport.server_host(), "tools.example.com");
    }

    #[test]
    fn request_ids_increment() {
        let config = McpTransportConfig::new("http://localhost:8080");
        let transport = McpTransport::new(config).unwrap();
        let id1 = transport.next_request_id();
        let id2 = transport.next_request_id();
        let id3 = transport.next_request_id();
        assert_eq!(id1, 1);
        assert_eq!(id2, 2);
        assert_eq!(id3, 3);
    }

    #[test]
    fn session_id_management() {
        let config = McpTransportConfig::new("http://localhost:8080");
        let transport = McpTransport::new(config).unwrap();
        {
            let guard = transport.session_id.read().unwrap();
            assert!(guard.is_none());
        }
        transport.set_session_id("test-session-123".into());
        {
            let guard = transport.session_id.read().unwrap();
            assert_eq!(guard.as_deref(), Some("test-session-123"));
        }
    }

    #[test]
    fn transport_error_display() {
        let err = McpTransportError::HttpStatus {
            status: 404,
            body: "Not Found".into(),
        };
        assert!(format!("{err}").contains("404"));

        let err = McpTransportError::Timeout;
        assert!(format!("{err}").contains("timed out"));

        let err = McpTransportError::InvalidUrl("bad".into());
        assert!(format!("{err}").contains("bad"));
    }

    #[test]
    fn truncate_for_display_short_string() {
        assert_eq!(truncate_for_display("hello", 10), "hello");
    }

    #[test]
    fn truncate_for_display_long_string() {
        let long = "a".repeat(300);
        let truncated = truncate_for_display(&long, 200);
        assert_eq!(truncated.len(), 200);
    }
}
