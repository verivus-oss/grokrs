use std::sync::Arc;

use futures::StreamExt;
use reqwest::Method;
use wiremock::matchers::{header, method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

use grokrs_api::transport::auth::ApiKeySecret;
use grokrs_api::transport::client::{HttpClient, HttpClientConfig};
use grokrs_api::transport::error::TransportError;
use grokrs_api::transport::policy_gate::{AllowAllGate, PolicyDecision, PolicyGate};
use grokrs_api::transport::retry::RetryConfig;

/// Helper to build an `HttpClient` pointed at the given mock server.
///
/// Uses `AllowAllGate` explicitly because the default is deny-all.
fn client_for_mock(server: &MockServer) -> HttpClient {
    let config = HttpClientConfig {
        base_url: server.uri(),
        retry: RetryConfig {
            max_retries: 2,
            base_delay_ms: 10, // very short for tests
            max_delay_ms: 100,
        },
        ..Default::default()
    };
    HttpClient::new(
        config,
        ApiKeySecret::new("test-bearer-token"),
        Some(Arc::new(AllowAllGate)),
    )
    .unwrap()
}

/// Helper to build an `HttpClient` with a policy gate.
fn client_with_gate(server: &MockServer, gate: Arc<dyn PolicyGate>) -> HttpClient {
    let config = HttpClientConfig {
        base_url: server.uri(),
        retry: RetryConfig {
            max_retries: 0,
            base_delay_ms: 10,
            max_delay_ms: 100,
        },
        ..Default::default()
    };
    HttpClient::new(config, ApiKeySecret::new("test-key"), Some(gate)).unwrap()
}

// ---------------------------------------------------------------------------
// Test: Authorization Bearer header is injected
// ---------------------------------------------------------------------------
#[tokio::test]
async fn bearer_token_is_sent_in_authorization_header() {
    let server = MockServer::start().await;

    Mock::given(method("POST"))
        .and(path("/v1/test"))
        .and(header("Authorization", "Bearer test-bearer-token"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({"ok": true})))
        .expect(1)
        .mount(&server)
        .await;

    let client = client_for_mock(&server);
    let body = serde_json::json!({"prompt": "hello"});
    let resp: serde_json::Value = client
        .send_json(Method::POST, "/v1/test", &body)
        .await
        .unwrap();
    assert_eq!(resp["ok"], true);
}

// ---------------------------------------------------------------------------
// Test: Retry on 429 status
// ---------------------------------------------------------------------------
#[tokio::test]
async fn retries_on_429_then_succeeds() {
    let server = MockServer::start().await;

    // First request: 429
    Mock::given(method("POST"))
        .and(path("/v1/retry-test"))
        .respond_with(ResponseTemplate::new(429).set_body_json(serde_json::json!({
            "error": {"message": "rate limited"}
        })))
        .up_to_n_times(1)
        .expect(1)
        .mount(&server)
        .await;

    // Second request: 200
    Mock::given(method("POST"))
        .and(path("/v1/retry-test"))
        .respond_with(
            ResponseTemplate::new(200).set_body_json(serde_json::json!({"result": "success"})),
        )
        .expect(1)
        .mount(&server)
        .await;

    let client = client_for_mock(&server);
    let body = serde_json::json!({"input": "test"});
    let resp: serde_json::Value = client
        .send_json(Method::POST, "/v1/retry-test", &body)
        .await
        .unwrap();
    assert_eq!(resp["result"], "success");
}

// ---------------------------------------------------------------------------
// Test: Retry on 503 status
// ---------------------------------------------------------------------------
#[tokio::test]
async fn retries_on_503_then_succeeds() {
    let server = MockServer::start().await;

    Mock::given(method("POST"))
        .and(path("/v1/retry-503"))
        .respond_with(ResponseTemplate::new(503).set_body_json(serde_json::json!({
            "error": {"message": "service unavailable"}
        })))
        .up_to_n_times(1)
        .expect(1)
        .mount(&server)
        .await;

    Mock::given(method("POST"))
        .and(path("/v1/retry-503"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({"ok": true})))
        .expect(1)
        .mount(&server)
        .await;

    let client = client_for_mock(&server);
    let body = serde_json::json!({});
    let resp: serde_json::Value = client
        .send_json(Method::POST, "/v1/retry-503", &body)
        .await
        .unwrap();
    assert_eq!(resp["ok"], true);
}

// ---------------------------------------------------------------------------
// Test: Non-2xx parsed into ApiError with x-request-id
// ---------------------------------------------------------------------------
#[tokio::test]
async fn non_2xx_parsed_into_api_error_with_request_id() {
    let server = MockServer::start().await;

    Mock::given(method("POST"))
        .and(path("/v1/error-test"))
        .respond_with(
            ResponseTemplate::new(400)
                .set_body_json(serde_json::json!({
                    "error": {
                        "message": "Invalid model",
                        "type": "invalid_request_error",
                        "code": "model_not_found"
                    }
                }))
                .append_header("x-request-id", "req-test-12345"),
        )
        .expect(1)
        .mount(&server)
        .await;

    let client = client_for_mock(&server);
    let body = serde_json::json!({"model": "nonexistent"});
    let err = client
        .send_json::<_, serde_json::Value>(Method::POST, "/v1/error-test", &body)
        .await
        .unwrap_err();

    match err {
        TransportError::Api(api_err) => {
            assert_eq!(api_err.status_code, 400);
            assert_eq!(api_err.message, "Invalid model");
            assert_eq!(api_err.error_type.as_deref(), Some("invalid_request_error"));
            assert_eq!(api_err.code.as_deref(), Some("model_not_found"));
            assert_eq!(api_err.request_id.as_deref(), Some("req-test-12345"));
        }
        other => panic!("expected Api error, got: {other}"),
    }
}

// ---------------------------------------------------------------------------
// Test: SSE streaming through the client
// ---------------------------------------------------------------------------
#[tokio::test]
async fn sse_streaming_yields_data_lines() {
    let server = MockServer::start().await;

    let sse_body = "data: {\"chunk\":1}\n\ndata: {\"chunk\":2}\n\ndata: [DONE]\n\n";

    Mock::given(method("POST"))
        .and(path("/v1/stream"))
        .respond_with(ResponseTemplate::new(200).set_body_raw(sse_body, "text/event-stream"))
        .expect(1)
        .mount(&server)
        .await;

    let client = client_for_mock(&server);
    let body = serde_json::json!({"stream": true});
    let mut stream = client
        .send_sse(Method::POST, "/v1/stream", &body)
        .await
        .unwrap();

    let first = stream.next().await.unwrap().unwrap();
    let v1: serde_json::Value = serde_json::from_str(&first).unwrap();
    assert_eq!(v1["chunk"], 1);

    let second = stream.next().await.unwrap().unwrap();
    let v2: serde_json::Value = serde_json::from_str(&second).unwrap();
    assert_eq!(v2["chunk"], 2);

    // Stream should end after [DONE]
    assert!(stream.next().await.is_none());
}

// ---------------------------------------------------------------------------
// Test: SSE retry on 429 for the initial connection
// ---------------------------------------------------------------------------
#[tokio::test]
async fn sse_retries_on_429_initial_connection() {
    let server = MockServer::start().await;

    // First request: 429
    Mock::given(method("POST"))
        .and(path("/v1/sse-retry"))
        .respond_with(ResponseTemplate::new(429).set_body_json(serde_json::json!({
            "error": {"message": "rate limited"}
        })))
        .up_to_n_times(1)
        .expect(1)
        .mount(&server)
        .await;

    // Second request: 200 with SSE
    Mock::given(method("POST"))
        .and(path("/v1/sse-retry"))
        .respond_with(ResponseTemplate::new(200).set_body_raw(
            "data: {\"ok\":true}\n\ndata: [DONE]\n\n",
            "text/event-stream",
        ))
        .expect(1)
        .mount(&server)
        .await;

    let client = client_for_mock(&server);
    let body = serde_json::json!({});
    let mut stream = client
        .send_sse(Method::POST, "/v1/sse-retry", &body)
        .await
        .unwrap();

    let first = stream.next().await.unwrap().unwrap();
    let v: serde_json::Value = serde_json::from_str(&first).unwrap();
    assert_eq!(v["ok"], true);
}

// ---------------------------------------------------------------------------
// Test: Policy gate deny blocks request
// ---------------------------------------------------------------------------
#[tokio::test]
async fn policy_gate_deny_blocks_send_json() {
    let server = MockServer::start().await;

    struct DenyGate;
    impl PolicyGate for DenyGate {
        fn evaluate_network(&self, _host: &str) -> PolicyDecision {
            PolicyDecision::Deny {
                reason: "blocked by policy".into(),
            }
        }
    }

    let client = client_with_gate(&server, Arc::new(DenyGate));
    let body = serde_json::json!({"test": true});
    let err = client
        .send_json::<_, serde_json::Value>(Method::POST, "/v1/test", &body)
        .await
        .unwrap_err();

    match err {
        TransportError::PolicyDenied { reason, .. } => {
            assert_eq!(reason, "blocked by policy");
        }
        other => panic!("expected PolicyDenied, got: {other}"),
    }
}

// ---------------------------------------------------------------------------
// Test: Policy gate allow permits request
// ---------------------------------------------------------------------------
#[tokio::test]
async fn policy_gate_allow_permits_request() {
    let server = MockServer::start().await;

    Mock::given(method("POST"))
        .and(path("/v1/allowed"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({"ok": true})))
        .expect(1)
        .mount(&server)
        .await;

    let client = client_with_gate(&server, Arc::new(AllowAllGate));
    let body = serde_json::json!({});
    let resp: serde_json::Value = client
        .send_json(Method::POST, "/v1/allowed", &body)
        .await
        .unwrap();
    assert_eq!(resp["ok"], true);
}

// ---------------------------------------------------------------------------
// Test: Policy gate deny blocks SSE
// ---------------------------------------------------------------------------
#[tokio::test]
async fn policy_gate_deny_blocks_send_sse() {
    let server = MockServer::start().await;

    struct DenyGate;
    impl PolicyGate for DenyGate {
        fn evaluate_network(&self, _host: &str) -> PolicyDecision {
            PolicyDecision::Deny {
                reason: "sse blocked".into(),
            }
        }
    }

    let client = client_with_gate(&server, Arc::new(DenyGate));
    let body = serde_json::json!({});
    let result = client.send_sse(Method::POST, "/v1/stream", &body).await;

    match result {
        Err(err) => match err {
            TransportError::PolicyDenied { reason, .. } => {
                assert_eq!(reason, "sse blocked");
            }
            other => panic!("expected PolicyDenied, got: {other}"),
        },
        Ok(_) => panic!("expected error, got Ok"),
    }
}

// ---------------------------------------------------------------------------
// Test: Non-2xx error without valid JSON body
// ---------------------------------------------------------------------------
#[tokio::test]
async fn non_2xx_with_non_json_body_produces_fallback_error() {
    let server = MockServer::start().await;

    Mock::given(method("POST"))
        .and(path("/v1/bad-body"))
        .respond_with(
            ResponseTemplate::new(502)
                .set_body_string("Bad Gateway")
                .append_header("x-request-id", "req-bad-gw"),
        )
        .expect(1)
        .mount(&server)
        .await;

    let client = client_for_mock(&server);
    let body = serde_json::json!({});
    let err = client
        .send_json::<_, serde_json::Value>(Method::POST, "/v1/bad-body", &body)
        .await
        .unwrap_err();

    match err {
        TransportError::Api(api_err) => {
            assert_eq!(api_err.status_code, 502);
            assert!(api_err.message.contains("Bad Gateway"));
            assert_eq!(api_err.request_id.as_deref(), Some("req-bad-gw"));
        }
        other => panic!("expected Api error, got: {other}"),
    }
}

// ---------------------------------------------------------------------------
// Test: Exhausted retries produces error
// ---------------------------------------------------------------------------
#[tokio::test]
async fn exhausted_retries_returns_error() {
    let server = MockServer::start().await;

    // Always returns 429
    Mock::given(method("POST"))
        .and(path("/v1/always-429"))
        .respond_with(
            ResponseTemplate::new(429)
                .set_body_json(serde_json::json!({
                    "error": {"message": "rate limited forever"}
                }))
                .append_header("x-request-id", "req-rate-limit"),
        )
        .expect(3) // initial + 2 retries
        .mount(&server)
        .await;

    let client = client_for_mock(&server);
    let body = serde_json::json!({});
    let err = client
        .send_json::<_, serde_json::Value>(Method::POST, "/v1/always-429", &body)
        .await
        .unwrap_err();

    match err {
        TransportError::Api(api_err) => {
            assert_eq!(api_err.status_code, 429);
            assert_eq!(api_err.message, "rate limited forever");
            assert_eq!(api_err.request_id.as_deref(), Some("req-rate-limit"));
        }
        other => panic!("expected Api error, got: {other}"),
    }
}

// ===========================================================================
// Issue 5: Models endpoint wiremock integration tests
// ===========================================================================

use grokrs_api::endpoints::models::ModelsClient;
use grokrs_api::types::model::{LanguageModelList, ModelList};

/// Helper to build a `ModelsClient` backed by a mock server.
fn models_client_for_mock(server: &MockServer) -> (Arc<HttpClient>, ModelsClient) {
    let http = Arc::new(client_for_mock(server));
    let models = ModelsClient::new(Arc::clone(&http));
    (http, models)
}

#[tokio::test]
async fn models_list_models_deserializes_correctly() {
    let server = MockServer::start().await;

    Mock::given(method("GET"))
        .and(path("/v1/models"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "object": "list",
            "data": [
                {
                    "id": "grok-4",
                    "created": 1700000000,
                    "owned_by": "xai",
                    "object": "model"
                },
                {
                    "id": "grok-4-mini",
                    "created": 1700000001,
                    "owned_by": "xai"
                }
            ]
        })))
        .expect(1)
        .mount(&server)
        .await;

    let (_http, client) = models_client_for_mock(&server);
    let list: ModelList = client.list_models().await.unwrap();
    assert_eq!(list.object.as_deref(), Some("list"));
    assert_eq!(list.data.len(), 2);
    assert_eq!(list.data[0].id, "grok-4");
    assert_eq!(list.data[0].owned_by, "xai");
    assert_eq!(list.data[1].id, "grok-4-mini");
}

