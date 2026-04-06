use reqwest::Method;

use crate::transport::client::HttpClient;
use crate::transport::error::TransportError;
use crate::types::tokenize::{TokenizeRequest, TokenizeResponse};

/// Client for the xAI Tokenizer API.
///
/// Provides access to the `/v1/tokenize-text` endpoint for converting text
/// into token representations using a specified model's tokenizer.
pub struct TokenizeClient<'a> {
    http: &'a HttpClient,
}

impl<'a> TokenizeClient<'a> {
    /// Create a new `TokenizeClient` wrapping the given HTTP client.
    #[must_use]
    pub fn new(http: &'a HttpClient) -> Self {
        Self { http }
    }

    /// Tokenize text using the specified model's tokenizer.
    ///
    /// Sends a `POST /v1/tokenize-text` request and returns the tokenized
    /// representation including token IDs, string tokens, and raw byte
    /// sequences.
    ///
    /// # Arguments
    /// * `text` - The text to tokenize.
    /// * `model` - The model whose tokenizer should be used (e.g., "grok-4").
    pub async fn tokenize(
        &self,
        text: &str,
        model: &str,
    ) -> Result<TokenizeResponse, TransportError> {
        let body = TokenizeRequest {
            text: text.to_string(),
            model: model.to_string(),
        };
        self.http
            .send_json(Method::POST, "/v1/tokenize-text", &body)
            .await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tokenize_request_construction() {
        let body = TokenizeRequest {
            text: "Hello, world!".to_string(),
            model: "grok-4".to_string(),
        };
        let json = serde_json::to_string(&body).unwrap();
        assert!(json.contains("\"text\":\"Hello, world!\""));
        assert!(json.contains("\"model\":\"grok-4\""));
    }

    #[test]
    fn tokenize_endpoint_path() {
        let path = "/v1/tokenize-text";
        assert_eq!(path, "/v1/tokenize-text");
    }

    #[test]
    fn tokenize_request_with_empty_text() {
        let body = TokenizeRequest {
            text: String::new(),
            model: "grok-4-mini".to_string(),
        };
        let json = serde_json::to_string(&body).unwrap();
        assert!(json.contains("\"text\":\"\""));
    }

    #[test]
    fn tokenize_request_with_unicode() {
        let body = TokenizeRequest {
            text: "Hello \u{1F600} world \u{4E16}\u{754C}".to_string(),
            model: "grok-4".to_string(),
        };
        let json = serde_json::to_string(&body).unwrap();
        let back: TokenizeRequest = serde_json::from_str(&json).unwrap();
        assert_eq!(body, back);
    }
}
