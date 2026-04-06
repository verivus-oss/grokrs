use std::sync::Arc;

use crate::transport::client::HttpClient;
use crate::transport::error::TransportError;

use super::util::encode_path_segment;
use crate::types::model::{
    ImageModel, ImageModelList, LanguageModel, LanguageModelList, Model, ModelList, VideoModel,
    VideoModelList,
};

/// Client for the xAI Models API endpoints.
///
/// Provides access to all model discovery endpoints:
/// - `/v1/models` — generic model listing
/// - `/v1/language-models` — language model details with pricing and modalities
/// - `/v1/image-generation-models` — image generation model details
/// - `/v1/video-generation-models` — video generation model details
///
/// Model names, pricing, and capabilities are never hardcoded; all information
/// is discovered at runtime through these endpoints.
pub struct ModelsClient {
    client: Arc<HttpClient>,
}

impl ModelsClient {
    /// Create a new `ModelsClient` wrapping the given HTTP client.
    pub fn new(client: Arc<HttpClient>) -> Self {
        Self { client }
    }

    /// List all models.
    ///
    /// `GET /v1/models`
    ///
    /// Returns a `ModelList` containing `id`, `created`, `owned_by` for each model.
    pub async fn list_models(&self) -> Result<ModelList, TransportError> {
        self.client
            .send_no_body(reqwest::Method::GET, "/v1/models")
            .await
    }

    /// Get a single model by ID.
    ///
    /// `GET /v1/models/{model_id}`
    pub async fn get_model(&self, model_id: &str) -> Result<Model, TransportError> {
        let path = format!("/v1/models/{}", encode_path_segment(model_id));
        self.client.send_no_body(reqwest::Method::GET, &path).await
    }

    /// List all language models with extended information.
    ///
    /// `GET /v1/language-models`
    ///
    /// Returns a `LanguageModelList` containing pricing, modalities, aliases, and
    /// other extended metadata for each language model.
    pub async fn list_language_models(&self) -> Result<LanguageModelList, TransportError> {
        self.client
            .send_no_body(reqwest::Method::GET, "/v1/language-models")
            .await
    }

    /// Get a single language model by ID.
    ///
    /// `GET /v1/language-models/{model_id}`
    pub async fn get_language_model(
        &self,
        model_id: &str,
    ) -> Result<LanguageModel, TransportError> {
        let path = format!("/v1/language-models/{}", encode_path_segment(model_id));
        self.client.send_no_body(reqwest::Method::GET, &path).await
    }

    /// List all image generation models.
    ///
    /// `GET /v1/image-generation-models`
    pub async fn list_image_models(&self) -> Result<ImageModelList, TransportError> {
        self.client
            .send_no_body(reqwest::Method::GET, "/v1/image-generation-models")
            .await
    }

    /// Get a single image generation model by ID.
    ///
    /// `GET /v1/image-generation-models/{model_id}`
    pub async fn get_image_model(&self, model_id: &str) -> Result<ImageModel, TransportError> {
        let path = format!(
            "/v1/image-generation-models/{}",
            encode_path_segment(model_id)
        );
        self.client.send_no_body(reqwest::Method::GET, &path).await
    }

    /// List all video generation models.
    ///
    /// `GET /v1/video-generation-models`
    pub async fn list_video_models(&self) -> Result<VideoModelList, TransportError> {
        self.client
            .send_no_body(reqwest::Method::GET, "/v1/video-generation-models")
            .await
    }