#[tokio::test]
async fn models_list_language_models_deserializes_correctly() {
    let server = MockServer::start().await;

    Mock::given(method("GET"))
        .and(path("/v1/language-models"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "models": [
                {
                    "id": "grok-4",
                    "created": 1700000000,
                    "owned_by": "xai",
                    "aliases": ["grok-latest"],
                    "input_modalities": ["text", "image"],
                    "output_modalities": ["text"],
                    "prompt_text_token_price": 500,
                    "completion_text_token_price": 1500
                }
            ]
        })))
        .expect(1)
        .mount(&server)
        .await;

    let (_http, client) = models_client_for_mock(&server);
    let list: LanguageModelList = client.list_language_models().await.unwrap();
    assert_eq!(list.models.len(), 1);
    assert_eq!(list.models[0].id, "grok-4");
    assert_eq!(list.models[0].aliases, vec!["grok-latest"]);
    assert_eq!(list.models[0].input_modalities, vec!["text", "image"]);
    assert_eq!(list.models[0].prompt_text_token_price, Some(500));
    assert_eq!(list.models[0].completion_text_token_price, Some(1500));
}

#[tokio::test]
async fn models_get_model_with_url_encoded_id() {
    let server = MockServer::start().await;

    // Model ID with a slash that requires percent-encoding
    Mock::given(method("GET"))
        .and(path("/v1/models/org%2Fgrok-4"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "id": "org/grok-4",
            "created": 1700000000,
            "owned_by": "xai",
            "object": "model"
        })))
        .expect(1)
        .mount(&server)
        .await;

    let (_http, client) = models_client_for_mock(&server);
    let model = client.get_model("org/grok-4").await.unwrap();
    assert_eq!(model.id, "org/grok-4");
    assert_eq!(model.owned_by, "xai");
}

