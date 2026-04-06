//! Wire types for the xAI Voice Agent API.
//!
//! These types model the WebSocket-based voice conversation protocol. The voice
//! agent uses a bidirectional WebSocket connection where:
//! - Client sends: audio data, text input, function results, control messages
//! - Server sends: audio chunks, transcripts, function calls, state changes, errors
//!
//! Audio is transmitted as raw PCM (16-bit, 24kHz, mono) in both directions.
//! The server handles Voice Activity Detection (VAD) -- the client just streams
//! raw audio without needing to detect speech boundaries.

use serde::{Deserialize, Serialize};

// ---------------------------------------------------------------------------
// Voice configuration
// ---------------------------------------------------------------------------

/// Voice selection for the voice agent session.
///
/// Maps to the same voice identifiers used by the TTS API.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum VoiceId {
    /// Eve voice.
    Eve,
    /// Ara voice.
    Ara,
    /// Rex voice.
    Rex,
    /// Sal voice.
    Sal,
    /// Leo voice.
    Leo,
}

impl std::fmt::Display for VoiceId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Eve => write!(f, "eve"),
            Self::Ara => write!(f, "ara"),
            Self::Rex => write!(f, "rex"),
            Self::Sal => write!(f, "sal"),
            Self::Leo => write!(f, "leo"),
        }
    }
}

impl std::str::FromStr for VoiceId {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_lowercase().as_str() {
            "eve" => Ok(Self::Eve),
            "ara" => Ok(Self::Ara),
            "rex" => Ok(Self::Rex),
            "sal" => Ok(Self::Sal),
            "leo" => Ok(Self::Leo),
            other => Err(format!(
                "unknown voice ID '{other}'; expected one of: eve, ara, rex, sal, leo"
            )),
        }
    }
}

/// Turn detection mode for the voice agent.
///
/// Controls how the server determines when the user has finished speaking.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TurnDetectionMode {
    /// Server-side Voice Activity Detection determines turn boundaries.
    #[default]
    ServerVad,
    /// Client explicitly signals turn boundaries via control messages.
    Manual,
}

/// VAD (Voice Activity Detection) sensitivity level.
///
/// Controls how aggressively the server detects speech vs silence.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum VadSensitivity {
    /// Low sensitivity: requires louder/clearer speech to trigger.
    Low,
    /// Medium sensitivity: balanced detection (default).
    #[default]
    Medium,
    /// High sensitivity: triggers on quieter speech, may pick up background noise.
    High,
}

/// Audio format specification for the voice session.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct AudioFormat {
    /// Audio encoding format.
    pub encoding: AudioEncoding,
    /// Sample rate in Hz (e.g., 24000).
    pub sample_rate: u32,
    /// Number of audio channels (1 = mono, 2 = stereo).
    pub channels: u8,
}

impl Default for AudioFormat {
    fn default() -> Self {
        Self {
            encoding: AudioEncoding::Pcm16,
            sample_rate: 24000,
            channels: 1,
        }
    }
}

/// Audio encoding format.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AudioEncoding {
    /// 16-bit PCM (little-endian).
    Pcm16,
    /// G.711 mu-law.
    Mulaw,
    /// G.711 A-law.
    Alaw,
}