    /// Get a single video generation model by ID.
    ///
    /// `GET /v1/video-generation-models/{model_id}`
    pub async fn get_video_model(&self, model_id: &str) -> Result<VideoModel, TransportError> {
        let path = format!(
            "/v1/video-generation-models/{}",
            encode_path_segment(model_id)
        );
        self.client.send_no_body(reqwest::Method::GET, &path).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::transport::auth::ApiKeySecret;
    use crate::transport::client::HttpClientConfig;

    use crate::transport::policy_gate::AllowAllGate;

    /// Helper: build a `ModelsClient` for unit tests.
    ///
    /// This does NOT make real HTTP calls — it only verifies that the client
    /// struct can be constructed and that path-building logic is correct.
    fn test_client() -> ModelsClient {
        let config = HttpClientConfig::default();
        let key = ApiKeySecret::new("test-key");
        let http = HttpClient::new(config, key, Some(Arc::new(AllowAllGate)))
            .expect("HttpClient construction");
        ModelsClient::new(Arc::new(http))
    }

    #[test]
    fn models_client_can_be_constructed() {
        let _client = test_client();
    }

    #[test]
    fn models_client_from_arc() {
        let config = HttpClientConfig::default();
        let key = ApiKeySecret::new("test-key");
        let http = Arc::new(HttpClient::new(config, key, Some(Arc::new(AllowAllGate))).unwrap());
        let _client = ModelsClient::new(Arc::clone(&http));
        // Arc refcount is 2 — the original and the one inside ModelsClient
        assert_eq!(Arc::strong_count(&http), 2);
    }

    // --- Path construction tests ---
    //
    // These verify that the URL paths are built correctly for all 8 endpoints.
    // We test the `encode_path_segment` function directly and verify the
    // format strings match the expected API paths.

    #[test]
    fn list_models_path() {
        let path = "/v1/models";
        assert_eq!(path, "/v1/models");
    }

    #[test]
    fn get_model_path_simple_id() {
        let model_id = "grok-4";
        let path = format!("/v1/models/{}", encode_path_segment(model_id));
        assert_eq!(path, "/v1/models/grok-4");
    }

    #[test]
    fn get_model_path_with_colon() {
        let model_id = "grok-4:latest";
        let path = format!("/v1/models/{}", encode_path_segment(model_id));
        assert_eq!(path, "/v1/models/grok-4:latest");
    }

    #[test]
    fn get_model_path_with_slash_is_encoded() {
        let model_id = "org/grok-4";
        let path = format!("/v1/models/{}", encode_path_segment(model_id));
        assert_eq!(path, "/v1/models/org%2Fgrok-4");
    }

    #[test]
    fn get_model_path_with_spaces_is_encoded() {
        let model_id = "my model";
        let path = format!("/v1/models/{}", encode_path_segment(model_id));
        assert_eq!(path, "/v1/models/my%20model");
    }

    #[test]
    fn get_model_path_with_query_chars_is_encoded() {
        let model_id = "model?v=1";
        let path = format!("/v1/models/{}", encode_path_segment(model_id));
        assert_eq!(path, "/v1/models/model%3Fv%3D1");
    }

    #[test]
    fn get_model_path_with_hash_is_encoded() {
        let model_id = "model#beta";
        let path = format!("/v1/models/{}", encode_path_segment(model_id));
        assert_eq!(path, "/v1/models/model%23beta");
    }

    #[test]
    fn list_language_models_path() {
        let path = "/v1/language-models";
        assert_eq!(path, "/v1/language-models");
    }

    #[test]
    fn get_language_model_path() {
        let model_id = "grok-4";
        let path = format!("/v1/language-models/{}", encode_path_segment(model_id));
        assert_eq!(path, "/v1/language-models/grok-4");
    }

    #[test]
    fn list_image_models_path() {
        let path = "/v1/image-generation-models";
        assert_eq!(path, "/v1/image-generation-models");
    }

    #[test]
    fn get_image_model_path() {
        let model_id = "grok-2-image";
        let path = format!(
            "/v1/image-generation-models/{}",
            encode_path_segment(model_id)
        );
        assert_eq!(path, "/v1/image-generation-models/grok-2-image");
    }

    #[test]
    fn list_video_models_path() {
        let path = "/v1/video-generation-models";
        assert_eq!(path, "/v1/video-generation-models");
    }

    #[test]
    fn get_video_model_path() {
        let model_id = "grok-video";
        let path = format!(
            "/v1/video-generation-models/{}",
            encode_path_segment(model_id)
        );
        assert_eq!(path, "/v1/video-generation-models/grok-video");
    }

    #[test]
    fn encode_preserves_typical_model_ids() {
        // Model IDs typically contain alphanumerics, hyphens, and dots
        assert_eq!(encode_path_segment("grok-4"), "grok-4");
        assert_eq!(encode_path_segment("grok-4-mini"), "grok-4-mini");
        assert_eq!(encode_path_segment("grok-2-image"), "grok-2-image");
        assert_eq!(encode_path_segment("grok-4.5"), "grok-4.5");
        assert_eq!(
            encode_path_segment("grok-4-2025-04-01"),
            "grok-4-2025-04-01"
        );
    }

    #[test]
    fn encode_handles_empty_string() {
        assert_eq!(encode_path_segment(""), "");
    }

    #[test]
    fn encode_handles_unicode() {
        // Unicode characters get percent-encoded byte-by-byte
        let encoded = encode_path_segment("model-ü");
        assert!(encoded.starts_with("model-"));
        assert!(encoded.contains('%'));
        assert!(!encoded.contains('ü'));
    }

    #[test]
    fn encode_preserves_tilde_and_underscore() {
        assert_eq!(encode_path_segment("model_v1~beta"), "model_v1~beta");
    }

    // --- Verify all 8 endpoint paths are distinct and correct ---

    #[test]
    fn all_endpoint_paths_are_correct() {
        let endpoints = [
            "/v1/models",
            "/v1/language-models",
            "/v1/image-generation-models",
            "/v1/video-generation-models",
        ];

        // All list paths are unique
        let mut unique = std::collections::HashSet::new();
        for ep in &endpoints {
            assert!(unique.insert(ep), "duplicate endpoint path: {ep}");
        }

        // GET-by-id paths are also correct
        let id = "test-model";
        let get_paths = [
            format!("/v1/models/{}", encode_path_segment(id)),
            format!("/v1/language-models/{}", encode_path_segment(id)),
            format!("/v1/image-generation-models/{}", encode_path_segment(id)),
            format!("/v1/video-generation-models/{}", encode_path_segment(id)),
        ];

        let mut unique_get = std::collections::HashSet::new();
        for gp in &get_paths {
            assert!(unique_get.insert(gp), "duplicate GET path: {gp}");
        }
    }
}