#[tokio::test]
async fn models_get_model_returns_404_error() {
    let server = MockServer::start().await;

    Mock::given(method("GET"))
        .and(path("/v1/models/nonexistent-model"))
        .respond_with(
            ResponseTemplate::new(404)
                .set_body_json(serde_json::json!({
                    "error": {
                        "message": "Model not found",
                        "type": "not_found_error",
                        "code": "model_not_found"
                    }
                }))
                .append_header("x-request-id", "req-404-test"),
        )
        .expect(1)
        .mount(&server)
        .await;

    let (_http, client) = models_client_for_mock(&server);
    let err = client.get_model("nonexistent-model").await.unwrap_err();

    match err {
        TransportError::Api(api_err) => {
            assert_eq!(api_err.status_code, 404);
            assert_eq!(api_err.message, "Model not found");
            assert_eq!(api_err.error_type.as_deref(), Some("not_found_error"));
            assert_eq!(api_err.code.as_deref(), Some("model_not_found"));
            assert_eq!(api_err.request_id.as_deref(), Some("req-404-test"));
        }
        other => panic!("expected Api error, got: {other}"),
    }
}

#[tokio::test]
async fn models_all_methods_use_get() {
    let server = MockServer::start().await;

    // Mount mocks that only match GET — if any method uses POST/PUT/etc., it
    // will not match and wiremock will return 404.
    Mock::given(method("GET"))
        .and(path("/v1/models"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "object": "list",
            "data": []
        })))
        .expect(1)
        .mount(&server)
        .await;

    Mock::given(method("GET"))
        .and(path("/v1/language-models"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "models": []
        })))
        .expect(1)
        .mount(&server)
        .await;

    Mock::given(method("GET"))
        .and(path("/v1/models/grok-4"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "id": "grok-4",
            "created": 0,
            "owned_by": "xai"
        })))
        .expect(1)
        .mount(&server)
        .await;

    let (_http, client) = models_client_for_mock(&server);

    client.list_models().await.unwrap();
    client.list_language_models().await.unwrap();
    client.get_model("grok-4").await.unwrap();
    // If any of the above used a wrong HTTP method, wiremock would return 404
    // and the test would fail.
}

