//! Integration tests for the Voice Agent WebSocket types and protocol.
//!
//! Since setting up a full mock WebSocket server is complex and would require
//! additional dependencies, these tests thoroughly exercise the type serialization
//! and deserialization that constitutes the WebSocket protocol wire format.
//!
//! The tests validate:
//! - Full conversation flow type serialization (config -> messages -> events)
//! - Complex multi-message sequences
//! - Edge cases in message framing
//! - `VoiceAgentClient` construction and configuration
//! - `VoiceReceived` enum handling
//! - `AudioData` binary framing assumptions
//!
//! No real WebSocket connections or API keys are needed.

use serde_json::json;
use std::sync::Arc;

use grokrs_api::endpoints::voice::{VoiceAgentClient, VoiceReceived};
use grokrs_api::transport::auth::ApiKeySecret;
use grokrs_api::transport::policy_gate::AllowAllGate;
use grokrs_api::transport::websocket::WsClientConfig;
use grokrs_api::types::voice::*;

// ---------------------------------------------------------------------------
// Full conversation flow serialization
// ---------------------------------------------------------------------------

/// Simulate the full wire-level message sequence of a voice session:
/// 1. Client sends `SessionConfig`
/// 2. Server sends `SessionCreated`
/// 3. Client sends `TextInput`
/// 4. Server sends `StateChange(Thinking)`
/// 5. Server sends Transcript(Agent, interim)
/// 6. Server sends Transcript(Agent, final)
/// 7. Server sends `AudioChunk` metadata
/// 8. Server sends Usage
/// 9. Client sends Control(Close)
/// 10. Server sends StateChange(Closed)
// End-to-end serialization test covering the full 12-step voice conversation
// protocol; splitting would break the sequential flow narrative the test documents.
#[allow(clippy::too_many_lines)]
#[tokio::test]
async fn full_conversation_flow_serialization() {
    // 1. Client -> Server: Session config
    let config = VoiceConfig {
        model: "grok-4".to_owned(),
        voice: VoiceId::Rex,
        language: "en-US".to_owned(),
        system_instructions: Some("You are a helpful assistant.".to_owned()),
        silence_timeout_ms: Some(5000),
        max_duration_secs: Some(600),
        tools: Some(vec![ToolDefinition {
            tool_type: "function".to_owned(),
            function: FunctionDefinition {
                name: "get_weather".to_owned(),
                description: "Get current weather for a city".to_owned(),
                parameters: json!({
                    "type": "object",
                    "properties": {
                        "city": {"type": "string"},
                        "units": {"type": "string", "enum": ["celsius", "fahrenheit"]}
                    },
                    "required": ["city"]
                }),
            },
        }]),
        ..VoiceConfig::default()
    };
    let config_msg = VoiceMessage::SessionConfig(config.clone());
    let config_json = serde_json::to_string(&config_msg).unwrap();
    assert!(config_json.contains("\"type\":\"session_config\""));
    assert!(config_json.contains("\"model\":\"grok-4\""));
    assert!(config_json.contains("\"voice\":\"rex\""));
    assert!(config_json.contains("get_weather"));

    // Verify round-trip
    let parsed_msg: VoiceMessage = serde_json::from_str(&config_json).unwrap();
    match parsed_msg {
        VoiceMessage::SessionConfig(c) => {
            assert_eq!(c.model, "grok-4");
            assert_eq!(c.voice, VoiceId::Rex);
            assert_eq!(
                c.system_instructions.as_deref(),
                Some("You are a helpful assistant.")
            );
            assert_eq!(c.tools.unwrap().len(), 1);
        }
        other => panic!("expected SessionConfig, got: {other:?}"),
    }

    // 2. Server -> Client: Session created
    let created_json = json!({
        "type": "session_created",
        "session_id": "vs_abc123",
        "config": {
            "model": "grok-4",
            "voice": "rex",
            "language": "en-US",
            "turn_detection": "server_vad",
            "vad_sensitivity": "medium",
            "input_audio_format": {"encoding": "pcm16", "sample_rate": 24000, "channels": 1},
            "output_audio_format": {"encoding": "pcm16", "sample_rate": 24000, "channels": 1}
        }
    });
    let event: VoiceEvent = serde_json::from_value(created_json).unwrap();
    match event {
        VoiceEvent::SessionCreated { session_id, config } => {
            assert_eq!(session_id, "vs_abc123");
            let c = config.unwrap();
            assert_eq!(c.model, "grok-4");
            assert_eq!(c.voice, VoiceId::Rex);
        }
        other => panic!("expected SessionCreated, got: {other:?}"),
    }

    // 3. Client -> Server: Text input
    let text_msg = VoiceMessage::TextInput {
        text: "What's the weather in San Francisco?".to_owned(),
    };
    let text_json = serde_json::to_string(&text_msg).unwrap();
    assert!(text_json.contains("\"type\":\"text_input\""));
    assert!(text_json.contains("San Francisco"));

    // 4. Server -> Client: State change to Thinking
    let thinking_json = r#"{"type":"state_change","state":"thinking","reason":"processing input"}"#;
    let event: VoiceEvent = serde_json::from_str(thinking_json).unwrap();
    match event {
        VoiceEvent::StateChange { state, reason } => {
            assert_eq!(state, VoiceSessionState::Thinking);
            assert_eq!(reason.as_deref(), Some("processing input"));
        }
        other => panic!("expected StateChange, got: {other:?}"),
    }

    // 5. Server -> Client: Function call request
    let fc_json = json!({
        "type": "function_call",
        "call_id": "call_weather_1",
        "name": "get_weather",
        "arguments": "{\"city\":\"San Francisco\",\"units\":\"celsius\"}"
    });
    let event: VoiceEvent = serde_json::from_value(fc_json).unwrap();
    match event {
        VoiceEvent::FunctionCall {
            call_id,
            name,
            arguments,
        } => {
            assert_eq!(call_id, "call_weather_1");
            assert_eq!(name, "get_weather");
            let args: serde_json::Value = serde_json::from_str(&arguments).unwrap();
            assert_eq!(args["city"], "San Francisco");
        }
        other => panic!("expected FunctionCall, got: {other:?}"),
    }

    // 6. Client -> Server: Function result
    let result_msg = VoiceMessage::FunctionResult {
        call_id: "call_weather_1".to_owned(),
        result: r#"{"temperature":18,"condition":"foggy"}"#.to_owned(),
    };
    let result_json = serde_json::to_string(&result_msg).unwrap();
    assert!(result_json.contains("\"type\":\"function_result\""));
    assert!(result_json.contains("call_weather_1"));

    // 7. Server -> Client: Interim transcript
    let interim_json = json!({
        "type": "transcript",
        "role": "agent",
        "text": "The weather in San",
        "is_final": false
    });
    let event: VoiceEvent = serde_json::from_value(interim_json).unwrap();
    match event {
        VoiceEvent::Transcript {
            role,
            text,
            is_final,
        } => {
            assert_eq!(role, TranscriptRole::Agent);
            assert_eq!(text, "The weather in San");
            assert!(!is_final);
        }
        other => panic!("expected Transcript, got: {other:?}"),
    }

    // 8. Server -> Client: Final transcript
    let final_json = json!({
        "type": "transcript",
        "role": "agent",
        "text": "The weather in San Francisco is 18 degrees and foggy.",
        "is_final": true
    });
    let event: VoiceEvent = serde_json::from_value(final_json).unwrap();
    match &event {
        VoiceEvent::Transcript { text, is_final, .. } => {
            assert!(text.contains("18 degrees"));
            assert!(is_final);
        }
        other => panic!("expected Transcript, got: {other:?}"),
    }

    // 9. Server -> Client: Audio chunk metadata
    let audio_json = json!({
        "type": "audio_chunk",
        "sequence": 0,
        "duration_ms": 20
    });
    let event: VoiceEvent = serde_json::from_value(audio_json).unwrap();
    match event {
        VoiceEvent::AudioChunk {
            sequence,
            duration_ms,
        } => {
            assert_eq!(sequence, 0);
            assert_eq!(duration_ms, Some(20));
        }
        other => panic!("expected AudioChunk, got: {other:?}"),
    }

    // 10. Server -> Client: Usage statistics
    let usage_json = json!({
        "type": "usage",
        "input_tokens": 250,
        "output_tokens": 180,
        "input_audio_secs": 3.5,
        "output_audio_secs": 5.2
    });
    let event: VoiceEvent = serde_json::from_value(usage_json).unwrap();
    match event {
        VoiceEvent::Usage {
            input_tokens,
            output_tokens,
            input_audio_secs,
            output_audio_secs,
        } => {
            assert_eq!(input_tokens, 250);
            assert_eq!(output_tokens, 180);
            assert!((input_audio_secs - 3.5).abs() < f64::EPSILON);
            assert!((output_audio_secs - 5.2).abs() < f64::EPSILON);
        }
        other => panic!("expected Usage, got: {other:?}"),
    }

    // 11. Client -> Server: Close
    let close_msg = VoiceMessage::Control {
        action: ControlAction::Close,
    };
    let close_json = serde_json::to_string(&close_msg).unwrap();
    assert!(close_json.contains("\"action\":\"close\""));

    // 12. Server -> Client: Session closed
    let state_change_json = json!({
        "type": "state_change",
        "state": "closed",
        "reason": "client requested close"
    });
    let event: VoiceEvent = serde_json::from_value(state_change_json).unwrap();
    match event {
        VoiceEvent::StateChange { state, reason } => {
            assert_eq!(state, VoiceSessionState::Closed);
            assert_eq!(reason.as_deref(), Some("client requested close"));
        }
        other => panic!("expected StateChange, got: {other:?}"),
    }
}