/// Configuration for a voice agent session.
///
/// Sent as the initial configuration when establishing a WebSocket connection.
/// All fields have sensible defaults for a standard voice conversation.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VoiceConfig {
    /// The model to use for the voice agent (e.g., "grok-4").
    pub model: String,

    /// Voice to use for agent speech output.
    #[serde(default = "default_voice")]
    pub voice: VoiceId,

    /// Language code for the conversation (BCP 47, e.g., "en-US").
    #[serde(default = "default_language")]
    pub language: String,

    /// How turn boundaries are detected.
    #[serde(default)]
    pub turn_detection: TurnDetectionMode,

    /// VAD sensitivity when using `ServerVad` turn detection.
    #[serde(default)]
    pub vad_sensitivity: VadSensitivity,

    /// Audio format for input (client-to-server) audio.
    #[serde(default)]
    pub input_audio_format: AudioFormat,

    /// Audio format for output (server-to-client) audio.
    #[serde(default)]
    pub output_audio_format: AudioFormat,

    /// Optional system instructions for the voice agent.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub system_instructions: Option<String>,

    /// Optional list of tool definitions the agent can invoke.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tools: Option<Vec<ToolDefinition>>,

    /// Silence timeout in milliseconds. If the server detects silence for this
    /// duration, it may end the turn or send a prompt.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub silence_timeout_ms: Option<u32>,

    /// Maximum duration for the voice session in seconds.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_duration_secs: Option<u32>,
}

fn default_voice() -> VoiceId {
    VoiceId::Eve
}

fn default_language() -> String {
    "en-US".to_owned()
}

impl Default for VoiceConfig {
    fn default() -> Self {
        Self {
            model: "grok-4".to_owned(),
            voice: default_voice(),
            language: default_language(),
            turn_detection: TurnDetectionMode::default(),
            vad_sensitivity: VadSensitivity::default(),
            input_audio_format: AudioFormat::default(),
            output_audio_format: AudioFormat::default(),
            system_instructions: None,
            tools: None,
            silence_timeout_ms: None,
            max_duration_secs: None,
        }
    }
}

/// A tool definition that the voice agent can invoke during conversation.
///
/// This mirrors the function calling tool definitions used in the Responses API,
/// adapted for the voice agent protocol.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolDefinition {
    /// The type of tool (always "function" for now).
    #[serde(rename = "type")]
    pub tool_type: String,

    /// The function definition.
    pub function: FunctionDefinition,
}

/// A function definition within a tool definition.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FunctionDefinition {
    /// The name of the function.
    pub name: String,

    /// A description of what the function does.
    pub description: String,

    /// JSON Schema for the function parameters.
    pub parameters: serde_json::Value,
}

// ---------------------------------------------------------------------------
// Client-to-server messages (VoiceMessage)
// ---------------------------------------------------------------------------

/// Messages sent from the client to the voice agent server.
///
/// These are serialized to JSON and sent as WebSocket text frames, except for
/// `AudioData` which is sent as a binary frame containing raw PCM audio.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum VoiceMessage {
    /// Session configuration sent at connection start.
    SessionConfig(VoiceConfig),

    /// Text input instead of audio (for text-only mode).
    TextInput {
        /// The text message to send to the agent.
        text: String,
    },

    /// Result of a function call requested by the server.
    FunctionResult {
        /// The call ID that this result corresponds to.
        call_id: String,
        /// The function result as a JSON string.
        result: String,
    },

    /// Control message to signal turn boundaries or session lifecycle.
    Control {
        /// The control action to perform.
        action: ControlAction,
    },
}

/// Control actions for voice session management.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ControlAction {
    /// Signal the end of the current user turn (manual turn detection).
    EndTurn,
    /// Request the agent to stop generating audio/text.
    Interrupt,
    /// Gracefully close the session.
    Close,
    /// Ping to keep the connection alive.
    Ping,
}

// ---------------------------------------------------------------------------
// Server-to-client events (VoiceEvent)
// ---------------------------------------------------------------------------

