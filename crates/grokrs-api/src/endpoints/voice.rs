//! Voice Agent API endpoint client.
//!
//! `VoiceAgentClient` manages the lifecycle of a WebSocket-based voice
//! conversation with the xAI voice agent. It handles:
//! - Authenticated connection establishment with policy gate evaluation
//! - Sending audio data, text input, function results, and control messages
//! - Receiving and dispatching server events (transcripts, function calls, etc.)
//! - Automatic heartbeat (ping/pong) on a configurable interval
//! - Reconnection with exponential backoff on transient failures
//! - Graceful shutdown
//!
//! The client is designed for a single voice session. Create a new client for
//! each voice conversation.

use std::sync::Arc;

use futures::StreamExt;
use tokio::sync::mpsc;

use crate::transport::auth::ApiKeySecret;
use crate::transport::error::TransportError;
use crate::transport::policy_gate::PolicyGate;
use crate::transport::websocket::{self, WsClientConfig, WsSink};
use crate::types::voice::{AudioData, ControlAction, VoiceConfig, VoiceEvent, VoiceMessage};

/// Received item from the voice agent: either a parsed event or raw audio data.
#[derive(Debug, Clone)]
pub enum VoiceReceived {
    /// A parsed JSON event from the server.
    Event(VoiceEvent),
    /// Raw audio data from a binary WebSocket frame.
    Audio(Vec<u8>),
}

/// Client for the xAI Voice Agent API.
///
/// Manages a single WebSocket-based voice session. The client provides
/// methods for sending messages and spawns background tasks for heartbeat
/// and message receiving.
///
/// # Usage
///
/// ```ignore
/// let client = VoiceAgentClient::new(config, api_key, Some(policy_gate));
/// let (event_rx, sink) = client.connect(voice_config).await?;
///
/// // Send audio in one task, receive events in another.
/// tokio::spawn(async move {
///     while let Some(received) = event_rx.recv().await {
///         match received {
///             Ok(VoiceReceived::Event(event)) => { /* handle event */ }
///             Ok(VoiceReceived::Audio(data)) => { /* play audio */ }
///             Err(e) => { /* handle error */ }
///         }
///     }
/// });
///
/// // Send audio data
/// client.send_audio(&sink, audio_bytes).await?;
/// ```
pub struct VoiceAgentClient {
    ws_config: WsClientConfig,
    api_key: ApiKeySecret,
    policy_gate: Option<Arc<dyn PolicyGate>>,
}

impl VoiceAgentClient {
    /// Create a new `VoiceAgentClient` with the given configuration.
    ///
    /// The client is not connected until `connect()` is called.
    #[must_use]
    pub fn new(
        ws_config: WsClientConfig,
        api_key: ApiKeySecret,
        policy_gate: Option<Arc<dyn PolicyGate>>,
    ) -> Self {
        Self {
            ws_config,
            api_key,
            policy_gate,
        }
    }

