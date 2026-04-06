use std::sync::Arc;

use reqwest::Method;

use crate::transport::client::HttpClient;
use crate::transport::error::TransportError;
use crate::types::responses::{CreateResponseRequest, ResponseObject};

use super::util::encode_path_segment;

/// API path prefix for the Responses endpoint.
const RESPONSES_PATH: &str = "/v1/responses";

/// Client for the xAI Responses API.
///
/// Wraps an `Arc<HttpClient>` and exposes typed methods for each endpoint
/// operation: create, retrieve, and delete.
#[derive(Debug, Clone)]
pub struct ResponsesClient {
    http: Arc<HttpClient>,
}

impl ResponsesClient {
    /// Create a new `ResponsesClient` from a shared `HttpClient`.
    #[must_use]
    pub fn new(http: Arc<HttpClient>) -> Self {
        Self { http }
    }

    /// Create a response — `POST /v1/responses`.
    ///
    /// Sends the given request to the xAI Responses API and returns the
    /// completed `ResponseObject`.
    ///
    /// # Errors
    ///
    /// Returns [`TransportError`] if the API request fails.
    pub async fn create(
        &self,
        request: &CreateResponseRequest,
    ) -> Result<ResponseObject, TransportError> {
        self.http
            .send_json(Method::POST, RESPONSES_PATH, request)
            .await
    }

    /// Retrieve an existing response by ID — `GET /v1/responses/{id}`.
    ///
    /// # Errors
    ///
    /// Returns [`TransportError`] if the API request fails.
    pub async fn retrieve(&self, id: &str) -> Result<ResponseObject, TransportError> {
        let path = format!("{}/{}", RESPONSES_PATH, encode_path_segment(id));
        self.http.send_no_body(Method::GET, &path).await
    }

    /// Delete a stored response by ID — `DELETE /v1/responses/{id}`.
    ///
    /// Returns `Ok(())` on success (the server typically returns 204 No Content).
    ///
    /// # Errors
    ///
    /// Returns [`TransportError`] if the API request fails.
    pub async fn delete(&self, id: &str) -> Result<(), TransportError> {
        let path = format!("{}/{}", RESPONSES_PATH, encode_path_segment(id));
        self.http.send_no_body_empty(Method::DELETE, &path).await
    }

    /// Create a streaming response — `POST /v1/responses` with SSE.
    ///
    /// Returns a stream of raw SSE data lines. Use
    /// [`crate::streaming::parser::parse_response_stream`] to deserialize
    /// into typed [`crate::types::stream::ResponseStreamEvent`] values.
    ///
    /// # Errors
    ///
    /// Returns [`TransportError`] if the API request fails.
    pub async fn create_stream(
        &self,
        request: &CreateResponseRequest,
    ) -> Result<impl futures::Stream<Item = Result<String, TransportError>> + use<>, TransportError>
    {
        self.http
            .send_sse(Method::POST, RESPONSES_PATH, request)
            .await
    }

    /// Create a response from a raw JSON request body — `POST /v1/responses`.
    ///
    /// This is useful when the request body contains fields that are not
    /// representable by `CreateResponseRequest` (e.g., function_call_output
    /// items in the input array for multi-turn function calling).
    ///
    /// # Errors
    ///
    /// Returns [`TransportError`] if the API request fails.
    pub async fn create_raw(
        &self,
        request: &serde_json::Value,
    ) -> Result<ResponseObject, TransportError> {
        self.http
            .send_json(Method::POST, RESPONSES_PATH, request)
            .await
    }