/// Events received from the voice agent server.
///
/// Most events arrive as JSON text frames. `AudioChunk` payloads arrive as
/// binary frames containing raw PCM audio data.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum VoiceEvent {
    /// Session has been established and configured.
    SessionCreated {
        /// The server-assigned session ID.
        session_id: String,
        /// The configuration acknowledged by the server.
        #[serde(skip_serializing_if = "Option::is_none")]
        config: Option<VoiceConfig>,
    },

    /// An audio chunk from the agent's speech output.
    ///
    /// The actual audio bytes are delivered as a WebSocket binary frame,
    /// not inline in the JSON. This event carries metadata only.
    AudioChunk {
        /// Sequence number for ordering audio chunks.
        sequence: u64,
        /// Duration of this chunk in milliseconds.
        #[serde(skip_serializing_if = "Option::is_none")]
        duration_ms: Option<u32>,
    },

    /// A transcript of speech (user or agent).
    Transcript {
        /// Who produced this transcript.
        role: TranscriptRole,
        /// The transcribed text.
        text: String,
        /// Whether this is a final (committed) or interim (in-progress) transcript.
        #[serde(default)]
        is_final: bool,
    },

    /// The server is requesting the client to execute a function.
    FunctionCall {
        /// Unique ID for this function call (used to match the result).
        call_id: String,
        /// Name of the function to invoke.
        name: String,
        /// JSON-encoded arguments for the function.
        arguments: String,
    },

    /// A change in the voice session state.
    StateChange {
        /// The new state of the session.
        state: VoiceSessionState,
        /// Optional human-readable reason for the state change.
        #[serde(skip_serializing_if = "Option::is_none")]
        reason: Option<String>,
    },

    /// An error occurred during the voice session.
    Error {
        /// A machine-readable error code.
        code: String,
        /// A human-readable error message.
        message: String,
        /// Whether the error is fatal (session will close).
        #[serde(default)]
        fatal: bool,
    },

    /// Server pong response to a client ping.
    Pong,

    /// Session usage statistics (sent periodically or at session end).
    Usage {
        /// Total input tokens consumed.
        #[serde(default)]
        input_tokens: u64,
        /// Total output tokens generated.
        #[serde(default)]
        output_tokens: u64,
        /// Total audio duration in seconds (input).
        #[serde(default)]
        input_audio_secs: f64,
        /// Total audio duration in seconds (output).
        #[serde(default)]
        output_audio_secs: f64,
    },
}

/// Role for transcript events.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TranscriptRole {
    /// The user's speech.
    User,
    /// The agent's speech.
    Agent,
}

impl std::fmt::Display for TranscriptRole {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::User => write!(f, "user"),
            Self::Agent => write!(f, "agent"),
        }
    }
}

/// Voice session states as reported by the server.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum VoiceSessionState {
    /// Session is initializing.
    Initializing,
    /// Session is ready, waiting for user input.
    Listening,
    /// Server is processing the user's input.
    Thinking,
    /// Agent is generating speech output.
    Speaking,
    /// Waiting for a function call result from the client.
    WaitingForTool,
    /// Session is closing.
    Closing,
    /// Session has closed.
    Closed,
    /// Session encountered a fatal error.
    Error,
}

impl std::fmt::Display for VoiceSessionState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Initializing => write!(f, "initializing"),
            Self::Listening => write!(f, "listening"),
            Self::Thinking => write!(f, "thinking"),
            Self::Speaking => write!(f, "speaking"),
            Self::WaitingForTool => write!(f, "waiting_for_tool"),
            Self::Closing => write!(f, "closing"),
            Self::Closed => write!(f, "closed"),
            Self::Error => write!(f, "error"),
        }
    }
}

// ---------------------------------------------------------------------------
// Audio data wrapper
// ---------------------------------------------------------------------------

/// Raw audio data for binary WebSocket frames.
///
/// This is not serialized via serde -- it is sent/received as raw binary
/// WebSocket frames. The struct exists for type safety in the transport layer.
#[derive(Debug, Clone)]
pub struct AudioData {
    /// Raw PCM audio bytes.
    pub pcm_data: Vec<u8>,
    /// Sequence number for ordering (matches `AudioChunk::sequence`).
    pub sequence: u64,
}

// ---------------------------------------------------------------------------
// Session summary
// ---------------------------------------------------------------------------

