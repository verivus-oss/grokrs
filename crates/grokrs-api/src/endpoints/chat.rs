//! Chat Completions API endpoint client (legacy).
//!
//! This module provides `ChatClient`, which wraps `HttpClient` to send
//! requests to the xAI `/v1/chat/completions` endpoint. It is intentionally
//! **deprecated** — new integrations should use the Responses API endpoint.

#![allow(deprecated)]

use reqwest::Method;

use crate::transport::client::HttpClient;
use crate::transport::error::TransportError;
use crate::types::chat::{ChatCompletion, ChatCompletionRequest};

use super::util::encode_path_segment;

/// The path for the Chat Completions endpoint.
const CHAT_COMPLETIONS_PATH: &str = "/v1/chat/completions";

/// The path prefix for deferred completion polling.
const DEFERRED_COMPLETION_PATH_PREFIX: &str = "/v1/chat/deferred-completion/";

/// A client for the legacy xAI Chat Completions API.
///
/// Holds a reference to a shared `HttpClient` and provides typed methods
/// for creating chat completions and polling deferred results.
///
/// # Deprecation
///
/// The Chat Completions API is a legacy interface. Prefer `ResponsesClient`
/// for all new integrations.
#[deprecated(note = "Use ResponsesClient instead")]
pub struct ChatClient<'a> {
    http: &'a HttpClient,
}

#[allow(deprecated)]
impl<'a> ChatClient<'a> {
    /// Create a new `ChatClient` backed by the given `HttpClient`.
    #[must_use]
    pub fn new(http: &'a HttpClient) -> Self {
        Self { http }
    }

    /// Send a chat completion request and return the completed response.
    ///
    /// Issues `POST /v1/chat/completions` with the given request body.
    /// For streaming responses, set `stream: Some(true)` on the request
    /// and use the SSE transport methods on `HttpClient` directly.
    pub async fn create(
        &self,
        request: &ChatCompletionRequest,
    ) -> Result<ChatCompletion, TransportError> {
        self.http
            .send_json(Method::POST, CHAT_COMPLETIONS_PATH, request)
            .await
    }

    /// Poll for the result of a deferred (asynchronous) chat completion.
    ///
    /// Issues `GET /v1/chat/deferred-completion/{request_id}`.
    ///
    /// - A **202** response means the completion is still processing; returns
    ///   `Ok(None)`.
    /// - A **200** response means the completion is finished; the body is a
    ///   plain `ChatCompletion` and the method returns `Ok(Some(completion))`.
    pub async fn poll_deferred(
        &self,
        request_id: &str,
    ) -> Result<Option<ChatCompletion>, TransportError> {
        let path = format!(
            "{}{}",
            DEFERRED_COMPLETION_PATH_PREFIX,
            encode_path_segment(request_id)
        );
        let (status, body) = self
            .http
            .send_no_body_with_status(Method::GET, &path, &[200, 202])
            .await?;

        match status {
            202 => Ok(None),
            _ => {
                // 200 or any other 2xx — deserialize as ChatCompletion.
                let completion: ChatCompletion =
                    serde_json::from_slice(&body).map_err(|e| TransportError::Deserialization {
                        message: format!("failed to deserialize deferred completion: {e}"),
                    })?;
                Ok(Some(completion))
            }
        }
    }
}

