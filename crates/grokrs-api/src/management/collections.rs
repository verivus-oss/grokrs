//! Collections endpoint client for the Management API.
//!
//! Provides CRUD operations on collections:
//! - `POST   /v1/collections`          — create
//! - `GET    /v1/collections`          — list
//! - `GET    /v1/collections/{id}`     — get
//! - `PUT    /v1/collections/{id}`     — update
//! - `DELETE /v1/collections/{id}`     — delete

use std::sync::Arc;

use reqwest::Method;

use crate::management::documents::DocumentsClient;
use crate::transport::client::HttpClient;
use crate::transport::error::TransportError;
use crate::types::collections::{
    Collection, CollectionList, CreateCollectionRequest, UpdateCollectionRequest,
};

/// Client for collection CRUD operations on the Management API.
pub struct CollectionsClient {
    http: Arc<HttpClient>,
}

impl CollectionsClient {
    /// Create a new `CollectionsClient` wrapping the given `HttpClient`.
    pub fn new(http: Arc<HttpClient>) -> Self {
        Self { http }
    }

    /// Create a new collection.
    ///
    /// `POST /v1/collections`
    pub async fn create(
        &self,
        request: &CreateCollectionRequest,
    ) -> Result<Collection, TransportError> {
        self.http
            .send_json(Method::POST, "/v1/collections", request)
            .await
    }

    /// List all collections.
    ///
    /// `GET /v1/collections`
    pub async fn list(&self) -> Result<CollectionList, TransportError> {
        self.http.send_no_body(Method::GET, "/v1/collections").await
    }

    /// Get a single collection by ID.
    ///
    /// `GET /v1/collections/{id}`
    pub async fn get(&self, collection_id: &str) -> Result<Collection, TransportError> {
        let path = format!("/v1/collections/{}", percent_encode(collection_id));
        self.http.send_no_body(Method::GET, &path).await
    }

    /// Update a collection by ID.
    ///
    /// `PUT /v1/collections/{id}`
    ///
    /// Only the fields present in the request are updated. Absent fields are
    /// left unchanged on the server.
    pub async fn update(
        &self,
        collection_id: &str,
        request: &UpdateCollectionRequest,
    ) -> Result<Collection, TransportError> {
        let path = format!("/v1/collections/{}", percent_encode(collection_id));
        self.http.send_json(Method::PUT, &path, request).await
    }

    /// Delete a collection by ID.
    ///
    /// `DELETE /v1/collections/{id}`
    pub async fn delete(&self, collection_id: &str) -> Result<(), TransportError> {
        let path = format!("/v1/collections/{}", percent_encode(collection_id));
        self.http.send_no_body_empty(Method::DELETE, &path).await
    }

    /// Access the Documents API client for a specific collection.
    pub fn documents(&self, collection_id: &str) -> DocumentsClient {
        DocumentsClient::new(Arc::clone(&self.http), collection_id.to_owned())
    }
}

/// Percent-encode a path segment using the same utility as other endpoint
/// clients. Uses proper path-segment encoding (%20 for spaces), not
/// form-encoding (+ for spaces).
fn percent_encode(value: &str) -> String {
    crate::endpoints::util::encode_path_segment(value)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn percent_encode_plain_id() {
        assert_eq!(percent_encode("col_abc123"), "col_abc123");
    }

    #[test]
    fn percent_encode_special_characters() {
        let encoded = percent_encode("col/with spaces&special=chars");
        assert!(!encoded.contains('/'));
        assert!(!encoded.contains(' '));
        assert!(!encoded.contains('&'));
        assert!(!encoded.contains('='));
    }
}

#[cfg(test)]
mod wiremock_tests {
    use std::sync::Arc;
    use std::time::Duration;

