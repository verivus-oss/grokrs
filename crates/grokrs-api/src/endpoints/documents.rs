//! Document Search API endpoint client.
//!
//! This module provides `DocumentsClient`, which wraps `HttpClient` to send
//! requests to the xAI `POST /v1/documents/search` endpoint. Document search
//! uses the standard inference API key (not the management key).

use std::sync::Arc;

use reqwest::Method;

use crate::transport::client::HttpClient;
use crate::transport::error::TransportError;
use crate::types::documents::{DocumentSearchRequest, DocumentSearchResponse};

/// The path for the document search endpoint.
const DOCUMENT_SEARCH_PATH: &str = "/v1/documents/search";

/// A client for the xAI Document Search API.
///
/// Holds a shared reference to an `HttpClient` and provides a typed method for
/// searching document collections. This client uses the standard inference API
/// key — it does not depend on the Collections Management API or management key.
#[derive(Debug, Clone)]
pub struct DocumentsClient {
    http: Arc<HttpClient>,
}

impl DocumentsClient {
    /// Create a new `DocumentsClient` from a shared `HttpClient`.
    pub fn new(http: Arc<HttpClient>) -> Self {
        Self { http }
    }

    /// Search document collections for matching chunks.
    ///
    /// Issues `POST /v1/documents/search` with the given request body.
    ///
    /// # Errors
    ///
    /// Returns `TransportError::Serialization` if `collection_ids` is empty
    /// (validated at the client level to avoid confusing server errors).
    ///
    /// Returns other `TransportError` variants for network, policy, or API errors.
    pub async fn search(
        &self,
        request: &DocumentSearchRequest,
    ) -> Result<DocumentSearchResponse, TransportError> {
        if request.source.collection_ids.is_empty() {
            return Err(TransportError::Serialization {
                message: "collection_ids must contain at least one collection ID".to_string(),
            });
        }

        self.http
            .send_json(Method::POST, DOCUMENT_SEARCH_PATH, request)
            .await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::transport::auth::ApiKeySecret;
    use crate::transport::client::HttpClientConfig;
    use crate::transport::policy_gate::AllowAllGate;
    use crate::types::documents::{
        DocumentSearchRequest, DocumentsSource, RankingMetric, RetrievalMode,
    };
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

    fn minimal_request() -> DocumentSearchRequest {
        DocumentSearchRequest {
            query: "What is Rust?".to_string(),
            source: DocumentsSource {
                collection_ids: vec!["col-abc".to_string()],
            },
            filter: None,
            group_by: None,
            ranking_metric: None,
            retrieval_mode: None,
            limit: None,
        }
    }

    #[test]
    fn search_path_is_correct() {
        assert_eq!(DOCUMENT_SEARCH_PATH, "/v1/documents/search");
    }

    #[test]
    fn documents_client_can_be_constructed() {
        let config = HttpClientConfig::default();
        let key = ApiKeySecret::new("test-key");
        let http = HttpClient::new(config, key, Some(Arc::new(AllowAllGate)))
            .expect("HttpClient construction");
        let _client = DocumentsClient::new(Arc::new(http));
    }

    #[tokio::test]
    async fn search_sends_post_and_deserializes() {
        let server = MockServer::start().await;
        let response_body = serde_json::json!({
            "matches": [
                {
                    "file_id": "file-123",
                    "chunk_id": "chunk-456",
                    "chunk_content": "Rust is a systems programming language.",
                    "score": 0.95,
                    "collection_ids": ["col-abc"],
                    "page_number": 3
                }
            ]
        });

        Mock::given(method("POST"))
            .and(path("/v1/documents/search"))
            .and(header("authorization", "Bearer test-key"))
            .and(body_partial_json(serde_json::json!({
                "query": "What is Rust?",
                "source": {"collection_ids": ["col-abc"]}
            })))
            .respond_with(ResponseTemplate::new(200).set_body_json(&response_body))
            .expect(1)
            .mount(&server)
            .await;

        let http = mock_http_client(&server.uri());
        let client = DocumentsClient::new(http);
        let resp = client.search(&minimal_request()).await.unwrap();

        assert_eq!(resp.matches.len(), 1);
        assert_eq!(resp.matches[0].file_id, "file-123");
        assert_eq!(resp.matches[0].chunk_id, "chunk-456");
        assert!((resp.matches[0].score - 0.95).abs() < f64::EPSILON);
        assert_eq!(resp.matches[0].page_number, Some(3));
    }

    #[tokio::test]
    async fn search_with_all_options() {
        let server = MockServer::start().await;
        let response_body = serde_json::json!({
            "matches": []
        });

        Mock::given(method("POST"))
            .and(path("/v1/documents/search"))
            .and(body_partial_json(serde_json::json!({
                "query": "advanced search",
                "source": {"collection_ids": ["col-1", "col-2"]},
                "filter": "metadata.lang = \"en\"",
                "group_by": "file_id",
                "ranking_metric": "cosine",
                "retrieval_mode": {"type": "hybrid"},
                "limit": 5
            })))
            .respond_with(ResponseTemplate::new(200).set_body_json(&response_body))
            .expect(1)
            .mount(&server)
            .await;

        let http = mock_http_client(&server.uri());
        let client = DocumentsClient::new(http);

        let request = DocumentSearchRequest {
            query: "advanced search".to_string(),
            source: DocumentsSource {
                collection_ids: vec!["col-1".to_string(), "col-2".to_string()],
            },
            filter: Some("metadata.lang = \"en\"".to_string()),
            group_by: Some("file_id".to_string()),
            ranking_metric: Some(RankingMetric::Cosine),
            retrieval_mode: Some(RetrievalMode {
                mode_type: "hybrid".to_string(),
            }),
            limit: Some(5),
        };

        let resp = client.search(&request).await.unwrap();
        assert!(resp.matches.is_empty());
    }

    #[tokio::test]
    async fn search_returns_error_on_4xx() {
        let server = MockServer::start().await;
        let error_body = serde_json::json!({
            "error": {
                "message": "Collection not found",
                "type": "not_found_error"
            }
        });

        Mock::given(method("POST"))
            .and(path("/v1/documents/search"))
            .respond_with(ResponseTemplate::new(404).set_body_json(&error_body))
            .expect(1)
            .mount(&server)
            .await;

        let http = mock_http_client(&server.uri());
        let client = DocumentsClient::new(http);

        let err = client.search(&minimal_request()).await.unwrap_err();
        let display = format!("{err}");
        assert!(display.contains("Collection not found"));
    }

    #[tokio::test]
    async fn search_rejects_empty_collection_ids() {
        let server = MockServer::start().await;

        // No mocks mounted — the request should never reach the server.
        let http = mock_http_client(&server.uri());
        let client = DocumentsClient::new(http);

        let request = DocumentSearchRequest {
            query: "test".to_string(),
            source: DocumentsSource {
                collection_ids: vec![],
            },
            filter: None,
            group_by: None,
            ranking_metric: None,
            retrieval_mode: None,
            limit: None,
        };

        let err = client.search(&request).await.unwrap_err();
        match err {
            TransportError::Serialization { message } => {
                assert!(
                    message.contains("collection_ids"),
                    "error should mention collection_ids: {message}"
                );
            }
            other => panic!("expected Serialization error, got: {other}"),
        }
    }

    #[tokio::test]
    async fn search_multiple_matches_with_fields() {
        let server = MockServer::start().await;
        let response_body = serde_json::json!({
            "matches": [
                {
                    "file_id": "f1",
                    "chunk_id": "c1",
                    "chunk_content": "First match",
                    "score": 0.99,
                    "collection_ids": ["col-1"],
                    "fields": {"source": "wiki", "year": 2024},
                    "page_number": 1
                },
                {
                    "file_id": "f2",
                    "chunk_id": "c2",
                    "chunk_content": "Second match",
                    "score": 0.85,
                    "collection_ids": ["col-1", "col-2"],
                    "fields": null
                }
            ]
        });

        Mock::given(method("POST"))
            .and(path("/v1/documents/search"))
            .respond_with(ResponseTemplate::new(200).set_body_json(&response_body))
            .expect(1)
            .mount(&server)
            .await;

        let http = mock_http_client(&server.uri());
        let client = DocumentsClient::new(http);
        let resp = client.search(&minimal_request()).await.unwrap();

        assert_eq!(resp.matches.len(), 2);
        assert_eq!(resp.matches[0].file_id, "f1");
        assert!(resp.matches[0].fields.is_some());
        assert_eq!(
            resp.matches[0].fields.as_ref().unwrap()["source"],
            serde_json::json!("wiki")
        );
        assert_eq!(resp.matches[1].file_id, "f2");
        assert_eq!(resp.matches[1].collection_ids.len(), 2);
    }
}
