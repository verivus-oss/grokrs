//! Integration tests for headless agent JSON output and exit codes.
//!
//! These tests validate the stable CLI API for `HeadlessEvent` JSON serialization
//! and `AgentExitCode` mapping. They do NOT launch the full CLI binary or make
//! API calls; instead they exercise the types and mapping functions directly.
//!
//! The agent command's headless mode emits newline-delimited JSON events (NDJSON)
//! to stdout. Scripts and CI pipelines parse these events, so the format is a
//! stable API contract.

use grokrs_cli::commands::agent::{AgentExitCode, HeadlessEvent};

// ---------------------------------------------------------------------------
// HeadlessEvent serialization
// ---------------------------------------------------------------------------

#[test]
fn headless_event_tool_call_serializes_to_ndjson() {
    let event = HeadlessEvent::ToolCall {
        name: "read_file".to_owned(),
        arguments: r#"{"path":"src/main.rs"}"#.to_owned(),
    };
    let json = serde_json::to_string(&event).unwrap();
    assert!(json.contains(r#""type":"tool_call""#));
    assert!(json.contains(r#""name":"read_file""#));
    assert!(json.contains(r#""arguments":"#));

    // Should deserialize back.
    let parsed: serde_json::Value = serde_json::from_str(&json).unwrap();
    assert_eq!(parsed["type"], "tool_call");
    assert_eq!(parsed["name"], "read_file");
}

#[test]
fn headless_event_tool_result_serializes() {
    let event = HeadlessEvent::ToolResult {
        name: "read_file".to_owned(),
        output: "fn main() { println!(\"hello\"); }".to_owned(),
    };
    let json = serde_json::to_string(&event).unwrap();
    let parsed: serde_json::Value = serde_json::from_str(&json).unwrap();
    assert_eq!(parsed["type"], "tool_result");
    assert_eq!(parsed["name"], "read_file");
    assert!(parsed["output"].as_str().unwrap().contains("println"));
}

#[test]
fn headless_event_message_serializes() {
    let event = HeadlessEvent::Message {
        content: "The task is complete. I've refactored the error handling.".to_owned(),
    };
    let json = serde_json::to_string(&event).unwrap();
    let parsed: serde_json::Value = serde_json::from_str(&json).unwrap();
    assert_eq!(parsed["type"], "message");
    assert!(parsed["content"].as_str().unwrap().contains("refactored"));
}

#[test]
fn headless_event_error_serializes_with_exit_code() {
    let event = HeadlessEvent::Error {
        message: "Policy denied: network access is denied".to_owned(),
        exit_code: 2,
    };
    let json = serde_json::to_string(&event).unwrap();
    let parsed: serde_json::Value = serde_json::from_str(&json).unwrap();
    assert_eq!(parsed["type"], "error");
    assert_eq!(parsed["exit_code"], 2);
    assert!(
        parsed["message"]
            .as_str()
            .unwrap()
            .contains("Policy denied")
    );
}

#[test]
fn headless_event_usage_serializes() {
    let event = HeadlessEvent::Usage {
        input_tokens: 1500,
        output_tokens: 2000,
        total_tokens: 3500,
    };
    let json = serde_json::to_string(&event).unwrap();
    let parsed: serde_json::Value = serde_json::from_str(&json).unwrap();
    assert_eq!(parsed["type"], "usage");
    assert_eq!(parsed["input_tokens"], 1500);
    assert_eq!(parsed["output_tokens"], 2000);
    assert_eq!(parsed["total_tokens"], 3500);
}

// ---------------------------------------------------------------------------
// AgentExitCode values (stable API)
// ---------------------------------------------------------------------------

#[test]
fn exit_code_success_is_zero() {
    assert_eq!(AgentExitCode::Success.code(), 0i32);
}

#[test]
fn exit_code_agent_error_is_one() {
    assert_eq!(AgentExitCode::AgentError.code(), 1i32);
}

#[test]
fn exit_code_policy_denial_is_two() {
    assert_eq!(AgentExitCode::PolicyDenial.code(), 2i32);
}

#[test]
fn exit_code_api_error_is_three() {
    assert_eq!(AgentExitCode::ApiError.code(), 3i32);
}

#[test]
fn exit_code_timeout_is_four() {
    assert_eq!(AgentExitCode::Timeout.code(), 4i32);
}

// ---------------------------------------------------------------------------
// NDJSON stream simulation
// ---------------------------------------------------------------------------

#[test]
fn ndjson_stream_of_events_is_parseable() {
    // Simulate a full headless session output.
    let events = [
        HeadlessEvent::ToolCall {
            name: "read_file".to_owned(),
            arguments: r#"{"path":"Cargo.toml"}"#.to_owned(),
        },
        HeadlessEvent::ToolResult {
            name: "read_file".to_owned(),
            output: "[package]\nname = \"grokrs\"".to_owned(),
        },
        HeadlessEvent::ToolCall {
            name: "write_file".to_owned(),
            arguments: r##"{"path":"README.md","content":"# grokrs"}"##.to_owned(),
        },
        HeadlessEvent::ToolResult {
            name: "write_file".to_owned(),
            output: "wrote 9 bytes to README.md".to_owned(),
        },
        HeadlessEvent::Message {
            content: "Done! I created a README.md file.".to_owned(),
        },
        HeadlessEvent::Usage {
            input_tokens: 500,
            output_tokens: 300,
            total_tokens: 800,
        },
    ];

    // Serialize as NDJSON (one JSON object per line).
    let ndjson: String = events
        .iter()
        .map(|e| serde_json::to_string(e).unwrap())
        .collect::<Vec<_>>()
        .join("\n");

    // Parse each line independently (as a consumer would).
    let lines: Vec<&str> = ndjson.lines().collect();
    assert_eq!(lines.len(), 6);

    for line in &lines {
        let parsed: serde_json::Value = serde_json::from_str(line).unwrap();
        assert!(
            parsed.get("type").is_some(),
            "each NDJSON line must have a 'type' field"
        );
    }

    // Verify event type sequence.
    let types: Vec<&str> = lines
        .iter()
        .map(|l| {
            let v: serde_json::Value = serde_json::from_str(l).unwrap();
            match v["type"].as_str().unwrap() {
                "tool_call" => "tool_call",
                "tool_result" => "tool_result",
                "message" => "message",
                "usage" => "usage",
                "error" => "error",
                other => panic!("unexpected type: {other}"),
            }
        })
        .collect();
    assert_eq!(
        types,
        vec![
            "tool_call",
            "tool_result",
            "tool_call",
            "tool_result",
            "message",
            "usage"
        ]
    );
}

// ---------------------------------------------------------------------------
// Error event with various exit codes
// ---------------------------------------------------------------------------

#[test]
fn error_events_carry_correct_exit_codes() {
    let test_cases = vec![
        (AgentExitCode::AgentError, 1, "max iterations exceeded"),
        (
            AgentExitCode::PolicyDenial,
            2,
            "network access denied by policy",
        ),
        (AgentExitCode::ApiError, 3, "HTTP 429: rate limited"),
        (
            AgentExitCode::Timeout,
            4,
            "execution exceeded 300s deadline",
        ),
    ];

    for (exit_code, expected_code, message) in test_cases {
        assert_eq!(exit_code.code(), expected_code);

        // RATIONALE: AgentExitCode values are 0–4, always fit in u8.
        #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
        let code_u8 = exit_code.code() as u8;
        let event = HeadlessEvent::Error {
            message: message.to_owned(),
            exit_code: code_u8,
        };
        let json = serde_json::to_string(&event).unwrap();
        let parsed: serde_json::Value = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed["exit_code"], expected_code);
    }
}

// ---------------------------------------------------------------------------
// Headless events with special characters
// ---------------------------------------------------------------------------

#[test]
fn headless_event_with_newlines_and_quotes_in_content() {
    let event = HeadlessEvent::Message {
        content: "Line 1\nLine 2\n\"quoted\" text\ttab".to_owned(),
    };
    let json = serde_json::to_string(&event).unwrap();
    // The JSON should escape special characters.
    assert!(json.contains("\\n"));
    assert!(json.contains("\\\""));
    assert!(json.contains("\\t"));

    // Should parse back correctly.
    let parsed: serde_json::Value = serde_json::from_str(&json).unwrap();
    let content = parsed["content"].as_str().unwrap();
    assert!(content.contains('\n'));
    assert!(content.contains('"'));
    assert!(content.contains('\t'));
}

#[test]
fn headless_event_with_unicode_content() {
    let event = HeadlessEvent::Message {
        content: "Hello \u{1F600} world \u{2764} Rust".to_owned(),
    };
    let json = serde_json::to_string(&event).unwrap();
    let parsed: serde_json::Value = serde_json::from_str(&json).unwrap();
    let content = parsed["content"].as_str().unwrap();
    assert!(content.contains('\u{1F600}'));
    assert!(content.contains('\u{2764}'));
}

#[test]
fn headless_event_with_empty_strings() {
    let event = HeadlessEvent::ToolCall {
        name: String::new(),
        arguments: "{}".to_owned(),
    };
    let json = serde_json::to_string(&event).unwrap();
    let parsed: serde_json::Value = serde_json::from_str(&json).unwrap();
    assert_eq!(parsed["name"], "");
    assert_eq!(parsed["arguments"], "{}");
}
