//! Integration tests for encrypted reasoning round-trip.
//!
//! These tests exercise the full cycle of:
//! 1. Server returning a `ResponseObject` with `OutputItem::Reasoning` containing
//!    `encrypted_content`.
//! 2. Client extracting the encrypted blob and constructing an `InputItem::Reasoning`
//!    for the next turn.
//! 3. Serialization fidelity — the blob survives a full serialize-deserialize-serialize
//!    cycle bit-for-bit.
//!
//! All tests use wiremock to serve mock Responses API payloads. No real API calls.

use serde_json::json;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

use grokrs_api::types::common::{ContentBlock, MessageContent};
use grokrs_api::types::responses::{
    InputItem, OutputItem, ReasoningConfig, ResponseInput, ResponseObject, ResponseStatus,
};

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Build a mock Responses API response with reasoning + `encrypted_content`.
fn mock_reasoning_response(
    encrypted_blob: &str,
    reasoning_text: &str,
    message_text: &str,
) -> serde_json::Value {
    json!({
        "id": "resp_test_001",
        "object": "response",
        "status": "completed",
        "output": [
            {
                "type": "reasoning",
                "id": "rs_test_001",
                "content": [
                    {"type": "thinking", "text": reasoning_text}
                ],
                "encrypted_content": encrypted_blob
            },
            {
                "type": "message",
                "role": "assistant",
                "content": [
                    {"type": "text", "text": message_text}
                ]
            }
        ],
        "usage": {
            "input_tokens": 100,
            "output_tokens": 200,
            "total_tokens": 300
        }
    })
}

/// Build a second-turn mock response (to verify the encrypted replay was received).
fn mock_continuation_response(message_text: &str) -> serde_json::Value {
    json!({
        "id": "resp_test_002",
        "object": "response",
        "status": "completed",
        "output": [
            {
                "type": "message",
                "role": "assistant",
                "content": [
                    {"type": "text", "text": message_text}
                ]
            }
        ],
        "usage": {
            "input_tokens": 150,
            "output_tokens": 100,
            "total_tokens": 250
        }
    })
}

// ---------------------------------------------------------------------------
// Tests: ResponseObject deserialization with encrypted reasoning
// ---------------------------------------------------------------------------

#[tokio::test]
async fn deserialize_response_with_encrypted_reasoning() {
    let blob = "ENC:aGVsbG8td29ybGQ=:base64opaque";
    let body = mock_reasoning_response(blob, "Let me think about this...", "The answer is 42.");

    let resp: ResponseObject = serde_json::from_value(body).expect("should deserialize");
    assert_eq!(resp.id, "resp_test_001");
    assert_eq!(resp.status, ResponseStatus::Completed);
    assert_eq!(resp.output.len(), 2);

    // First output item: Reasoning with encrypted_content.
    match &resp.output[0] {
        OutputItem::Reasoning {
            id,
            content,
            encrypted_content,
        } => {
            assert_eq!(id.as_deref(), Some("rs_test_001"));
            assert_eq!(content.len(), 1);
            assert_eq!(content[0].text, "Let me think about this...");
            assert_eq!(encrypted_content.as_deref(), Some(blob));
        }
        other => panic!("expected Reasoning, got: {other:?}"),
    }

    // Second output item: Message.
    match &resp.output[1] {
        OutputItem::Message { content, .. } => {
            assert_eq!(content.len(), 1);
            match &content[0] {
                ContentBlock::Text { text } => assert_eq!(text, "The answer is 42."),
                other => panic!("expected Text, got: {other:?}"),
            }
        }
        other => panic!("expected Message, got: {other:?}"),
    }
}

#[tokio::test]
async fn encrypted_content_round_trip_preserves_blob() {
    let blob = "ENC:VGhpcyBpcyBhIHRlc3QgYmxvYg==:with-special/chars+and=padding";

    // Simulate receiving a response with encrypted reasoning.
    let response_json = mock_reasoning_response(blob, "Analyzing...", "Done.");
    let resp: ResponseObject = serde_json::from_value(response_json).unwrap();

    // Extract the encrypted content from the response.
    let encrypted = match &resp.output[0] {
        OutputItem::Reasoning {
            encrypted_content, ..
        } => encrypted_content
            .clone()
            .expect("should have encrypted_content"),
        _ => panic!("expected Reasoning"),
    };

    // Verify the blob is identical.
    assert_eq!(encrypted, blob);

    // Construct an InputItem::Reasoning for the next turn.
    let replay_item = InputItem::Reasoning {
        r#type: "reasoning".to_owned(),
        id: Some("rs_test_001".to_owned()),
        encrypted_content: encrypted.clone(),
    };

    // Serialize and deserialize the input item.
    let serialized = serde_json::to_string(&replay_item).unwrap();
    let deserialized: InputItem = serde_json::from_str(&serialized).unwrap();

    // Verify the blob survived the round-trip.
    match deserialized {
        InputItem::Reasoning {
            encrypted_content, ..
        } => {
            assert_eq!(encrypted_content, blob);
        }
        InputItem::Message(_) => panic!("expected Reasoning input item"),
    }
}

