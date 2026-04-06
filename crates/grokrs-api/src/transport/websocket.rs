//! WebSocket transport for the xAI Voice Agent API.
//!
//! This module provides a WebSocket client distinct from `HttpClient`. The
//! voice agent protocol uses persistent bidirectional WebSocket connections
//! with a fundamentally different lifecycle from HTTP request/response pairs.
//!
//! Key design decisions:
//! - Separate from `HttpClient` -- different error handling, different lifecycle
//! - Automatic ping/pong heartbeat on a configurable interval
//! - Reconnection with exponential backoff on transient failures
//! - Policy gate evaluation before WebSocket connection establishment
//! - Thread-safe: the `WsClient` can be shared via `Arc` across tasks

use std::sync::Arc;
use std::time::Duration;

use futures::stream::{SplitSink, SplitStream};
use futures::{SinkExt, StreamExt};
use tokio::net::TcpStream;
use tokio::sync::Mutex;
use tokio_tungstenite::tungstenite::client::IntoClientRequest;
use tokio_tungstenite::tungstenite::http::HeaderValue;
use tokio_tungstenite::tungstenite::protocol::Message;
use tokio_tungstenite::{MaybeTlsStream, WebSocketStream, connect_async};

use crate::transport::auth::ApiKeySecret;
use crate::transport::error::TransportError;
use crate::transport::policy_bridge::DenyAllGate;
use crate::transport::policy_gate::{PolicyDecision, PolicyGate};

/// Type alias for the WebSocket write half.
pub type WsSink = SplitSink<WebSocketStream<MaybeTlsStream<TcpStream>>, Message>;

/// Type alias for the WebSocket read half.
pub type WsSource = SplitStream<WebSocketStream<MaybeTlsStream<TcpStream>>>;

/// Configuration for the WebSocket client.
#[derive(Debug, Clone)]
pub struct WsClientConfig {
    /// Base URL for the API (e.g., "<wss://api.x.ai>").
    pub base_url: String,

    /// Path for the voice agent endpoint (e.g., "/v1/voice-agent").
    pub endpoint_path: String,

    /// Interval between heartbeat pings in seconds.
    pub heartbeat_interval_secs: u64,

    /// Maximum number of reconnection attempts on transient failure.
    pub max_reconnect_attempts: u32,

    /// Base delay for reconnection backoff in milliseconds.
    pub reconnect_base_delay_ms: u64,

    /// Maximum delay between reconnection attempts in milliseconds.
    pub reconnect_max_delay_ms: u64,

    /// Connection timeout in seconds.
    pub connect_timeout_secs: u64,
}

impl Default for WsClientConfig {
    fn default() -> Self {
        Self {
            base_url: "wss://api.x.ai".into(),
            endpoint_path: "/v1/voice-agent".into(),
            heartbeat_interval_secs: 30,
            max_reconnect_attempts: 5,
            reconnect_base_delay_ms: 500,
            reconnect_max_delay_ms: 30_000,
            connect_timeout_secs: 30,
        }
    }
}

/// State of a WebSocket connection.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WsConnectionState {
    /// Not connected.
    Disconnected,
    /// Attempting to connect.
    Connecting,
    /// Connected and ready.
    Connected,
    /// Connection failed after all retry attempts.
    Failed,
    /// Intentionally closed by the client.
    Closed,
}

/// A WebSocket connection split into send and receive halves.
///
/// The send half is wrapped in a `Mutex` so multiple tasks can send messages
/// concurrently (e.g., heartbeat task + audio streaming task). The receive
/// half is typically owned by a single reader task.
pub struct WsConnection {
    /// The write half, protected by a mutex for concurrent senders.
    pub sink: Arc<Mutex<WsSink>>,
    /// The read half, owned by the event-processing task.
    pub source: WsSource,
    /// Current connection state.
    pub state: WsConnectionState,
}

/// Result of a WebSocket frame parse operation.
#[derive(Debug, Clone, PartialEq)]
pub enum WsFrame {
    /// A JSON text message from the server.
    Text(String),
    /// A binary audio data frame from the server.
    Binary(Vec<u8>),
    /// A ping frame (responded to automatically, but exposed for logging).
    Ping(Vec<u8>),
    /// A pong response to our ping.
    Pong(Vec<u8>),
    /// The server initiated a close handshake.
    Close {
        /// Close code, if provided.
        code: Option<u16>,
        /// Close reason, if provided.
        reason: Option<String>,
    },
}