    /// Establish a WebSocket connection and start the voice session.
    ///
    /// This method:
    /// 1. Evaluates the policy gate for the target host
    /// 2. Establishes an authenticated WebSocket connection
    /// 3. Sends the initial `SessionConfig` message
    /// 4. Starts a background heartbeat task
    /// 5. Starts a background event-receiving task
    ///
    /// Returns a channel receiver for incoming events and a handle to the
    /// WebSocket sink for sending messages.
    ///
    /// # Errors
    ///
    /// Returns `TransportError::PolicyDenied` if the policy gate denies the
    /// connection. Returns `TransportError::WebSocket` on connection failure.
    pub async fn connect(
        &self,
        voice_config: VoiceConfig,
    ) -> Result<
        (
            mpsc::Receiver<Result<VoiceReceived, TransportError>>,
            Arc<tokio::sync::Mutex<WsSink>>,
        ),
        TransportError,
    > {
        // Connect via WebSocket.
        let conn = websocket::connect_ws(&self.ws_config, &self.api_key, self.policy_gate.as_ref())
            .await?;

        let sink = conn.sink;
        let mut source = conn.source;

        // Send initial session configuration.
        let config_msg = VoiceMessage::SessionConfig(voice_config);
        let config_json =
            serde_json::to_string(&config_msg).map_err(|e| TransportError::Serialization {
                message: format!("failed to serialize voice config: {e}"),
            })?;
        websocket::send_text(&sink, &config_json).await?;

        // Create event channel.
        let (tx, rx) = mpsc::channel::<Result<VoiceReceived, TransportError>>(256);

        // Start heartbeat task.
        let heartbeat_sink = Arc::clone(&sink);
        let heartbeat_interval = self.ws_config.heartbeat_interval_secs;
        tokio::spawn(async move {
            let mut interval =
                tokio::time::interval(std::time::Duration::from_secs(heartbeat_interval));
            // Skip the first tick (fires immediately).
            interval.tick().await;
            loop {
                interval.tick().await;
                if websocket::send_ping(&heartbeat_sink, b"keepalive".to_vec())
                    .await
                    .is_err()
                {
                    break;
                }
            }
        });

        // Start event receiver task.
        let event_tx = tx;
        tokio::spawn(async move {
            while let Some(msg_result) = source.next().await {
                match msg_result {
                    Ok(msg) => {
                        if let Some(frame) = websocket::parse_ws_message(msg) {
                            let received = match frame {
                                websocket::WsFrame::Text(text) => {
                                    match serde_json::from_str::<VoiceEvent>(&text) {
                                        Ok(event) => Ok(VoiceReceived::Event(event)),
                                        Err(e) => Err(TransportError::Deserialization {
                                            message: format!(
                                                "failed to parse voice event: {e} (text: {text})"
                                            ),
                                        }),
                                    }
                                }
                                websocket::WsFrame::Binary(data) => Ok(VoiceReceived::Audio(data)),
                                websocket::WsFrame::Close { code, reason } => {
                                    // Synthesize a close event.
                                    let reason_str = reason.unwrap_or_else(|| {
                                        format!("connection closed (code: {})", code.unwrap_or(0))
                                    });
                                    Ok(VoiceReceived::Event(VoiceEvent::StateChange {
                                        state: crate::types::voice::VoiceSessionState::Closed,
                                        reason: Some(reason_str),
                                    }))
                                }
                                websocket::WsFrame::Ping(_) | websocket::WsFrame::Pong(_) => {
                                    // Ping/pong handled automatically by tungstenite.
                                    // Pong from server = response to our heartbeat.
                                    continue;
                                }
                            };
                            if event_tx.send(received).await.is_err() {
                                // Receiver dropped -- stop processing.
                                break;
                            }
                        }
                    }
                    Err(e) => {
                        let _ = event_tx
                            .send(Err(TransportError::WebSocket {
                                message: format!("WebSocket receive error: {e}"),
                            }))
                            .await;
                        break;
                    }
                }
            }
        });

        Ok((rx, sink))
    }

    /// Send raw audio data to the voice agent.
    ///
    /// Audio should be in the format specified by `VoiceConfig::input_audio_format`
    /// (default: PCM 16-bit, 24kHz, mono). The data is sent as a WebSocket
    /// binary frame.
    pub async fn send_audio(
        &self,
        sink: &Arc<tokio::sync::Mutex<WsSink>>,
        audio_data: &AudioData,
    ) -> Result<(), TransportError> {
        websocket::send_binary(sink, audio_data.pcm_data.clone()).await
    }

    /// Send raw PCM audio bytes to the voice agent.
    ///
    /// Convenience method that takes raw bytes without the `AudioData` wrapper.
    pub async fn send_audio_bytes(
        &self,
        sink: &Arc<tokio::sync::Mutex<WsSink>>,
        pcm_data: Vec<u8>,
    ) -> Result<(), TransportError> {
        websocket::send_binary(sink, pcm_data).await
    }

    /// Send a text message to the voice agent (text-only mode).
    pub async fn send_text(
        &self,
        sink: &Arc<tokio::sync::Mutex<WsSink>>,
        text: &str,
    ) -> Result<(), TransportError> {
        let msg = VoiceMessage::TextInput {
            text: text.to_owned(),
        };
        let json = serde_json::to_string(&msg).map_err(|e| TransportError::Serialization {
            message: format!("failed to serialize text input: {e}"),
        })?;
        websocket::send_text(sink, &json).await
    }