// ---------------------------------------------------------------------------
// VoiceAgentClient construction and configuration
// ---------------------------------------------------------------------------

#[tokio::test]
async fn voice_client_construction_default_config() {
    let client = VoiceAgentClient::new(
        WsClientConfig::default(),
        ApiKeySecret::new("test-key"),
        None,
    );
    let ws_config = client.ws_config();
    assert_eq!(ws_config.heartbeat_interval_secs, 30); // default
}

#[tokio::test]
async fn voice_client_construction_custom_config() {
    let config = WsClientConfig {
        heartbeat_interval_secs: 10,
        ..WsClientConfig::default()
    };
    let client = VoiceAgentClient::new(config, ApiKeySecret::new("key"), None);
    assert_eq!(client.ws_config().heartbeat_interval_secs, 10);
}

#[tokio::test]
async fn voice_client_with_policy_gate() {
    let gate: Arc<dyn grokrs_api::transport::policy_gate::PolicyGate> = Arc::new(AllowAllGate);
    let client = VoiceAgentClient::new(
        WsClientConfig::default(),
        ApiKeySecret::new("test-secret-api-key-xyz"),
        Some(gate),
    );
    let debug = format!("{client:?}");
    assert!(debug.contains("Some(<PolicyGate>)"));
    assert!(!debug.contains("test-secret-api-key-xyz")); // API key value redacted
}