#[cfg(test)]
#[allow(deprecated)]
mod tests {
    use super::*;
    use crate::transport::auth::ApiKeySecret;
    use crate::transport::client::HttpClientConfig;
    use crate::types::chat::ChatCompletionBuilder;
    use crate::types::common::Role;
    use crate::types::message::Message;
    use wiremock::matchers::{body_partial_json, header, method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    use std::sync::Arc;

    use crate::transport::policy_gate::AllowAllGate;

    #[allow(unused_imports)]
    use crate::types::chat::DeferredCompletion;

    /// Create an `HttpClient` pointed at the mock server.
    fn mock_http_client(base_url: &str) -> HttpClient {
        let config = HttpClientConfig {
            base_url: base_url.to_string(),
            ..Default::default()
        };
        HttpClient::new(
            config,
            ApiKeySecret::new("test-key"),
            Some(Arc::new(AllowAllGate)),
        )
        .unwrap()
    }

    #[tokio::test]
    async fn create_sends_post_and_deserializes_response() {
        let server = MockServer::start().await;
        let response_body = serde_json::json!({
            "id": "chatcmpl-mock-1",
            "object": "chat.completion",
            "created": 1700000000,
            "model": "grok-4",
            "choices": [
                {
                    "index": 0,
                    "message": {
                        "role": "assistant",
                        "content": "Hello from mock!"
                    },
                    "finish_reason": "stop"
                }
            ],
            "usage": {
                "prompt_tokens": 5,
                "completion_tokens": 4,
                "total_tokens": 9
            }
        });

        Mock::given(method("POST"))
            .and(path("/v1/chat/completions"))
            .and(header("authorization", "Bearer test-key"))
            .and(body_partial_json(serde_json::json!({
                "model": "grok-4",
                "messages": [
                    {"role": "user", "content": "Hello"}
                ]
            })))
            .respond_with(ResponseTemplate::new(200).set_body_json(&response_body))
            .expect(1)
            .mount(&server)
            .await;

        let http = mock_http_client(&server.uri());
        let client = ChatClient::new(&http);

        let request =
            ChatCompletionBuilder::new("grok-4", vec![Message::text(Role::User, "Hello")]).build();

        let completion = client.create(&request).await.unwrap();
        assert_eq!(completion.id, "chatcmpl-mock-1");
        assert_eq!(completion.choices.len(), 1);
        assert_eq!(
            completion.choices[0].message.content.as_deref(),
            Some("Hello from mock!")
        );
    }

    #[tokio::test]
    async fn create_with_deferred_sends_deferred_field() {
        let server = MockServer::start().await;
        let response_body = serde_json::json!({
            "id": "chatcmpl-deferred-1",
            "choices": [],
            "request_id": "req_123"
        });

        Mock::given(method("POST"))
            .and(path("/v1/chat/completions"))
            .and(body_partial_json(serde_json::json!({
                "deferred": true
            })))
            .respond_with(ResponseTemplate::new(200).set_body_json(&response_body))
            .expect(1)
            .mount(&server)
            .await;

        let http = mock_http_client(&server.uri());
        let client = ChatClient::new(&http);

        let request =
            ChatCompletionBuilder::new("grok-4", vec![Message::text(Role::User, "Long task")])
                .deferred(true)
                .build();

        let completion = client.create(&request).await.unwrap();
        assert_eq!(completion.id, "chatcmpl-deferred-1");
    }

    #[tokio::test]
    async fn poll_deferred_returns_none_on_202() {
        let server = MockServer::start().await;

        // xAI returns 202 with an empty body while the completion is processing.
        Mock::given(method("GET"))
            .and(path("/v1/chat/deferred-completion/req_abc"))
            .and(header("authorization", "Bearer test-key"))
            .respond_with(ResponseTemplate::new(202))
            .expect(1)
            .mount(&server)
            .await;

        let http = mock_http_client(&server.uri());
        let client = ChatClient::new(&http);

        let result = client.poll_deferred("req_abc").await.unwrap();
        assert!(
            result.is_none(),
            "202 should return None (still processing)"
        );
    }

    #[tokio::test]
    async fn poll_deferred_returns_completion_on_200() {
        let server = MockServer::start().await;

        // xAI returns 200 with a plain ChatCompletion body when ready.
        let response_body = serde_json::json!({
            "id": "chatcmpl-result-1",
            "choices": [
                {
                    "index": 0,
                    "message": {
                        "role": "assistant",
                        "content": "Deferred result!"
                    },
                    "finish_reason": "stop"
                }
            ]
        });

        Mock::given(method("GET"))
            .and(path("/v1/chat/deferred-completion/req_done"))
            .respond_with(ResponseTemplate::new(200).set_body_json(&response_body))
            .expect(1)
            .mount(&server)
            .await;

        let http = mock_http_client(&server.uri());
        let client = ChatClient::new(&http);

        let result = client.poll_deferred("req_done").await.unwrap();
        assert!(result.is_some(), "200 should return Some(ChatCompletion)");
        let completion = result.unwrap();
        assert_eq!(completion.id, "chatcmpl-result-1");
        assert_eq!(
            completion.choices[0].message.content.as_deref(),
            Some("Deferred result!")
        );
    }

    #[tokio::test]
    async fn poll_deferred_202_empty_body() {
        let server = MockServer::start().await;

        // 202 with completely empty body — the real xAI behaviour.
        Mock::given(method("GET"))
            .and(path("/v1/chat/deferred-completion/req_empty"))
            .respond_with(ResponseTemplate::new(202))
            .expect(1)
            .mount(&server)
            .await;

        let http = mock_http_client(&server.uri());
        let client = ChatClient::new(&http);

        let result = client.poll_deferred("req_empty").await.unwrap();
        assert!(result.is_none());
    }

    #[test]
    fn poll_deferred_path_encodes_slash() {
        let request_id = "req/abc";
        let path = format!(
            "{}{}",
            DEFERRED_COMPLETION_PATH_PREFIX,
            encode_path_segment(request_id)
        );
        assert_eq!(path, "/v1/chat/deferred-completion/req%2Fabc");
    }

    #[test]
    fn poll_deferred_path_encodes_query_chars() {
        let request_id = "req?v=1";
        let path = format!(
            "{}{}",
            DEFERRED_COMPLETION_PATH_PREFIX,
            encode_path_segment(request_id)
        );
        assert_eq!(path, "/v1/chat/deferred-completion/req%3Fv%3D1");
    }

    #[test]
    fn poll_deferred_path_encodes_hash() {
        let request_id = "req#frag";
        let path = format!(
            "{}{}",
            DEFERRED_COMPLETION_PATH_PREFIX,
            encode_path_segment(request_id)
        );
        assert_eq!(path, "/v1/chat/deferred-completion/req%23frag");
    }

    #[test]
    fn poll_deferred_path_encodes_space() {
        let request_id = "req abc";
        let path = format!(
            "{}{}",
            DEFERRED_COMPLETION_PATH_PREFIX,
            encode_path_segment(request_id)
        );
        assert_eq!(path, "/v1/chat/deferred-completion/req%20abc");
    }

    #[test]
    fn poll_deferred_path_preserves_normal_id() {
        let request_id = "req_done";
        let path = format!(
            "{}{}",
            DEFERRED_COMPLETION_PATH_PREFIX,
            encode_path_segment(request_id)
        );
        assert_eq!(path, "/v1/chat/deferred-completion/req_done");
    }

    #[tokio::test]
    async fn create_returns_error_on_4xx() {
        let server = MockServer::start().await;
        let error_body = serde_json::json!({
            "error": {
                "message": "Invalid model",
                "type": "invalid_request_error",
                "code": "model_not_found"
            }
        });

        Mock::given(method("POST"))
            .and(path("/v1/chat/completions"))
            .respond_with(ResponseTemplate::new(400).set_body_json(&error_body))
            .expect(1)
            .mount(&server)
            .await;

        let http = mock_http_client(&server.uri());
        let client = ChatClient::new(&http);

        let request =
            ChatCompletionBuilder::new("nonexistent-model", vec![Message::text(Role::User, "hi")])
                .build();

        let err = client.create(&request).await.unwrap_err();
        let display = format!("{err}");
        assert!(display.contains("Invalid model"));
    }

    #[tokio::test]
    async fn create_with_tool_calls_response() {
        let server = MockServer::start().await;
        let response_body = serde_json::json!({
            "id": "chatcmpl-tools-1",
            "choices": [
                {
                    "index": 0,
                    "message": {
                        "role": "assistant",
                        "content": null,
                        "tool_calls": [
                            {
                                "id": "call_xyz",
                                "type": "function",
                                "function": {
                                    "name": "get_weather",
                                    "arguments": "{\"city\":\"Tokyo\"}"
                                }
                            }
                        ]
                    },
                    "finish_reason": "tool_calls"
                }
            ]
        });

        Mock::given(method("POST"))
            .and(path("/v1/chat/completions"))
            .respond_with(ResponseTemplate::new(200).set_body_json(&response_body))
            .expect(1)
            .mount(&server)
            .await;

        let http = mock_http_client(&server.uri());
        let client = ChatClient::new(&http);

        let request = ChatCompletionBuilder::new(
            "grok-4",
            vec![Message::text(Role::User, "Weather in Tokyo?")],
        )
        .build();

        let completion = client.create(&request).await.unwrap();
        let msg = &completion.choices[0].message;
        assert!(msg.content.is_none());
        let tool_calls = msg.tool_calls.as_ref().unwrap();
        assert_eq!(tool_calls.len(), 1);
        assert_eq!(tool_calls[0].function.name, "get_weather");
        assert_eq!(tool_calls[0].function.arguments, r#"{"city":"Tokyo"}"#);
    }
}