/// Parse a raw WebSocket `Message` into a typed `WsFrame`.
///
/// This separates the tungstenite message type from our domain types,
/// making it easier to test frame handling without a live WebSocket.
pub fn parse_ws_message(msg: Message) -> Option<WsFrame> {
    match msg {
        Message::Text(text) => Some(WsFrame::Text(text.to_string())),
        Message::Binary(data) => Some(WsFrame::Binary(data.to_vec())),
        Message::Ping(data) => Some(WsFrame::Ping(data.to_vec())),
        Message::Pong(data) => Some(WsFrame::Pong(data.to_vec())),
        Message::Close(frame) => Some(WsFrame::Close {
            code: frame.as_ref().map(|f| f.code.into()),
            reason: frame.map(|f| f.reason.to_string()),
        }),
        // tungstenite Frame variant -- should not appear in normal message stream
        Message::Frame(_) => None,
    }
}

/// Compute the reconnection delay for a given attempt using exponential backoff.
///
/// Uses the same deterministic jitter approach as the HTTP retry module.
#[must_use]
pub fn reconnect_delay(attempt: u32, config: &WsClientConfig) -> Duration {
    let exp_delay_ms = config
        .reconnect_base_delay_ms
        .saturating_mul(1u64 << attempt.min(20));

    // Deterministic jitter from system time nanoseconds.
    let jitter_seed = u64::from(
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or(Duration::ZERO)
            .subsec_nanos(),
    );

    let base_jitter_ms = u64::from(attempt).saturating_add(1).saturating_mul(100);
    let scale_permille = 500 + (jitter_seed % 1000);
    let jitter_ms = base_jitter_ms.saturating_mul(scale_permille) / 1000;

    let total_ms = exp_delay_ms.saturating_add(jitter_ms);
    let clamped_ms = total_ms.min(config.reconnect_max_delay_ms);

    Duration::from_millis(clamped_ms)
}