// ===========================================================================
// Issue 6: Files, Tokenizer, API Key endpoint wiremock integration tests
// ===========================================================================

use grokrs_api::endpoints::api_key::ApiKeyClient;
use grokrs_api::endpoints::files::FilesClient;
use grokrs_api::endpoints::tokenize::TokenizeClient;

// ---------------------------------------------------------------------------
// FilesClient integration tests
// ---------------------------------------------------------------------------

#[tokio::test]
async fn files_list_deserializes_correctly() {
    let server = MockServer::start().await;

    Mock::given(method("GET"))
        .and(path("/v1/files"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "data": [
                {
                    "id": "file-abc123",
                    "object": "file",
                    "bytes": 12345,
                    "created_at": 1700000000,
                    "filename": "data.jsonl",
                    "purpose": "assistants"
                },
                {
                    "id": "file-def456",
                    "object": "file",
                    "bytes": 6789,
                    "filename": "other.csv"
                }
            ],
            "has_more": false
        })))
        .expect(1)
        .mount(&server)
        .await;

    let http = client_for_mock(&server);
    let client = FilesClient::new(&http);
    let list = client.list(None).await.unwrap();
    assert_eq!(list.data.len(), 2);
    assert_eq!(list.data[0].id, "file-abc123");
    assert_eq!(list.data[0].filename.as_deref(), Some("data.jsonl"));
    assert_eq!(list.data[0].bytes, Some(12345));
    assert_eq!(list.data[1].id, "file-def456");
    assert_eq!(list.has_more, Some(false));
}