// ---------------------------------------------------------------------------
// VoiceReceived enum
// ---------------------------------------------------------------------------

#[tokio::test]
async fn voice_received_event_construction() {
    let event = VoiceEvent::Pong;
    let received = VoiceReceived::Event(event);
    match &received {
        VoiceReceived::Event(VoiceEvent::Pong) => {}
        other => panic!("expected Event(Pong), got: {other:?}"),
    }
}

#[tokio::test]
async fn voice_received_audio_construction() {
    let pcm = vec![0u8; 1920]; // 20ms of 16-bit 24kHz mono
    let received = VoiceReceived::Audio(pcm.clone());
    match &received {
        VoiceReceived::Audio(data) => {
            assert_eq!(data.len(), 1920);
            assert_eq!(*data, pcm);
        }
        other => panic!("expected Audio, got: {other:?}"),
    }
}

// ---------------------------------------------------------------------------
// AudioData type
// ---------------------------------------------------------------------------

#[tokio::test]
async fn audio_data_construction_and_sizing() {
    // 20ms of 16-bit 24kHz mono = 24000 * 0.020 * 2 bytes = 960 bytes
    // RATIONALE: the result (960) is a known positive compile-time constant;
    // the floating-point multiplication is only for readability.
    #[allow(clippy::cast_sign_loss, clippy::cast_possible_truncation)]
    let frame_size = (24000.0_f64 * 0.020 * 2.0) as usize;
    assert_eq!(frame_size, 960);

    let data = AudioData {
        pcm_data: vec![0u8; frame_size],
        sequence: 42,
    };
    assert_eq!(data.pcm_data.len(), 960);
    assert_eq!(data.sequence, 42);
}