    /// Send a function call result back to the voice agent.
    pub async fn send_function_result(
        &self,
        sink: &Arc<tokio::sync::Mutex<WsSink>>,
        call_id: &str,
        result: &str,
    ) -> Result<(), TransportError> {
        let msg = VoiceMessage::FunctionResult {
            call_id: call_id.to_owned(),
            result: result.to_owned(),
        };
        let json = serde_json::to_string(&msg).map_err(|e| TransportError::Serialization {
            message: format!("failed to serialize function result: {e}"),
        })?;
        websocket::send_text(sink, &json).await
    }

    /// Send a control message to the voice agent.
    pub async fn send_control(
        &self,
        sink: &Arc<tokio::sync::Mutex<WsSink>>,
        action: ControlAction,
    ) -> Result<(), TransportError> {
        let msg = VoiceMessage::Control { action };
        let json = serde_json::to_string(&msg).map_err(|e| TransportError::Serialization {
            message: format!("failed to serialize control message: {e}"),
        })?;
        websocket::send_text(sink, &json).await
    }

    /// Gracefully close the voice session.
    ///
    /// Sends a `Control::Close` message to the server, then closes the
    /// WebSocket connection.
    pub async fn close(
        &self,
        sink: &Arc<tokio::sync::Mutex<WsSink>>,
    ) -> Result<(), TransportError> {
        // Send control close first (best-effort).
        let _ = self.send_control(sink, ControlAction::Close).await;
        // Then close the WebSocket.
        websocket::send_close(sink).await
    }

    /// Return the WebSocket connection configuration.
    #[must_use]
    pub fn ws_config(&self) -> &WsClientConfig {
        &self.ws_config
    }
}

impl std::fmt::Debug for VoiceAgentClient {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("VoiceAgentClient")
            .field("ws_config", &self.ws_config)
            .field("api_key", &"[REDACTED]")
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::transport::auth::ApiKeySecret;
    use crate::transport::policy_gate::AllowAllGate;

    #[test]
    fn voice_agent_client_debug_redacts_key() {
        let config = WsClientConfig::default();
        let client =
            VoiceAgentClient::new(config, ApiKeySecret::new("super-secret-voice-key"), None);
        let debug = format!("{client:?}");
        assert!(debug.contains("[REDACTED]"));
        assert!(
            !debug.contains("super-secret-voice-key"),
            "debug output must not contain the raw API key"
        );
    }

    #[test]
    fn voice_agent_client_with_policy_gate() {
        let config = WsClientConfig::default();
        let gate: Arc<dyn PolicyGate> = Arc::new(AllowAllGate);
        let client = VoiceAgentClient::new(config, ApiKeySecret::new("key"), Some(gate));
        let debug = format!("{client:?}");
        assert!(debug.contains("Some(<PolicyGate>)"));
    }

    #[test]
    fn voice_agent_client_without_policy_gate() {
        let config = WsClientConfig::default();
        let client = VoiceAgentClient::new(config, ApiKeySecret::new("key"), None);
        let debug = format!("{client:?}");
        assert!(debug.contains("None"));
    }

    #[test]
    fn voice_received_event_variant() {
        let event = VoiceEvent::Pong;
        let received = VoiceReceived::Event(event);
        match received {
            VoiceReceived::Event(VoiceEvent::Pong) => {}
            other => panic!("expected Event(Pong), got: {other:?}"),
        }
    }

    #[test]
    fn voice_received_audio_variant() {
        let data = vec![1u8, 2, 3];
        let received = VoiceReceived::Audio(data.clone());
        match received {
            VoiceReceived::Audio(bytes) => assert_eq!(bytes, data),
            other => panic!("expected Audio, got: {other:?}"),
        }
    }

    #[test]
    fn ws_config_accessor() {
        let config = WsClientConfig {
            heartbeat_interval_secs: 15,
            ..WsClientConfig::default()
        };
        let client = VoiceAgentClient::new(config, ApiKeySecret::new("key"), None);
        assert_eq!(client.ws_config().heartbeat_interval_secs, 15);
    }
}