#[tokio::test]
async fn encrypted_reasoning_replay_via_wiremock() {
    let mock_server = MockServer::start().await;
    let blob = "ENC:encrypted-reasoning-data-blob-12345";

    // Mount first response: returns reasoning with encrypted_content.
    Mock::given(method("POST"))
        .and(path("/v1/responses"))
        .respond_with(
            ResponseTemplate::new(200).set_body_json(mock_reasoning_response(
                blob,
                "Thinking...",
                "Initial answer.",
            )),
        )
        .expect(1..=2) // allow 1-2 calls
        .mount(&mock_server)
        .await;

    // Simulate the client flow: send first request, get response.
    let client = reqwest::Client::new();
    let first_resp = client
        .post(format!("{}/v1/responses", mock_server.uri()))
        .json(&json!({
            "model": "grok-3-mini",
            "input": "What is the meaning of life?",
            "reasoning": {"effort": "high"}
        }))
        .send()
        .await
        .expect("first request should succeed");

    assert_eq!(first_resp.status(), 200);
    let first_body: ResponseObject = first_resp.json().await.unwrap();

    // Extract encrypted_content and build continuation input.
    let enc_content = match &first_body.output[0] {
        OutputItem::Reasoning {
            id,
            encrypted_content,
            ..
        } => {
            assert!(id.is_some());
            encrypted_content
                .clone()
                .expect("should have encrypted_content")
        }
        other => panic!("expected Reasoning, got: {other:?}"),
    };

    assert_eq!(enc_content, blob);

    // Build the second-turn input with the reasoning replay.
    let continuation_input = ResponseInput::Items(vec![
        InputItem::Reasoning {
            r#type: "reasoning".to_owned(),
            id: Some("rs_test_001".to_owned()),
            encrypted_content: enc_content,
        },
        InputItem::Message(grokrs_api::types::message::InputMessage {
            role: grokrs_api::types::common::Role::User,
            content: MessageContent::Text("Can you elaborate?".to_owned()),
            name: None,
        }),
    ]);

    // Verify the continuation input serializes correctly.
    let input_json = serde_json::to_value(&continuation_input).unwrap();
    let items = input_json.as_array().expect("should be an array");
    assert_eq!(items.len(), 2);

    // First item should be the reasoning replay with encrypted_content.
    assert_eq!(items[0]["type"], "reasoning");
    assert_eq!(items[0]["id"], "rs_test_001");
    assert_eq!(items[0]["encrypted_content"], blob);

    // Second item should be the user message.
    assert_eq!(items[1]["role"], "user");
}

#[tokio::test]
async fn response_without_encrypted_content_has_none() {
    let body = json!({
        "id": "resp_no_enc",
        "object": "response",
        "status": "completed",
        "output": [
            {
                "type": "reasoning",
                "id": "rs_no_enc",
                "content": [{"type": "thinking", "text": "Plain thinking"}]
            }
        ],
        "usage": {"input_tokens": 10, "output_tokens": 20, "total_tokens": 30}
    });

    let resp: ResponseObject = serde_json::from_value(body).unwrap();
    match &resp.output[0] {
        OutputItem::Reasoning {
            encrypted_content, ..
        } => {
            assert!(encrypted_content.is_none(), "expected no encrypted_content");
        }
        other => panic!("expected Reasoning, got: {other:?}"),
    }
}

#[tokio::test]
async fn large_encrypted_blob_survives_round_trip() {
    // 100 KB opaque blob — tests that serialization handles large payloads.
    let large_blob = "ENC:".to_owned() + &"A".repeat(100_000);

    let response_json = mock_reasoning_response(&large_blob, "Thinking big...", "Big answer.");
    let resp: ResponseObject = serde_json::from_value(response_json).unwrap();

    let extracted = match &resp.output[0] {
        OutputItem::Reasoning {
            encrypted_content, ..
        } => encrypted_content.clone().unwrap(),
        _ => panic!("expected Reasoning"),
    };
    assert_eq!(extracted.len(), large_blob.len());
    assert_eq!(extracted, large_blob);

    // Round-trip through InputItem.
    let replay = InputItem::Reasoning {
        r#type: "reasoning".to_owned(),
        id: None,
        encrypted_content: extracted.clone(),
    };
    let serialized = serde_json::to_string(&replay).unwrap();
    let deserialized: InputItem = serde_json::from_str(&serialized).unwrap();
    match deserialized {
        InputItem::Reasoning {
            encrypted_content, ..
        } => assert_eq!(encrypted_content, large_blob),
        InputItem::Message(_) => panic!("expected Reasoning"),
    }
}