#[tokio::test]
async fn files_get_deserializes_correctly() {
    let server = MockServer::start().await;

    Mock::given(method("GET"))
        .and(path("/v1/files/file-abc123"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "id": "file-abc123",
            "object": "file",
            "bytes": 12345,
            "created_at": 1700000000,
            "filename": "data.jsonl",
            "purpose": "assistants"
        })))
        .expect(1)
        .mount(&server)
        .await;

    let http = client_for_mock(&server);
    let client = FilesClient::new(&http);
    let file = client.get("file-abc123").await.unwrap();
    assert_eq!(file.id, "file-abc123");
    assert_eq!(file.purpose.as_deref(), Some("assistants"));
    assert_eq!(file.bytes, Some(12345));
}

#[tokio::test]
async fn files_download_returns_raw_bytes() {
    let server = MockServer::start().await;

    let raw_content = b"binary file content here\x00\xFF\xFE";

    Mock::given(method("POST"))
        .and(path("/v1/files:download"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_raw(raw_content.to_vec(), "application/octet-stream"),
        )
        .expect(1)
        .mount(&server)
        .await;

    let http = client_for_mock(&server);
    let client = FilesClient::new(&http);
    let bytes = client.download("file-abc123").await.unwrap();
    assert_eq!(bytes, raw_content.to_vec());
}

#[tokio::test]
async fn files_list_returns_error_on_500() {
    let server = MockServer::start().await;

    Mock::given(method("GET"))
        .and(path("/v1/files"))
        .respond_with(
            ResponseTemplate::new(500)
                .set_body_json(serde_json::json!({
                    "error": {
                        "message": "Internal server error",
                        "type": "server_error"
                    }
                }))
                .append_header("x-request-id", "req-files-500"),
        )
        .expect(1)
        .mount(&server)
        .await;

    let http = client_for_mock(&server);
    let client = FilesClient::new(&http);
    let err = client.list(None).await.unwrap_err();

    match err {
        TransportError::Api(api_err) => {
            assert_eq!(api_err.status_code, 500);
            assert_eq!(api_err.message, "Internal server error");
            assert_eq!(api_err.request_id.as_deref(), Some("req-files-500"));
        }
        other => panic!("expected Api error, got: {other}"),
    }
}

// ---------------------------------------------------------------------------
// TokenizeClient integration tests
// ---------------------------------------------------------------------------

#[tokio::test]
async fn tokenize_sends_correct_body_and_deserializes_response() {
    let server = MockServer::start().await;

    Mock::given(method("POST"))
        .and(path("/v1/tokenize-text"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "token_ids": [
                {
                    "token_id": 9906,
                    "string_token": "Hello",
                    "token_bytes": [72, 101, 108, 108, 111]
                },
                {
                    "token_id": 11,
                    "string_token": ",",
                    "token_bytes": [44]
                }
            ]
        })))
        .expect(1)
        .mount(&server)
        .await;

    let http = client_for_mock(&server);
    let client = TokenizeClient::new(&http);
    let resp = client.tokenize("Hello,", "grok-4").await.unwrap();

    assert_eq!(resp.token_ids.len(), 2);
    assert_eq!(resp.token_ids[0].token_id, 9906);
    assert_eq!(resp.token_ids[0].string_token, "Hello");
    assert_eq!(resp.token_ids[0].token_bytes, vec![72, 101, 108, 108, 111]);
    assert_eq!(resp.token_ids[1].token_id, 11);
    assert_eq!(resp.token_ids[1].string_token, ",");

    // Verify the request body was correct
    let received = server.received_requests().await.unwrap();
    assert_eq!(received.len(), 1);
    let body: serde_json::Value = serde_json::from_slice(&received[0].body).unwrap();
    assert_eq!(body["text"], "Hello,");
    assert_eq!(body["model"], "grok-4");
}