    /// Return the HTTP path used for the create endpoint.
    ///
    /// Useful for testing and diagnostics.
    #[must_use]
    pub fn create_path() -> &'static str {
        RESPONSES_PATH
    }

    /// Return the HTTP path used for retrieve/delete operations on a specific response.
    ///
    /// Useful for testing and diagnostics.
    #[must_use]
    pub fn resource_path(id: &str) -> String {
        format!("{}/{}", RESPONSES_PATH, encode_path_segment(id))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn create_path_is_correct() {
        assert_eq!(ResponsesClient::create_path(), "/v1/responses");
    }

    #[test]
    fn resource_path_is_correct() {
        assert_eq!(
            ResponsesClient::resource_path("resp_abc123"),
            "/v1/responses/resp_abc123"
        );
    }

    #[test]
    fn resource_path_with_complex_id() {
        assert_eq!(
            ResponsesClient::resource_path("resp_abc-123_XYZ"),
            "/v1/responses/resp_abc-123_XYZ"
        );
    }

    #[test]
    fn resource_path_encodes_slash() {
        assert_eq!(
            ResponsesClient::resource_path("resp/abc"),
            "/v1/responses/resp%2Fabc"
        );
    }

    #[test]
    fn resource_path_encodes_query_chars() {
        assert_eq!(
            ResponsesClient::resource_path("resp?v=1"),
            "/v1/responses/resp%3Fv%3D1"
        );
    }

    #[test]
    fn resource_path_encodes_hash() {
        assert_eq!(
            ResponsesClient::resource_path("resp#frag"),
            "/v1/responses/resp%23frag"
        );
    }

    #[test]
    fn resource_path_encodes_space() {
        assert_eq!(
            ResponsesClient::resource_path("resp abc"),
            "/v1/responses/resp%20abc"
        );
    }

    #[test]
    fn create_uses_post_method() {
        // Verify the constant and method are correct by checking the path.
        // The actual HTTP method is hardcoded as Method::POST in the create() method.
        assert_eq!(RESPONSES_PATH, "/v1/responses");
    }

    #[test]
    fn retrieve_uses_get_path() {
        let path = ResponsesClient::resource_path("resp_test");
        assert_eq!(path, "/v1/responses/resp_test");
    }

    #[test]
    fn delete_uses_delete_path() {
        let path = ResponsesClient::resource_path("resp_del");
        assert_eq!(path, "/v1/responses/resp_del");
    }

    /// Integration-style test verifying that `ResponsesClient::create` sends
    /// the correct HTTP method and path by using a wiremock server.
    #[tokio::test]
    async fn create_sends_post_to_correct_path() {
        use crate::transport::auth::ApiKeySecret;
        use crate::transport::client::HttpClientConfig;
        use crate::types::responses::{CreateResponseBuilder, ResponseInput};
        use wiremock::matchers::{method, path};
        use wiremock::{Mock, MockServer, ResponseTemplate};

        let server = MockServer::start().await;

        let response_body = serde_json::json!({
            "id": "resp_test_123",
            "status": "completed",
            "output": []
        });

        Mock::given(method("POST"))
            .and(path("/v1/responses"))
            .respond_with(ResponseTemplate::new(200).set_body_json(&response_body))
            .expect(1)
            .mount(&server)
            .await;

        let config = HttpClientConfig {
            base_url: server.uri(),
            ..Default::default()
        };
        let client = HttpClient::new(
            config,
            ApiKeySecret::new("test-key"),
            Some(Arc::new(crate::transport::policy_gate::AllowAllGate)),
        )
        .unwrap();
        let responses_client = ResponsesClient::new(Arc::new(client));

        let request =
            CreateResponseBuilder::new("grok-4", ResponseInput::Text("Hello".into())).build();

        let result = responses_client.create(&request).await.unwrap();
        assert_eq!(result.id, "resp_test_123");
    }

    /// Verify that retrieve sends GET to the correct path.
    #[tokio::test]
    async fn retrieve_sends_get_to_correct_path() {
        use crate::transport::auth::ApiKeySecret;
        use crate::transport::client::HttpClientConfig;
        use wiremock::matchers::{method, path};
        use wiremock::{Mock, MockServer, ResponseTemplate};

        let server = MockServer::start().await;

        let response_body = serde_json::json!({
            "id": "resp_retrieve_1",
            "status": "completed",
            "output": [
                {
                    "type": "message",
                    "role": "assistant",
                    "content": [{"type":"text","text":"Hello!"}]
                }
            ]
        });

        Mock::given(method("GET"))
            .and(path("/v1/responses/resp_retrieve_1"))
            .respond_with(ResponseTemplate::new(200).set_body_json(&response_body))
            .expect(1)
            .mount(&server)
            .await;

        let config = HttpClientConfig {
            base_url: server.uri(),
            ..Default::default()
        };
        let client = HttpClient::new(
            config,
            ApiKeySecret::new("test-key"),
            Some(Arc::new(crate::transport::policy_gate::AllowAllGate)),
        )
        .unwrap();
        let responses_client = ResponsesClient::new(Arc::new(client));

        let result = responses_client.retrieve("resp_retrieve_1").await.unwrap();
        assert_eq!(result.id, "resp_retrieve_1");
        assert_eq!(result.output.len(), 1);
    }

    /// Verify that delete sends DELETE to the correct path.
    #[tokio::test]
    async fn delete_sends_delete_to_correct_path() {
        use crate::transport::auth::ApiKeySecret;
        use crate::transport::client::HttpClientConfig;
        use wiremock::matchers::{method, path};
        use wiremock::{Mock, MockServer, ResponseTemplate};

        let server = MockServer::start().await;

        Mock::given(method("DELETE"))
            .and(path("/v1/responses/resp_del_1"))
            .respond_with(ResponseTemplate::new(204))
            .expect(1)
            .mount(&server)
            .await;

        let config = HttpClientConfig {
            base_url: server.uri(),
            ..Default::default()
        };
        let client = HttpClient::new(
            config,
            ApiKeySecret::new("test-key"),
            Some(Arc::new(crate::transport::policy_gate::AllowAllGate)),
        )
        .unwrap();
        let responses_client = ResponsesClient::new(Arc::new(client));

        let result = responses_client.delete("resp_del_1").await;
        assert!(result.is_ok());
    }

    /// Verify that create sends the `store: false` default in the request body.
    #[tokio::test]
    async fn create_sends_store_false_by_default() {
        use crate::transport::auth::ApiKeySecret;
        use crate::transport::client::HttpClientConfig;
        use crate::types::responses::{CreateResponseBuilder, ResponseInput};
        use wiremock::matchers::{method, path};
        use wiremock::{Mock, MockServer, ResponseTemplate};

        let server = MockServer::start().await;

        let response_body = serde_json::json!({
            "id": "resp_store_check",
            "status": "completed",
            "output": []
        });

        Mock::given(method("POST"))
            .and(path("/v1/responses"))
            .respond_with(ResponseTemplate::new(200).set_body_json(&response_body))
            .expect(1)
            .mount(&server)
            .await;

        let config = HttpClientConfig {
            base_url: server.uri(),
            ..Default::default()
        };
        let client = HttpClient::new(
            config,
            ApiKeySecret::new("test-key"),
            Some(Arc::new(crate::transport::policy_gate::AllowAllGate)),
        )
        .unwrap();
        let responses_client = ResponsesClient::new(Arc::new(client));

        let request =
            CreateResponseBuilder::new("grok-4", ResponseInput::Text("test store".into())).build();

        let _ = responses_client.create(&request).await.unwrap();

        // Verify the request body contained store: false
        let received = server.received_requests().await.unwrap();
        assert_eq!(received.len(), 1);
        let body: serde_json::Value = serde_json::from_slice(&received[0].body).unwrap();
        assert_eq!(body["store"], serde_json::json!(false));
    }

    /// Verify error handling when the server returns an error status.
    #[tokio::test]
    async fn create_returns_error_on_4xx() {
        use crate::transport::auth::ApiKeySecret;
        use crate::transport::client::HttpClientConfig;
        use crate::types::responses::{CreateResponseBuilder, ResponseInput};
        use wiremock::matchers::{method, path};
        use wiremock::{Mock, MockServer, ResponseTemplate};

        let server = MockServer::start().await;

        let error_body = serde_json::json!({
            "error": {
                "message": "Invalid model",
                "type": "invalid_request_error"
            }
        });

        Mock::given(method("POST"))
            .and(path("/v1/responses"))
            .respond_with(ResponseTemplate::new(400).set_body_json(&error_body))
            .expect(1)
            .mount(&server)
            .await;

        let config = HttpClientConfig {
            base_url: server.uri(),
            ..Default::default()
        };
        let client = HttpClient::new(
            config,
            ApiKeySecret::new("test-key"),
            Some(Arc::new(crate::transport::policy_gate::AllowAllGate)),
        )
        .unwrap();
        let responses_client = ResponsesClient::new(Arc::new(client));

        let request =
            CreateResponseBuilder::new("invalid-model", ResponseInput::Text("test".into())).build();

        let result = responses_client.create(&request).await;
        assert!(result.is_err());
        match result.unwrap_err() {
            TransportError::Api(api_err) => {
                assert_eq!(api_err.status_code, 400);
                assert!(api_err.message.contains("Invalid model"));
            }
            other => panic!("expected Api error, got: {other}"),
        }
    }
}