/// Extract the host portion from a WebSocket URL.
///
/// Validates the URL and rejects userinfo in the authority section.
fn extract_ws_host(url: &str) -> Result<String, TransportError> {
    let parsed = url::Url::parse(url).map_err(|e| TransportError::InvalidBaseUrl {
        url: url.to_string(),
        reason: e.to_string(),
    })?;

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

/// Check the policy gate before establishing a WebSocket connection.
///
/// Extracts the host from the WebSocket URL and evaluates it against the
/// policy gate. If no gate is provided, `DenyAllGate` is used (fail-closed).
///
/// # Errors
///
/// Returns [`TransportError::InvalidBaseUrl`] if the host cannot be extracted from the URL.
/// Returns [`TransportError::PolicyDenied`] if the policy gate denies the connection.
/// Returns [`TransportError::ApprovalRequired`] if the policy gate returns `Ask`.
pub fn check_ws_policy_gate(
    url: &str,
    policy_gate: Option<&Arc<dyn PolicyGate>>,
) -> Result<(), TransportError> {
    let deny_all = DenyAllGate;
    let gate: &dyn PolicyGate = match policy_gate {
        Some(g) => g.as_ref(),
        None => &deny_all,
    };
    let host = extract_ws_host(url)?;
    match gate.evaluate_network(&host) {
        PolicyDecision::Allow => Ok(()),
        PolicyDecision::Deny { reason } => Err(TransportError::PolicyDenied { host, reason }),
        PolicyDecision::Ask => Err(TransportError::ApprovalRequired { host }),
    }
}

/// Establish an authenticated WebSocket connection to the voice agent endpoint.
///
/// This function:
/// 1. Evaluates the policy gate for the target host
/// 2. Constructs the authenticated WebSocket URL
/// 3. Connects with a timeout
/// 4. Splits the connection into send/receive halves
///
/// The caller is responsible for sending the initial `SessionConfig` message
/// after the connection is established.
///
/// # Errors
///
/// Returns [`TransportError::PolicyDenied`] if the policy gate denies the connection.
/// Returns [`TransportError::WebSocket`] if the WebSocket handshake fails.
/// Returns [`TransportError::Auth`] if the API key contains invalid header characters.
/// Returns [`TransportError::Timeout`] if the connection exceeds the configured timeout.
pub async fn connect_ws(
    config: &WsClientConfig,
    api_key: &ApiKeySecret,
    policy_gate: Option<&Arc<dyn PolicyGate>>,
) -> Result<WsConnection, TransportError> {
    let url = format!("{}{}", config.base_url, config.endpoint_path);

    // Policy gate check.
    check_ws_policy_gate(&url, policy_gate)?;

    // Build the WebSocket request with auth header.
    let mut request = url
        .into_client_request()
        .map_err(|e| TransportError::WebSocket {
            message: format!("failed to build WebSocket request: {e}"),
        })?;

    request.headers_mut().insert(
        "Authorization",
        HeaderValue::from_str(&format!("Bearer {}", api_key.expose())).map_err(|_| {
            TransportError::Auth {
                message: "API key contains invalid header characters".to_owned(),
            }
        })?,
    );

    // Connect with timeout.
    let connect_future = connect_async(request);
    let timeout = Duration::from_secs(config.connect_timeout_secs);

    let (ws_stream, _response) = tokio::time::timeout(timeout, connect_future)
        .await
        .map_err(|_| TransportError::Timeout)?
        .map_err(|e| TransportError::WebSocket {
            message: format!("WebSocket connection failed: {e}"),
        })?;

    let (sink, source) = ws_stream.split();

    Ok(WsConnection {
        sink: Arc::new(Mutex::new(sink)),
        source,
        state: WsConnectionState::Connected,
    })
}

/// Send a text message through the WebSocket sink.
///
/// # Errors
///
/// Returns [`TransportError::WebSocket`] if sending the message fails.
pub async fn send_text(sink: &Arc<Mutex<WsSink>>, text: &str) -> Result<(), TransportError> {
    let mut guard = sink.lock().await;
    guard
        .send(Message::Text(text.into()))
        .await
        .map_err(|e| TransportError::WebSocket {
            message: format!("failed to send text message: {e}"),
        })
}

/// Send a binary message (audio data) through the WebSocket sink.
///
/// # Errors
///
/// Returns [`TransportError::WebSocket`] if sending the message fails.
pub async fn send_binary(sink: &Arc<Mutex<WsSink>>, data: Vec<u8>) -> Result<(), TransportError> {
    let mut guard = sink.lock().await;
    guard
        .send(Message::Binary(data.into()))
        .await
        .map_err(|e| TransportError::WebSocket {
            message: format!("failed to send binary message: {e}"),
        })
}

/// Send a ping frame through the WebSocket sink.
///
/// # Errors
///
/// Returns [`TransportError::WebSocket`] if sending the ping fails.
pub async fn send_ping(sink: &Arc<Mutex<WsSink>>, payload: Vec<u8>) -> Result<(), TransportError> {
    let mut guard = sink.lock().await;
    guard
        .send(Message::Ping(payload.into()))
        .await
        .map_err(|e| TransportError::WebSocket {
            message: format!("failed to send ping: {e}"),
        })
}

/// Send a close frame through the WebSocket sink.
///
/// # Errors
///
/// Returns [`TransportError::WebSocket`] if sending the close frame fails.
pub async fn send_close(sink: &Arc<Mutex<WsSink>>) -> Result<(), TransportError> {
    let mut guard = sink.lock().await;
    guard
        .send(Message::Close(None))
        .await
        .map_err(|e| TransportError::WebSocket {
            message: format!("failed to send close: {e}"),
        })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::transport::policy_gate::AllowAllGate;

    // --- WsFrame parsing ---

    #[test]
    fn parse_text_message() {
        let msg = Message::Text(r#"{"type":"pong"}"#.into());
        let frame = parse_ws_message(msg).unwrap();
        match frame {
            WsFrame::Text(text) => assert!(text.contains("pong")),
            other => panic!("expected Text, got: {other:?}"),
        }
    }

    #[test]
    fn parse_binary_message() {
        let data = vec![0u8, 1, 2, 3, 4, 5];
        let msg = Message::Binary(data.clone().into());
        let frame = parse_ws_message(msg).unwrap();
        match frame {
            WsFrame::Binary(bytes) => assert_eq!(bytes, data),
            other => panic!("expected Binary, got: {other:?}"),
        }
    }

    #[test]
    fn parse_ping_message() {
        let payload = b"ping_data".to_vec();
        let msg = Message::Ping(payload.clone().into());
        let frame = parse_ws_message(msg).unwrap();
        match frame {
            WsFrame::Ping(data) => assert_eq!(data, payload),
            other => panic!("expected Ping, got: {other:?}"),
        }
    }

    #[test]
    fn parse_pong_message() {
        let payload = b"pong_data".to_vec();
        let msg = Message::Pong(payload.clone().into());
        let frame = parse_ws_message(msg).unwrap();
        match frame {
            WsFrame::Pong(data) => assert_eq!(data, payload),
            other => panic!("expected Pong, got: {other:?}"),
        }
    }

    #[test]
    fn parse_close_message_no_frame() {
        let msg = Message::Close(None);
        let frame = parse_ws_message(msg).unwrap();
        match frame {
            WsFrame::Close { code, reason } => {
                assert!(code.is_none());
                assert!(reason.is_none());
            }
            other => panic!("expected Close, got: {other:?}"),
        }
    }

    #[test]
    fn parse_close_message_with_frame() {
        use tokio_tungstenite::tungstenite::protocol::frame::CloseFrame;
        let close_frame = CloseFrame {
            code: tokio_tungstenite::tungstenite::protocol::frame::coding::CloseCode::Normal,
            reason: "session ended".into(),
        };
        let msg = Message::Close(Some(close_frame));
        let frame = parse_ws_message(msg).unwrap();
        match frame {
            WsFrame::Close { code, reason } => {
                assert_eq!(code, Some(1000));
                assert_eq!(reason.as_deref(), Some("session ended"));
            }
            other => panic!("expected Close, got: {other:?}"),
        }
    }

    // --- Reconnect delay ---

    #[test]
    fn reconnect_delay_increases_with_attempts() {
        let config = WsClientConfig::default();
        let delay0 = reconnect_delay(0, &config);
        let delay2 = reconnect_delay(2, &config);
        // With exponential backoff, later delays are larger.
        assert!(
            delay2 > delay0,
            "delay at attempt 2 ({delay2:?}) should exceed delay at attempt 0 ({delay0:?})"
        );
    }

    #[test]
    fn reconnect_delay_respects_max() {
        let config = WsClientConfig {
            reconnect_base_delay_ms: 1000,
            reconnect_max_delay_ms: 5000,
            ..WsClientConfig::default()
        };
        let delay = reconnect_delay(20, &config);
        assert!(
            delay.as_millis() <= 5000,
            "delay should be clamped to max, got {delay:?}",
        );
    }

    // --- Policy gate ---

    #[test]
    fn ws_policy_gate_allows_with_allow_gate() {
        let gate: Arc<dyn PolicyGate> = Arc::new(AllowAllGate);
        let result = check_ws_policy_gate("wss://api.x.ai/v1/voice-agent", Some(&gate));
        assert!(result.is_ok());
    }

    #[test]
    fn ws_policy_gate_denies_without_gate() {
        let result = check_ws_policy_gate("wss://api.x.ai/v1/voice-agent", None);
        assert!(result.is_err());
        match result.unwrap_err() {
            TransportError::PolicyDenied { host, .. } => {
                assert_eq!(host, "api.x.ai");
            }
            other => panic!("expected PolicyDenied, got: {other}"),
        }
    }

    #[test]
    fn ws_policy_gate_rejects_userinfo_url() {
        let gate: Arc<dyn PolicyGate> = Arc::new(AllowAllGate);
        let result = check_ws_policy_gate("wss://user:pass@api.x.ai/v1/voice-agent", Some(&gate));
        assert!(result.is_err());
        match result.unwrap_err() {
            TransportError::InvalidBaseUrl { reason, .. } => {
                assert!(reason.contains("userinfo"));
            }
            other => panic!("expected InvalidBaseUrl, got: {other}"),
        }
    }

    #[test]
    fn ws_policy_gate_rejects_invalid_url() {
        let gate: Arc<dyn PolicyGate> = Arc::new(AllowAllGate);
        let result = check_ws_policy_gate("not-a-url", Some(&gate));
        assert!(result.is_err());
    }

    // --- Host extraction ---

    #[test]
    fn extract_ws_host_from_wss_url() {
        assert_eq!(
            extract_ws_host("wss://api.x.ai/v1/voice-agent").unwrap(),
            "api.x.ai"
        );
    }

    #[test]
    fn extract_ws_host_from_ws_url() {
        assert_eq!(
            extract_ws_host("ws://localhost:8080/voice").unwrap(),
            "localhost"
        );
    }

    // --- WsClientConfig defaults ---

    #[test]
    fn ws_client_config_defaults() {
        let config = WsClientConfig::default();
        assert_eq!(config.base_url, "wss://api.x.ai");
        assert_eq!(config.endpoint_path, "/v1/voice-agent");
        assert_eq!(config.heartbeat_interval_secs, 30);
        assert_eq!(config.max_reconnect_attempts, 5);
        assert_eq!(config.connect_timeout_secs, 30);
    }

    // --- WsConnectionState ---

    #[test]
    fn ws_connection_state_equality() {
        assert_eq!(
            WsConnectionState::Disconnected,
            WsConnectionState::Disconnected
        );
        assert_ne!(WsConnectionState::Connected, WsConnectionState::Closed);
    }
}