    use wiremock::matchers::{body_json, method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    use crate::transport::auth::ApiKeySecret;
    use crate::transport::client::{HttpClient, HttpClientConfig};
    use crate::transport::policy_gate::AllowAllGate;
    use crate::types::collections::{
        ChunkConfig, ChunkStrategy, Collection, CollectionList, CreateCollectionRequest,
        IndexConfiguration, MetricSpace,
    };

    use super::CollectionsClient;

    /// Build a `CollectionsClient` pointed at the wiremock server.
    fn build_client(base_url: &str) -> CollectionsClient {
        let config = HttpClientConfig {
            base_url: base_url.to_owned(),
            timeout: Duration::from_secs(5),
            ..Default::default()
        };
        let key = ApiKeySecret::new("test-management-key");
        let http = HttpClient::new(config, key, Some(Arc::new(AllowAllGate))).unwrap();
        CollectionsClient::new(Arc::new(http))
    }

    #[tokio::test]
    async fn create_collection_wiremock() {
        let server = MockServer::start().await;

        let response_body = serde_json::json!({
            "collection_id": "col_new_123",
            "collection_name": "my-new-collection",
            "collection_description": "A brand new collection",
            "embedding_model": "grok-embedding-small",
            "chunk_configuration": {
                "strategy": "tokens",
                "chunk_size": 512,
                "chunk_overlap": 64
            },
            "index_configuration": {
                "metric_space": "cosine"
            },
            "field_definitions": [],
            "documents_count": 0,
            "created_at": "2026-04-05T12:00:00Z"
        });

        let request_body = serde_json::json!({
            "collection_name": "my-new-collection",
            "collection_description": "A brand new collection",
            "embedding_model": "grok-embedding-small",
            "chunk_configuration": {
                "strategy": "tokens",
                "chunk_size": 512,
                "chunk_overlap": 64
            },
            "index_configuration": {
                "metric_space": "cosine"
            }
        });

        Mock::given(method("POST"))
            .and(path("/v1/collections"))
            .and(body_json(&request_body))
            .respond_with(ResponseTemplate::new(200).set_body_json(&response_body))
            .expect(1)
            .mount(&server)
            .await;

        let client = build_client(&server.uri());
        let req = CreateCollectionRequest {
            name: "my-new-collection".into(),
            description: Some("A brand new collection".into()),
            embedding_model: "grok-embedding-small".into(),
            chunk_configuration: Some(ChunkConfig {
                strategy: ChunkStrategy::Tokens,
                chunk_size: 512,
                chunk_overlap: 64,
            }),
            index_configuration: Some(IndexConfiguration {
                metric_space: MetricSpace::Cosine,
            }),
            field_definitions: vec![],
        };

        let collection = client.create(&req).await.unwrap();
        assert_eq!(collection.id, "col_new_123");
        assert_eq!(collection.name, "my-new-collection");
        assert_eq!(
            collection.description.as_deref(),
            Some("A brand new collection")
        );
        assert_eq!(collection.documents_count, Some(0));
        assert!(collection.chunk_configuration.is_some());
        assert!(collection.index_configuration.is_some());
    }

    #[tokio::test]
    async fn list_collections_wiremock() {
        let server = MockServer::start().await;

        let response_body = serde_json::json!({
            "collections": [
                {
                    "collection_id": "col_aaa",
                    "collection_name": "docs-collection",
                    "embedding_model": "grok-embedding-small",
                    "documents_count": 42,
                    "created_at": "2026-04-01T00:00:00Z"
                },
                {
                    "collection_id": "col_bbb",
                    "collection_name": "code-collection",
                    "collection_description": "Source code embeddings",
                    "embedding_model": "grok-embedding-small",
                    "chunk_configuration": {
                        "strategy": "code",
                        "chunk_size": 256,
                        "chunk_overlap": 32
                    },
                    "index_configuration": {
                        "metric_space": "inner_product"
                    },
                    "documents_count": 100,
                    "created_at": "2026-04-02T00:00:00Z",
                    "modified_at": "2026-04-05T10:00:00Z"
                }
            ]
        });

        Mock::given(method("GET"))
            .and(path("/v1/collections"))
            .respond_with(ResponseTemplate::new(200).set_body_json(&response_body))
            .expect(1)
            .mount(&server)
            .await;

        let client = build_client(&server.uri());
        let list: CollectionList = client.list().await.unwrap();

        assert_eq!(list.collections.len(), 2);

        let first = &list.collections[0];
        assert_eq!(first.id, "col_aaa");
        assert_eq!(first.name, "docs-collection");
        assert!(first.description.is_none());
        assert_eq!(first.documents_count, Some(42));

        let second = &list.collections[1];
        assert_eq!(second.id, "col_bbb");
        assert_eq!(second.name, "code-collection");
        assert_eq!(
            second.description.as_deref(),
            Some("Source code embeddings")
        );
        assert_eq!(second.documents_count, Some(100));
        assert_eq!(
            second.index_configuration.as_ref().unwrap().metric_space,
            MetricSpace::InnerProduct
        );
    }

    #[tokio::test]
    async fn get_collection_wiremock() {
        let server = MockServer::start().await;

        let response_body = serde_json::json!({
            "collection_id": "col_xyz",
            "collection_name": "single-collection",
            "embedding_model": "grok-embedding-small",
            "documents_count": 10,
            "created_at": "2026-04-05T00:00:00Z"
        });

        Mock::given(method("GET"))
            .and(path("/v1/collections/col_xyz"))
            .respond_with(ResponseTemplate::new(200).set_body_json(&response_body))
            .expect(1)
            .mount(&server)
            .await;

        let client = build_client(&server.uri());
        let collection: Collection = client.get("col_xyz").await.unwrap();

        assert_eq!(collection.id, "col_xyz");
        assert_eq!(collection.name, "single-collection");
        assert_eq!(collection.documents_count, Some(10));
    }

    #[tokio::test]
    async fn update_collection_wiremock() {
        let server = MockServer::start().await;

        let response_body = serde_json::json!({
            "collection_id": "col_upd",
            "collection_name": "renamed-collection",
            "collection_description": "Updated description",
            "embedding_model": "grok-embedding-small",
            "documents_count": 5,
            "modified_at": "2026-04-05T15:00:00Z"
        });

        Mock::given(method("PUT"))
            .and(path("/v1/collections/col_upd"))
            .respond_with(ResponseTemplate::new(200).set_body_json(&response_body))
            .expect(1)
            .mount(&server)
            .await;

        let client = build_client(&server.uri());
        let req = crate::types::collections::UpdateCollectionRequest {
            name: Some("renamed-collection".into()),
            description: Some("Updated description".into()),
            chunk_configuration: None,
            field_definitions: None,
        };
        let collection = client.update("col_upd", &req).await.unwrap();

        assert_eq!(collection.name, "renamed-collection");
        assert_eq!(
            collection.description.as_deref(),
            Some("Updated description")
        );
    }

    #[tokio::test]
    async fn delete_collection_wiremock() {
        let server = MockServer::start().await;

        Mock::given(method("DELETE"))
            .and(path("/v1/collections/col_del"))
            .respond_with(ResponseTemplate::new(204))
            .expect(1)
            .mount(&server)
            .await;

        let client = build_client(&server.uri());
        let result = client.delete("col_del").await;
        assert!(result.is_ok());
    }
}