#[tokio::test]
async fn encrypted_blob_with_special_characters() {
    // Test that special JSON characters in the blob are handled correctly.
    let blobs = [
        r#"ENC:{"nested":"json"}"#,
        "ENC:line1\nline2\ttab",
        "ENC:unicode-\u{1F600}-emoji",
        "ENC:backslash\\path",
        "ENC:quotes\"inside\"blob",
    ];

    for blob in &blobs {
        let item = OutputItem::Reasoning {
            id: None,
            content: vec![],
            encrypted_content: Some(blob.to_string()),
        };
        let json = serde_json::to_string(&item).unwrap();
        let back: OutputItem = serde_json::from_str(&json).unwrap();
        match back {
            OutputItem::Reasoning {
                encrypted_content, ..
            } => {
                assert_eq!(
                    encrypted_content.as_deref(),
                    Some(*blob),
                    "blob did not survive round-trip: {blob}"
                );
            }
            _ => panic!("expected Reasoning"),
        }
    }
}

#[tokio::test]
async fn reasoning_config_serialization() {
    let config = ReasoningConfig {
        effort: Some("high".to_owned()),
        generate_summary: Some(true),
        summary: None,
    };
    let json = serde_json::to_value(&config).unwrap();
    assert_eq!(json["effort"], "high");
    assert_eq!(json["generate_summary"], true);
    assert!(json.get("summary").is_none());
}

#[tokio::test]
async fn multi_turn_reasoning_chain_serialization() {
    // Simulate a 3-turn conversation with encrypted reasoning continuity.
    let blobs = [
        "ENC:turn1-blob-aabbcc",
        "ENC:turn2-blob-ddeeff",
        "ENC:turn3-blob-112233",
    ];

    // Build a multi-turn input replaying all three reasoning blobs.
    let mut items: Vec<InputItem> = Vec::new();
    for (i, blob) in blobs.iter().enumerate() {
        items.push(InputItem::Reasoning {
            r#type: "reasoning".to_owned(),
            id: Some(format!("rs_turn_{}", i + 1)),
            encrypted_content: blob.to_string(),
        });
    }
    items.push(InputItem::Message(
        grokrs_api::types::message::InputMessage {
            role: grokrs_api::types::common::Role::User,
            content: MessageContent::Text("Continue the analysis.".to_owned()),
            name: None,
        },
    ));

    let input = ResponseInput::Items(items);
    let json = serde_json::to_value(&input).unwrap();
    let arr = json.as_array().unwrap();
    assert_eq!(arr.len(), 4);

    // Verify each reasoning replay.
    for (i, blob) in blobs.iter().enumerate() {
        assert_eq!(arr[i]["type"], "reasoning");
        assert_eq!(arr[i]["encrypted_content"], *blob);
        assert_eq!(arr[i]["id"], format!("rs_turn_{}", i + 1));
    }

    // Verify the user message.
    assert_eq!(arr[3]["role"], "user");
}

#[tokio::test]
async fn wiremock_full_two_turn_encrypted_reasoning() {
    // Full two-turn flow against a wiremock server.
    let mock_server = MockServer::start().await;
    let blob = "ENC:opaque-reasoning-continuation-blob-xyz";

    // Turn 1: returns reasoning with encrypted_content.
    Mock::given(method("POST"))
        .and(path("/v1/responses"))
        .respond_with(
            ResponseTemplate::new(200).set_body_json(mock_reasoning_response(
                blob,
                "Deep thought...",
                "42.",
            )),
        )
        .up_to_n_times(1)
        .mount(&mock_server)
        .await;

    let client = reqwest::Client::new();

    // First turn.
    let resp1: ResponseObject = client
        .post(format!("{}/v1/responses", mock_server.uri()))
        .json(&json!({
            "model": "grok-3",
            "input": "Analyze this problem",
            "reasoning": {"effort": "high"},
            "include": ["reasoning.encrypted_content"]
        }))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();

    let (reasoning_id, enc) = match &resp1.output[0] {
        OutputItem::Reasoning {
            id,
            encrypted_content,
            ..
        } => (id.clone(), encrypted_content.clone().unwrap()),
        _ => panic!("expected Reasoning"),
    };

    // Mount turn 2: continuation response.
    Mock::given(method("POST"))
        .and(path("/v1/responses"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_json(mock_continuation_response("Elaborated answer.")),
        )
        .expect(1)
        .mount(&mock_server)
        .await;

    // Build second-turn request body with the encrypted reasoning replay.
    let turn2_input = json!({
        "model": "grok-3",
        "input": [
            {
                "type": "reasoning",
                "id": reasoning_id,
                "encrypted_content": enc
            },
            {
                "role": "user",
                "content": [{"type": "text", "text": "Elaborate please."}]
            }
        ]
    });

    let resp2: ResponseObject = client
        .post(format!("{}/v1/responses", mock_server.uri()))
        .json(&turn2_input)
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();

    assert_eq!(resp2.id, "resp_test_002");
    match &resp2.output[0] {
        OutputItem::Message { content, .. } => match &content[0] {
            ContentBlock::Text { text } => assert_eq!(text, "Elaborated answer."),
            other => panic!("expected Text, got: {other:?}"),
        },
        other => panic!("expected Message, got: {other:?}"),
    }
}
