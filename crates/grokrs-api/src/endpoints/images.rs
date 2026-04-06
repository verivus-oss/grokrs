//! Image Generation API endpoint client.
//!
//! This module provides `ImagesClient`, which wraps `HttpClient` to send
//! requests to the xAI `/v1/images/generations` and `/v1/images/edits`
//! endpoints.

use std::sync::Arc;

use reqwest::Method;

use crate::transport::client::HttpClient;
use crate::transport::error::TransportError;
use crate::types::images::{ImageEditRequest, ImageGenerationRequest, ImageResponse};

/// The path for the image generation endpoint.
const IMAGE_GENERATIONS_PATH: &str = "/v1/images/generations";

/// The path for the image edit endpoint.
const IMAGE_EDITS_PATH: &str = "/v1/images/edits";

/// A client for the xAI Image Generation API.
///
/// Holds a shared reference to an `HttpClient` and provides typed methods for
/// generating and editing images.
#[derive(Debug, Clone)]
pub struct ImagesClient {
    http: Arc<HttpClient>,
}

impl ImagesClient {
    /// Create a new `ImagesClient` from a shared `HttpClient`.
    #[must_use]
    pub fn new(http: Arc<HttpClient>) -> Self {
        Self { http }
    }

    /// Generate images from a text prompt.
    ///
    /// Issues `POST /v1/images/generations` with the given request body.
    ///
    /// # Errors
    ///
    /// Returns [`TransportError`] if the API request fails.
    pub async fn generate(
        &self,
        request: &ImageGenerationRequest,
    ) -> Result<ImageResponse, TransportError> {
        self.http
            .send_json(Method::POST, IMAGE_GENERATIONS_PATH, request)
            .await
    }

    /// Edit an existing image based on a text prompt.
    ///
    /// Issues `POST /v1/images/edits` with the given request body.
    ///
    /// # Errors
    ///
    /// Returns [`TransportError`] if the API request fails.
    pub async fn edit(&self, request: &ImageEditRequest) -> Result<ImageResponse, TransportError> {
        self.http
            .send_json(Method::POST, IMAGE_EDITS_PATH, request)
            .await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::transport::auth::ApiKeySecret;
    use crate::transport::client::HttpClientConfig;
    use crate::transport::policy_gate::AllowAllGate;
    use crate::types::images::{AspectRatio, ImageResolution, ImageResponseFormat};
    use wiremock::matchers::{body_partial_json, header, method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    /// Create an `HttpClient` pointed at the mock server.
    fn mock_http_client(base_url: &str) -> Arc<HttpClient> {
        let config = HttpClientConfig {
            base_url: base_url.to_string(),
            ..Default::default()
        };
        Arc::new(
            HttpClient::new(
                config,
                ApiKeySecret::new("test-key"),
                Some(Arc::new(AllowAllGate)),
            )
            .unwrap(),
        )
    }

    #[test]
    fn generate_path_is_correct() {
        assert_eq!(IMAGE_GENERATIONS_PATH, "/v1/images/generations");
    }

    #[test]
    fn edit_path_is_correct() {
        assert_eq!(IMAGE_EDITS_PATH, "/v1/images/edits");
    }

    #[tokio::test]
    async fn generate_sends_post_and_deserializes() {
        let server = MockServer::start().await;
        let response_body = serde_json::json!({
            "created": 1_700_000_000,
            "data": [
                {
                    "url": "https://example.com/generated.png",
                    "revised_prompt": "A cat wearing a fancy top hat"
                }
            ]
        });

        Mock::given(method("POST"))
            .and(path("/v1/images/generations"))
            .and(header("authorization", "Bearer test-key"))
            .and(body_partial_json(serde_json::json!({
                "prompt": "A cat wearing a top hat",
                "model": "grok-2-image"
            })))
            .respond_with(ResponseTemplate::new(200).set_body_json(&response_body))
            .expect(1)
            .mount(&server)
            .await;

        let http = mock_http_client(&server.uri());
        let client = ImagesClient::new(http);

        let request = ImageGenerationRequest {
            prompt: "A cat wearing a top hat".to_string(),
            model: "grok-2-image".to_string(),
            n: Some(1),
            aspect_ratio: Some(AspectRatio::Square),
            quality: None,
            resolution: Some(ImageResolution::Res1024x1024),
            response_format: Some(ImageResponseFormat::Url),
        };

        let resp = client.generate(&request).await.unwrap();
        assert_eq!(resp.created, Some(1_700_000_000));
        assert_eq!(resp.data.len(), 1);
        assert_eq!(
            resp.data[0].url.as_deref(),
            Some("https://example.com/generated.png")
        );
    }

    #[tokio::test]
    async fn edit_sends_post_and_deserializes() {
        let server = MockServer::start().await;
        let response_body = serde_json::json!({
            "data": [
                {
                    "url": "https://example.com/edited.png"
                }
            ]
        });

        Mock::given(method("POST"))
            .and(path("/v1/images/edits"))
            .and(header("authorization", "Bearer test-key"))
            .and(body_partial_json(serde_json::json!({
                "prompt": "Remove the background",
                "model": "grok-2-image"
            })))
            .respond_with(ResponseTemplate::new(200).set_body_json(&response_body))
            .expect(1)
            .mount(&server)
            .await;

        let http = mock_http_client(&server.uri());
        let client = ImagesClient::new(http);

        let request = ImageEditRequest {
            prompt: "Remove the background".to_string(),
            model: "grok-2-image".to_string(),
            image: Some("https://example.com/photo.jpg".to_string()),
            images: None,
            mask: None,
        };

        let resp = client.edit(&request).await.unwrap();
        assert_eq!(resp.data.len(), 1);
        assert_eq!(
            resp.data[0].url.as_deref(),
            Some("https://example.com/edited.png")
        );
    }

    #[tokio::test]
    async fn generate_returns_error_on_4xx() {
        let server = MockServer::start().await;
        let error_body = serde_json::json!({
            "error": {
                "message": "Invalid prompt",
                "type": "invalid_request_error"
            }
        });

        Mock::given(method("POST"))
            .and(path("/v1/images/generations"))
            .respond_with(ResponseTemplate::new(400).set_body_json(&error_body))
            .expect(1)
            .mount(&server)
            .await;

        let http = mock_http_client(&server.uri());
        let client = ImagesClient::new(http);

        let request = ImageGenerationRequest {
            prompt: String::new(),
            model: "grok-2-image".to_string(),
            n: None,
            aspect_ratio: None,
            quality: None,
            resolution: None,
            response_format: None,
        };

        let err = client.generate(&request).await.unwrap_err();
        let display = format!("{err}");
        assert!(display.contains("Invalid prompt"));
    }
}