/// Summary of a voice session, returned on session close.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VoiceSessionSummary {
    /// The server-assigned session ID.
    pub session_id: String,
    /// Total duration of the session in seconds.
    pub duration_secs: f64,
    /// Number of conversation turns.
    pub turn_count: u32,
    /// Total input tokens consumed.
    pub input_tokens: u64,
    /// Total output tokens generated.
    pub output_tokens: u64,
    /// Total input audio duration in seconds.
    pub input_audio_secs: f64,
    /// Total output audio duration in seconds.
    pub output_audio_secs: f64,
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json;

    // --- VoiceConfig serialization ---

    #[test]
    fn voice_config_default_serializes() {
        let config = VoiceConfig::default();
        let json = serde_json::to_string(&config).expect("should serialize");
        assert!(json.contains("\"model\":\"grok-4\""));
        assert!(json.contains("\"voice\":\"eve\""));
        assert!(json.contains("\"language\":\"en-US\""));
        assert!(json.contains("\"turn_detection\":\"server_vad\""));
        assert!(json.contains("\"vad_sensitivity\":\"medium\""));
    }

    #[test]
    fn voice_config_roundtrips() {
        let config = VoiceConfig {
            model: "grok-4-mini".to_owned(),
            voice: VoiceId::Rex,
            language: "de-DE".to_owned(),
            turn_detection: TurnDetectionMode::Manual,
            vad_sensitivity: VadSensitivity::High,
            input_audio_format: AudioFormat::default(),
            output_audio_format: AudioFormat::default(),
            system_instructions: Some("You are a helpful assistant.".to_owned()),
            tools: None,
            silence_timeout_ms: Some(3000),
            max_duration_secs: Some(300),
        };
        let json = serde_json::to_string(&config).unwrap();
        let parsed: VoiceConfig = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed.model, "grok-4-mini");
        assert_eq!(parsed.voice, VoiceId::Rex);
        assert_eq!(parsed.language, "de-DE");
        assert_eq!(parsed.turn_detection, TurnDetectionMode::Manual);
        assert_eq!(parsed.vad_sensitivity, VadSensitivity::High);
        assert_eq!(
            parsed.system_instructions.as_deref(),
            Some("You are a helpful assistant.")
        );
        assert_eq!(parsed.silence_timeout_ms, Some(3000));
        assert_eq!(parsed.max_duration_secs, Some(300));
    }

    #[test]
    fn voice_config_omits_none_fields() {
        let config = VoiceConfig::default();
        let json = serde_json::to_string(&config).unwrap();
        assert!(
            !json.contains("system_instructions"),
            "None fields should be omitted"
        );
        assert!(!json.contains("tools"), "None fields should be omitted");
        assert!(
            !json.contains("silence_timeout_ms"),
            "None fields should be omitted"
        );
    }

    #[test]
    fn voice_config_with_tools() {
        let config = VoiceConfig {
            tools: Some(vec![ToolDefinition {
                tool_type: "function".to_owned(),
                function: FunctionDefinition {
                    name: "get_weather".to_owned(),
                    description: "Get current weather".to_owned(),
                    parameters: serde_json::json!({
                        "type": "object",
                        "properties": {
                            "city": {"type": "string"}
                        },
                        "required": ["city"]
                    }),
                },
            }]),
            ..VoiceConfig::default()
        };
        let json = serde_json::to_string(&config).unwrap();
        assert!(json.contains("get_weather"));
        let parsed: VoiceConfig = serde_json::from_str(&json).unwrap();
        let tools = parsed.tools.unwrap();
        assert_eq!(tools.len(), 1);
        assert_eq!(tools[0].function.name, "get_weather");
    }

    // --- VoiceMessage serialization ---

    #[test]
    fn voice_message_text_input_serializes() {
        let msg = VoiceMessage::TextInput {
            text: "Hello agent".to_owned(),
        };
        let json = serde_json::to_string(&msg).unwrap();
        assert!(json.contains("\"type\":\"text_input\""));
        assert!(json.contains("\"text\":\"Hello agent\""));
    }

    #[test]
    fn voice_message_function_result_serializes() {
        let msg = VoiceMessage::FunctionResult {
            call_id: "call_abc".to_owned(),
            result: "{\"temp\":72}".to_owned(),
        };
        let json = serde_json::to_string(&msg).unwrap();
        assert!(json.contains("\"type\":\"function_result\""));
        assert!(json.contains("\"call_id\":\"call_abc\""));
    }

    #[test]
    fn voice_message_control_serializes() {
        let msg = VoiceMessage::Control {
            action: ControlAction::EndTurn,
        };
        let json = serde_json::to_string(&msg).unwrap();
        assert!(json.contains("\"type\":\"control\""));
        assert!(json.contains("\"action\":\"end_turn\""));
    }

    #[test]
    fn voice_message_control_close_serializes() {
        let msg = VoiceMessage::Control {
            action: ControlAction::Close,
        };
        let json = serde_json::to_string(&msg).unwrap();
        assert!(json.contains("\"action\":\"close\""));
    }

    #[test]
    fn voice_message_control_ping_serializes() {
        let msg = VoiceMessage::Control {
            action: ControlAction::Ping,
        };
        let json = serde_json::to_string(&msg).unwrap();
        assert!(json.contains("\"action\":\"ping\""));
    }

    // --- VoiceEvent deserialization ---

    #[test]
    fn voice_event_session_created_deserializes() {
        let json = r#"{"type":"session_created","session_id":"sess_123"}"#;
        let event: VoiceEvent = serde_json::from_str(json).unwrap();
        match event {
            VoiceEvent::SessionCreated { session_id, config } => {
                assert_eq!(session_id, "sess_123");
                assert!(config.is_none());
            }
            other => panic!("expected SessionCreated, got: {other:?}"),
        }
    }

    #[test]
    fn voice_event_transcript_deserializes() {
        let json = r#"{"type":"transcript","role":"user","text":"Hello","is_final":true}"#;
        let event: VoiceEvent = serde_json::from_str(json).unwrap();
        match event {
            VoiceEvent::Transcript {
                role,
                text,
                is_final,
            } => {
                assert_eq!(role, TranscriptRole::User);
                assert_eq!(text, "Hello");
                assert!(is_final);
            }
            other => panic!("expected Transcript, got: {other:?}"),
        }
    }

    #[test]
    fn voice_event_transcript_interim_deserializes() {
        let json = r#"{"type":"transcript","role":"agent","text":"I think","is_final":false}"#;
        let event: VoiceEvent = serde_json::from_str(json).unwrap();
        match event {
            VoiceEvent::Transcript {
                role,
                text,
                is_final,
            } => {
                assert_eq!(role, TranscriptRole::Agent);
                assert_eq!(text, "I think");
                assert!(!is_final);
            }
            other => panic!("expected Transcript, got: {other:?}"),
        }
    }

    #[test]
    fn voice_event_transcript_default_is_final() {
        // is_final defaults to false if omitted.
        let json = r#"{"type":"transcript","role":"user","text":"Hi"}"#;
        let event: VoiceEvent = serde_json::from_str(json).unwrap();
        match event {
            VoiceEvent::Transcript { is_final, .. } => {
                assert!(!is_final, "is_final should default to false");
            }
            other => panic!("expected Transcript, got: {other:?}"),
        }
    }

    #[test]
    fn voice_event_function_call_deserializes() {
        let json = r#"{"type":"function_call","call_id":"call_1","name":"get_weather","arguments":"{\"city\":\"SF\"}"}"#;
        let event: VoiceEvent = serde_json::from_str(json).unwrap();
        match event {
            VoiceEvent::FunctionCall {
                call_id,
                name,
                arguments,
            } => {
                assert_eq!(call_id, "call_1");
                assert_eq!(name, "get_weather");
                assert!(arguments.contains("SF"));
            }
            other => panic!("expected FunctionCall, got: {other:?}"),
        }
    }

    #[test]
    fn voice_event_state_change_deserializes() {
        let json = r#"{"type":"state_change","state":"listening","reason":"ready"}"#;
        let event: VoiceEvent = serde_json::from_str(json).unwrap();
        match event {
            VoiceEvent::StateChange { state, reason } => {
                assert_eq!(state, VoiceSessionState::Listening);
                assert_eq!(reason.as_deref(), Some("ready"));
            }
            other => panic!("expected StateChange, got: {other:?}"),
        }
    }

    #[test]
    fn voice_event_error_deserializes() {
        let json =
            r#"{"type":"error","code":"rate_limit","message":"too many requests","fatal":false}"#;
        let event: VoiceEvent = serde_json::from_str(json).unwrap();
        match event {
            VoiceEvent::Error {
                code,
                message,
                fatal,
            } => {
                assert_eq!(code, "rate_limit");
                assert_eq!(message, "too many requests");
                assert!(!fatal);
            }
            other => panic!("expected Error, got: {other:?}"),
        }
    }

    #[test]
    fn voice_event_error_fatal_deserializes() {
        let json =
            r#"{"type":"error","code":"internal_error","message":"server crashed","fatal":true}"#;
        let event: VoiceEvent = serde_json::from_str(json).unwrap();
        match event {
            VoiceEvent::Error { fatal, .. } => {
                assert!(fatal);
            }
            other => panic!("expected Error, got: {other:?}"),
        }
    }

    #[test]
    fn voice_event_pong_deserializes() {
        let json = r#"{"type":"pong"}"#;
        let event: VoiceEvent = serde_json::from_str(json).unwrap();
        assert!(matches!(event, VoiceEvent::Pong));
    }

    #[test]
    fn voice_event_audio_chunk_deserializes() {
        let json = r#"{"type":"audio_chunk","sequence":42,"duration_ms":20}"#;
        let event: VoiceEvent = serde_json::from_str(json).unwrap();
        match event {
            VoiceEvent::AudioChunk {
                sequence,
                duration_ms,
            } => {
                assert_eq!(sequence, 42);
                assert_eq!(duration_ms, Some(20));
            }
            other => panic!("expected AudioChunk, got: {other:?}"),
        }
    }

    #[test]
    fn voice_event_usage_deserializes() {
        let json = r#"{"type":"usage","input_tokens":100,"output_tokens":200,"input_audio_secs":5.0,"output_audio_secs":8.5}"#;
        let event: VoiceEvent = serde_json::from_str(json).unwrap();
        match event {
            VoiceEvent::Usage {
                input_tokens,
                output_tokens,
                input_audio_secs,
                output_audio_secs,
            } => {
                assert_eq!(input_tokens, 100);
                assert_eq!(output_tokens, 200);
                assert!((input_audio_secs - 5.0).abs() < f64::EPSILON);
                assert!((output_audio_secs - 8.5).abs() < f64::EPSILON);
            }
            other => panic!("expected Usage, got: {other:?}"),
        }
    }

    // --- VoiceSessionState ---

    #[test]
    fn voice_session_state_all_variants_serialize() {
        let states = vec![
            (VoiceSessionState::Initializing, "initializing"),
            (VoiceSessionState::Listening, "listening"),
            (VoiceSessionState::Thinking, "thinking"),
            (VoiceSessionState::Speaking, "speaking"),
            (VoiceSessionState::WaitingForTool, "waiting_for_tool"),
            (VoiceSessionState::Closing, "closing"),
            (VoiceSessionState::Closed, "closed"),
            (VoiceSessionState::Error, "error"),
        ];
        for (state, expected) in states {
            let json = serde_json::to_string(&state).unwrap();
            assert_eq!(json, format!("\"{expected}\""), "state: {state:?}");
            let parsed: VoiceSessionState = serde_json::from_str(&json).unwrap();
            assert_eq!(parsed, state);
        }
    }

    #[test]
    fn voice_session_state_display() {
        assert_eq!(VoiceSessionState::Listening.to_string(), "listening");
        assert_eq!(
            VoiceSessionState::WaitingForTool.to_string(),
            "waiting_for_tool"
        );
    }

    // --- VoiceId ---

    #[test]
    fn voice_id_from_str() {
        assert_eq!("eve".parse::<VoiceId>().unwrap(), VoiceId::Eve);
        assert_eq!("EVE".parse::<VoiceId>().unwrap(), VoiceId::Eve);
        assert_eq!("Rex".parse::<VoiceId>().unwrap(), VoiceId::Rex);
        assert!("unknown".parse::<VoiceId>().is_err());
    }

    #[test]
    fn voice_id_display() {
        assert_eq!(VoiceId::Eve.to_string(), "eve");
        assert_eq!(VoiceId::Sal.to_string(), "sal");
    }

    #[test]
    fn voice_id_roundtrips() {
        for voice in &[
            VoiceId::Eve,
            VoiceId::Ara,
            VoiceId::Rex,
            VoiceId::Sal,
            VoiceId::Leo,
        ] {
            let json = serde_json::to_string(voice).unwrap();
            let parsed: VoiceId = serde_json::from_str(&json).unwrap();
            assert_eq!(&parsed, voice);
        }
    }

    // --- AudioFormat ---

    #[test]
    fn audio_format_default() {
        let fmt = AudioFormat::default();
        assert_eq!(fmt.encoding, AudioEncoding::Pcm16);
        assert_eq!(fmt.sample_rate, 24000);
        assert_eq!(fmt.channels, 1);
    }

    #[test]
    fn audio_format_serializes() {
        let fmt = AudioFormat::default();
        let json = serde_json::to_string(&fmt).unwrap();
        assert!(json.contains("\"encoding\":\"pcm16\""));
        assert!(json.contains("\"sample_rate\":24000"));
        assert!(json.contains("\"channels\":1"));
    }

    // --- TranscriptRole ---

    #[test]
    fn transcript_role_display() {
        assert_eq!(TranscriptRole::User.to_string(), "user");
        assert_eq!(TranscriptRole::Agent.to_string(), "agent");
    }

    // --- ControlAction ---

    #[test]
    fn control_action_all_variants_serialize() {
        let actions = vec![
            (ControlAction::EndTurn, "end_turn"),
            (ControlAction::Interrupt, "interrupt"),
            (ControlAction::Close, "close"),
            (ControlAction::Ping, "ping"),
        ];
        for (action, expected) in actions {
            let json = serde_json::to_string(&action).unwrap();
            assert_eq!(json, format!("\"{expected}\""), "action: {action:?}");
        }
    }

    // --- VoiceSessionSummary ---

    #[test]
    fn voice_session_summary_deserializes() {
        let json = r#"{"session_id":"s_1","duration_secs":120.5,"turn_count":5,"input_tokens":500,"output_tokens":800,"input_audio_secs":45.0,"output_audio_secs":60.0}"#;
        let summary: VoiceSessionSummary = serde_json::from_str(json).unwrap();
        assert_eq!(summary.session_id, "s_1");
        assert!((summary.duration_secs - 120.5).abs() < f64::EPSILON);
        assert_eq!(summary.turn_count, 5);
        assert_eq!(summary.input_tokens, 500);
        assert_eq!(summary.output_tokens, 800);
    }

    // --- AudioData ---

    #[test]
    fn audio_data_construction() {
        let data = AudioData {
            pcm_data: vec![0u8; 960],
            sequence: 1,
        };
        assert_eq!(data.pcm_data.len(), 960);
        assert_eq!(data.sequence, 1);
    }
}