// ---------------------------------------------------------------------------
// VoiceSessionSummary
// ---------------------------------------------------------------------------

#[tokio::test]
async fn voice_session_summary_round_trip() {
    let summary = VoiceSessionSummary {
        session_id: "vs_summary_test".to_owned(),
        duration_secs: 185.5,
        turn_count: 12,
        input_tokens: 1500,
        output_tokens: 2000,
        input_audio_secs: 90.0,
        output_audio_secs: 95.5,
    };

    let json = serde_json::to_string(&summary).unwrap();
    let parsed: VoiceSessionSummary = serde_json::from_str(&json).unwrap();

    assert_eq!(parsed.session_id, "vs_summary_test");
    assert!((parsed.duration_secs - 185.5).abs() < f64::EPSILON);
    assert_eq!(parsed.turn_count, 12);
    assert_eq!(parsed.input_tokens, 1500);
    assert_eq!(parsed.output_tokens, 2000);
    assert!((parsed.input_audio_secs - 90.0).abs() < f64::EPSILON);
    assert!((parsed.output_audio_secs - 95.5).abs() < f64::EPSILON);
}

// ---------------------------------------------------------------------------
// Error handling in voice events
// ---------------------------------------------------------------------------

#[tokio::test]
async fn voice_error_events_fatal_and_non_fatal() {
    // Non-fatal error
    let non_fatal = json!({
        "type": "error",
        "code": "rate_limit",
        "message": "Too many requests, please slow down",
        "fatal": false
    });
    let event: VoiceEvent = serde_json::from_value(non_fatal).unwrap();
    match event {
        VoiceEvent::Error {
            code,
            message,
            fatal,
        } => {
            assert_eq!(code, "rate_limit");
            assert!(message.contains("slow down"));
            assert!(!fatal);
        }
        other => panic!("expected Error, got: {other:?}"),
    }

    // Fatal error
    let fatal = json!({
        "type": "error",
        "code": "internal_error",
        "message": "Server encountered an unexpected error",
        "fatal": true
    });
    let event: VoiceEvent = serde_json::from_value(fatal).unwrap();
    match event {
        VoiceEvent::Error { fatal, .. } => assert!(fatal),
        other => panic!("expected Error, got: {other:?}"),
    }

    // Error with fatal defaulting to false when omitted
    let default_fatal = json!({
        "type": "error",
        "code": "transient",
        "message": "Temporary glitch"
    });
    let event: VoiceEvent = serde_json::from_value(default_fatal).unwrap();
    match event {
        VoiceEvent::Error { fatal, .. } => {
            assert!(!fatal, "fatal should default to false");
        }
        other => panic!("expected Error, got: {other:?}"),
    }
}

// ---------------------------------------------------------------------------
// All voice session states
// ---------------------------------------------------------------------------