#[tokio::test]
async fn tokenize_returns_error_on_400() {
    let server = MockServer::start().await;

    Mock::given(method("POST"))
        .and(path("/v1/tokenize-text"))
        .respond_with(ResponseTemplate::new(400).set_body_json(serde_json::json!({
            "error": {
                "message": "Invalid model for tokenization",
                "type": "invalid_request_error"
            }
        })))
        .expect(1)
        .mount(&server)
        .await;

    let http = client_for_mock(&server);
    let client = TokenizeClient::new(&http);
    let err = client.tokenize("test", "bad-model").await.unwrap_err();

    match err {
        TransportError::Api(api_err) => {
            assert_eq!(api_err.status_code, 400);
            assert!(api_err.message.contains("Invalid model"));
        }
        other => panic!("expected Api error, got: {other}"),
    }
}

// ---------------------------------------------------------------------------
// ApiKeyClient integration tests
// ---------------------------------------------------------------------------

#[tokio::test]
async fn api_key_info_deserializes_correctly() {
    let server = MockServer::start().await;

    Mock::given(method("GET"))
        .and(path("/v1/api-key"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "name": "production-key",
            "status": "active",
            "acls": ["chat:completions", "files:read"],
            "team_id": "team-prod-001",
            "blocked": false,
            "disabled": false
        })))
        .expect(1)
        .mount(&server)
        .await;

    let http = client_for_mock(&server);
    let client = ApiKeyClient::new(&http);
    let info = client.info().await.unwrap();

    assert_eq!(info.name.as_deref(), Some("production-key"));
    assert_eq!(info.status.as_deref(), Some("active"));
    let acls = info.acls.unwrap();
    assert_eq!(acls.len(), 2);
    assert_eq!(acls[0], "chat:completions");
    assert_eq!(acls[1], "files:read");
    assert_eq!(info.team_id.as_deref(), Some("team-prod-001"));
    assert_eq!(info.blocked, Some(false));
    assert_eq!(info.disabled, Some(false));
}