#[tokio::test]
async fn all_session_state_transitions_deserialize() {
    let states = vec![
        ("initializing", VoiceSessionState::Initializing),
        ("listening", VoiceSessionState::Listening),
        ("thinking", VoiceSessionState::Thinking),
        ("speaking", VoiceSessionState::Speaking),
        ("waiting_for_tool", VoiceSessionState::WaitingForTool),
        ("closing", VoiceSessionState::Closing),
        ("closed", VoiceSessionState::Closed),
        ("error", VoiceSessionState::Error),
    ];

    for (state_str, expected_state) in states {
        let json = json!({
            "type": "state_change",
            "state": state_str
        });
        let event: VoiceEvent = serde_json::from_value(json).unwrap();
        match event {
            VoiceEvent::StateChange { state, reason } => {
                assert_eq!(state, expected_state, "state mismatch for {state_str}");
                assert!(reason.is_none());
            }
            other => panic!("expected StateChange for {state_str}, got: {other:?}"),
        }
    }
}

// ---------------------------------------------------------------------------
// All control actions
// ---------------------------------------------------------------------------

#[tokio::test]
async fn all_control_actions_round_trip() {
    let actions = vec![
        (ControlAction::EndTurn, "end_turn"),
        (ControlAction::Interrupt, "interrupt"),
        (ControlAction::Close, "close"),
        (ControlAction::Ping, "ping"),
    ];

    for (action, expected_str) in actions {
        let msg = VoiceMessage::Control { action };
        let json = serde_json::to_string(&msg).unwrap();
        assert!(
            json.contains(expected_str),
            "expected '{expected_str}' in {json}"
        );
        let parsed: VoiceMessage = serde_json::from_str(&json).unwrap();
        match parsed {
            VoiceMessage::Control {
                action: parsed_action,
            } => {
                assert_eq!(parsed_action, action);
            }
            other => panic!("expected Control, got: {other:?}"),
        }
    }
}

// ---------------------------------------------------------------------------
// VoiceConfig edge cases
// ---------------------------------------------------------------------------

#[tokio::test]
async fn voice_config_all_audio_encodings() {
    let encodings = [
        (AudioEncoding::Pcm16, "pcm16"),
        (AudioEncoding::Mulaw, "mulaw"),
        (AudioEncoding::Alaw, "alaw"),
    ];
    for (encoding, expected_str) in &encodings {
        let fmt = AudioFormat {
            encoding: *encoding,
            sample_rate: 8000,
            channels: 1,
        };
        let json = serde_json::to_string(&fmt).unwrap();
        assert!(
            json.contains(expected_str),
            "expected '{expected_str}' in {json}"
        );
        let parsed: AudioFormat = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed.encoding, *encoding);
    }
}

#[tokio::test]
async fn voice_config_manual_turn_detection() {
    let config = VoiceConfig {
        turn_detection: TurnDetectionMode::Manual,
        ..VoiceConfig::default()
    };
    let json = serde_json::to_string(&config).unwrap();
    assert!(json.contains("\"turn_detection\":\"manual\""));
    let parsed: VoiceConfig = serde_json::from_str(&json).unwrap();
    assert_eq!(parsed.turn_detection, TurnDetectionMode::Manual);
}

#[tokio::test]
async fn voice_config_multiple_tools() {
    let config = VoiceConfig {
        tools: Some(vec![
            ToolDefinition {
                tool_type: "function".to_owned(),
                function: FunctionDefinition {
                    name: "search".to_owned(),
                    description: "Search the web".to_owned(),
                    parameters: json!({"type": "object", "properties": {"query": {"type": "string"}}}),
                },
            },
            ToolDefinition {
                tool_type: "function".to_owned(),
                function: FunctionDefinition {
                    name: "calculate".to_owned(),
                    description: "Evaluate a math expression".to_owned(),
                    parameters: json!({"type": "object", "properties": {"expression": {"type": "string"}}}),
                },
            },
        ]),
        ..VoiceConfig::default()
    };

    let json = serde_json::to_string(&config).unwrap();
    let parsed: VoiceConfig = serde_json::from_str(&json).unwrap();
    let tools = parsed.tools.unwrap();
    assert_eq!(tools.len(), 2);
    assert_eq!(tools[0].function.name, "search");
    assert_eq!(tools[1].function.name, "calculate");
}