#[tokio::test]
async fn api_key_info_returns_error_on_401() {
    let server = MockServer::start().await;

    Mock::given(method("GET"))
        .and(path("/v1/api-key"))
        .respond_with(
            ResponseTemplate::new(401)
                .set_body_json(serde_json::json!({
                    "error": {
                        "message": "Invalid API key",
                        "type": "authentication_error"
                    }
                }))
                .append_header("x-request-id", "req-auth-fail"),
        )
        .expect(1)
        .mount(&server)
        .await;

    let http = client_for_mock(&server);
    let client = ApiKeyClient::new(&http);
    let err = client.info().await.unwrap_err();

    match err {
        TransportError::Api(api_err) => {
            assert_eq!(api_err.status_code, 401);
            assert_eq!(api_err.message, "Invalid API key");
            assert_eq!(api_err.error_type.as_deref(), Some("authentication_error"));
            assert_eq!(api_err.request_id.as_deref(), Some("req-auth-fail"));
        }
        other => panic!("expected Api error, got: {other}"),
    }
}

#[tokio::test]
async fn api_key_info_uses_get_method() {
    let server = MockServer::start().await;

    // Only match GET — if ApiKeyClient used POST it would not match.
    Mock::given(method("GET"))
        .and(path("/v1/api-key"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "name": "test-key",
            "status": "active"
        })))
        .expect(1)
        .mount(&server)
        .await;

    let http = client_for_mock(&server);
    let client = ApiKeyClient::new(&http);
    let info = client.info().await.unwrap();
    assert_eq!(info.name.as_deref(), Some("test-key"));
}

// ---------------------------------------------------------------------------
// Deny-by-default verification test
// ---------------------------------------------------------------------------

#[tokio::test]
async fn http_client_no_gate_denies_requests() {
    let server = MockServer::start().await;

    // Mount a mock that should never be hit
    Mock::given(method("GET"))
        .and(path("/v1/models"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "object": "list",
            "data": []
        })))
        .expect(0) // should never be called
        .mount(&server)
        .await;

    let config = HttpClientConfig {
        base_url: server.uri(),
        ..Default::default()
    };
    // Explicitly pass None — should use DenyAllGate
    let http = Arc::new(HttpClient::new(config, ApiKeySecret::new("key"), None).unwrap());
    let client = ModelsClient::new(http);
    let err = client.list_models().await.unwrap_err();

    match err {
        TransportError::PolicyDenied { reason, .. } => {
            assert!(
                reason.contains("denied"),
                "reason should mention denied: {reason}"
            );
        }
        other => panic!("expected PolicyDenied (deny-by-default), got: {other}"),
    }
}
