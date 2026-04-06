//! Document management endpoint client for the Management API.
//!
//! Provides operations for managing documents within a collection:
//! - `POST   /v1/collections/{id}/documents/{file_id}`                — add
//! - `GET    /v1/collections/{id}/documents`                          — list
//! - `GET    /v1/collections/{id}/documents/{file_id}`                — get
//! - `PATCH  /v1/collections/{id}/documents/{file_id}`                — regenerate
//! - `DELETE /v1/collections/{id}/documents/{file_id}`                — remove
//! - `GET    /v1/collections/{id}/documents:batchGet?file_ids=...`    — batch get

use std::sync::Arc;

use reqwest::Method;

use crate::transport::client::HttpClient;
use crate::transport::error::TransportError;
use crate::types::collection_documents::{
    AddDocumentRequest, BatchGetDocumentsResponse, CollectionDocument, DocumentList,
};

/// Client for document management within a specific collection.
///
/// Created via `CollectionsClient::documents(collection_id)`.
pub struct DocumentsClient {
    http: Arc<HttpClient>,
    collection_id: String,
}

impl DocumentsClient {
    /// Create a new `DocumentsClient` for the given collection.
    pub fn new(http: Arc<HttpClient>, collection_id: String) -> Self {
        Self {
            http,
            collection_id,
        }
    }

    /// Add a document (referenced by `file_id`) to this collection.
    ///
    /// `POST /v1/collections/{collection_id}/documents/{file_id}`
    ///
    /// The `file_id` references a file previously uploaded via the inference
    /// Files API. The Management API will chunk and embed the file contents.
    pub async fn add(
        &self,
        file_id: &str,
        request: &AddDocumentRequest,
    ) -> Result<CollectionDocument, TransportError> {
        let path = format!(
            "/v1/collections/{}/documents/{}",
            percent_encode(&self.collection_id),
            percent_encode(file_id),
        );
        self.http.send_json(Method::POST, &path, request).await
    }

    /// List documents in this collection.
    ///
    /// `GET /v1/collections/{collection_id}/documents`
    ///
    /// Supports optional query parameters for cursor-based pagination and
    /// status filtering. Pass `filter` as `None` to list all documents
    /// regardless of status. Use `pagination_token` from a previous
    /// `DocumentList` response to fetch the next page.
    pub async fn list(
        &self,
        filter: Option<&str>,
        limit: Option<u32>,
        pagination_token: Option<&str>,
    ) -> Result<DocumentList, TransportError> {
        let mut path = format!(
            "/v1/collections/{}/documents",
            percent_encode(&self.collection_id),
        );

        let mut params = Vec::new();
        if let Some(f) = filter {
            params.push(format!("filter={}", percent_encode(f)));
        }
        if let Some(l) = limit {
            params.push(format!("limit={l}"));
        }
        if let Some(t) = pagination_token {
            params.push(format!("pagination_token={}", percent_encode(t)));
        }
        if !params.is_empty() {
            path.push('?');
            path.push_str(&params.join("&"));
        }

        self.http.send_no_body(Method::GET, &path).await
    }

    /// Get a single document by file ID.
    ///
    /// `GET /v1/collections/{collection_id}/documents/{file_id}`
    pub async fn get(&self, file_id: &str) -> Result<CollectionDocument, TransportError> {
        let path = format!(
            "/v1/collections/{}/documents/{}",
            percent_encode(&self.collection_id),
            percent_encode(file_id),
        );
        self.http.send_no_body(Method::GET, &path).await
    }

    /// Regenerate (re-chunk and re-embed) a document.
    ///
    /// `PATCH /v1/collections/{collection_id}/documents/{file_id}`
    ///
    /// This triggers the Management API to reprocess the document using the
    /// collection's current chunk configuration.
    pub async fn regenerate(&self, file_id: &str) -> Result<CollectionDocument, TransportError> {
        let path = format!(
            "/v1/collections/{}/documents/{}",
            percent_encode(&self.collection_id),
            percent_encode(file_id),
        );
        // PATCH with empty body triggers regeneration.
        let empty: serde_json::Value = serde_json::json!({});
        self.http.send_json(Method::PATCH, &path, &empty).await
    }

    /// Remove a document from this collection.
    ///
    /// `DELETE /v1/collections/{collection_id}/documents/{file_id}`
    pub async fn remove(&self, file_id: &str) -> Result<(), TransportError> {
        let path = format!(
            "/v1/collections/{}/documents/{}",
            percent_encode(&self.collection_id),
            percent_encode(file_id),
        );
        self.http.send_no_body_empty(Method::DELETE, &path).await
    }

    /// Batch-get multiple documents by their file IDs.
    ///
    /// `GET /v1/collections/{collection_id}/documents:batchGet?file_ids=id1,id2,...`
    pub async fn batch_get(
        &self,
        file_ids: &[&str],
    ) -> Result<BatchGetDocumentsResponse, TransportError> {
        let encoded_ids: Vec<String> = file_ids.iter().map(|id| percent_encode(id)).collect();
        let path = format!(
            "/v1/collections/{}/documents:batchGet?file_ids={}",
            percent_encode(&self.collection_id),
            encoded_ids.join(","),
        );
        self.http.send_no_body(Method::GET, &path).await
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
    fn percent_encode_preserves_safe_ids() {
        assert_eq!(percent_encode("file_abc123"), "file_abc123");
        assert_eq!(percent_encode("col_xyz"), "col_xyz");
    }

    #[test]
    fn percent_encode_escapes_special_chars() {
        let encoded = percent_encode("id/with spaces");
        assert!(!encoded.contains('/'));
        assert!(!encoded.contains(' '));
    }
}

#[cfg(test)]
mod wiremock_tests {
    use std::collections::HashMap;
    use std::sync::Arc;
    use std::time::Duration;

    use wiremock::matchers::{method, path, query_param};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    use crate::transport::auth::ApiKeySecret;
    use crate::transport::client::{HttpClient, HttpClientConfig};
    use crate::transport::policy_gate::AllowAllGate;
    use crate::types::collection_documents::{AddDocumentRequest, DocumentStatus};

    use super::DocumentsClient;

    /// Build a `DocumentsClient` pointed at the wiremock server.
    fn build_client(base_url: &str, collection_id: &str) -> DocumentsClient {
        let config = HttpClientConfig {
            base_url: base_url.to_owned(),
            timeout: Duration::from_secs(5),
            ..Default::default()
        };
        let key = ApiKeySecret::new("test-management-key");
        let http = HttpClient::new(config, key, Some(Arc::new(AllowAllGate))).unwrap();
        DocumentsClient::new(Arc::new(http), collection_id.to_owned())
    }

    #[tokio::test]
    async fn add_document_wiremock() {
        let server = MockServer::start().await;

        let response_body = serde_json::json!({
            "file_id": "file_new_001",
            "status": "DOCUMENT_STATUS_PENDING",
            "name": "quarterly-report.pdf",
            "file_metadata": {
                "filename": "quarterly-report.pdf",
                "content_type": "application/pdf",
                "size_bytes": 1048576
            },
            "created_at": "2026-04-05T12:00:00Z"
        });

        Mock::given(method("POST"))
            .and(path("/v1/collections/col_test/documents/file_new_001"))
            .respond_with(ResponseTemplate::new(200).set_body_json(&response_body))
            .expect(1)
            .mount(&server)
            .await;

        let client = build_client(&server.uri(), "col_test");
        let req = AddDocumentRequest {
            name: Some("quarterly-report.pdf".into()),
            fields: HashMap::new(),
        };

        let doc = client.add("file_new_001", &req).await.unwrap();
        assert_eq!(doc.file_id, "file_new_001");
        assert_eq!(doc.status, DocumentStatus::Pending);
        assert_eq!(doc.name.as_deref(), Some("quarterly-report.pdf"));
        let meta = doc.file_metadata.unwrap();
        assert_eq!(meta.filename.as_deref(), Some("quarterly-report.pdf"));
        assert_eq!(meta.content_type.as_deref(), Some("application/pdf"));
        assert_eq!(meta.size_bytes, Some(1048576));
    }

    #[tokio::test]
    async fn list_documents_wiremock() {
        let server = MockServer::start().await;

        let response_body = serde_json::json!({
            "documents": [
                {
                    "file_id": "file_001",
                    "status": "DOCUMENT_STATUS_READY",
                    "name": "readme.md",
                    "file_metadata": {
                        "filename": "readme.md",
                        "content_type": "text/markdown",
                        "size_bytes": 4096
                    },
                    "created_at": "2026-04-01T00:00:00Z",
                    "last_indexed_at": "2026-04-01T00:05:00Z"
                },
                {
                    "file_id": "file_002",
                    "status": "DOCUMENT_STATUS_INDEXING",
                    "name": "data.csv",
                    "created_at": "2026-04-05T00:00:00Z"
                },
                {
                    "file_id": "file_003",
                    "status": "DOCUMENT_STATUS_FAILED",
                    "error_message": "unsupported file format: .bin",
                    "created_at": "2026-04-05T01:00:00Z"
                }
            ],
            "pagination_token": "tok_next_page_abc"
        });

        Mock::given(method("GET"))
            .and(path("/v1/collections/col_docs/documents"))
            .respond_with(ResponseTemplate::new(200).set_body_json(&response_body))
            .expect(1)
            .mount(&server)
            .await;

        let client = build_client(&server.uri(), "col_docs");
        let list = client.list(None, None, None).await.unwrap();

        assert_eq!(list.documents.len(), 3);
        assert_eq!(list.pagination_token.as_deref(), Some("tok_next_page_abc"));

        // Verify first document (ready)
        let first = &list.documents[0];
        assert_eq!(first.file_id, "file_001");
        assert_eq!(first.status, DocumentStatus::Ready);
        assert_eq!(first.name.as_deref(), Some("readme.md"));
        assert!(first.file_metadata.is_some());
        assert_eq!(
            first.last_indexed_at.as_deref(),
            Some("2026-04-01T00:05:00Z")
        );

        // Verify second document (indexing)
        let second = &list.documents[1];
        assert_eq!(second.status, DocumentStatus::Indexing);

        // Verify third document (failed with error)
        let third = &list.documents[2];
        assert_eq!(third.status, DocumentStatus::Failed);
        assert_eq!(
            third.error_message.as_deref(),
            Some("unsupported file format: .bin")
        );
    }

    #[tokio::test]
    async fn list_documents_with_filter_and_pagination_token_wiremock() {
        let server = MockServer::start().await;

        let response_body = serde_json::json!({
            "documents": [
                {
                    "file_id": "file_page2_001",
                    "status": "DOCUMENT_STATUS_READY",
                    "created_at": "2026-04-03T00:00:00Z"
                }
            ]
        });

        Mock::given(method("GET"))
            .and(path("/v1/collections/col_paged/documents"))
            .and(query_param("filter", "status=ready"))
            .and(query_param("limit", "10"))
            .and(query_param("pagination_token", "tok_page2"))
            .respond_with(ResponseTemplate::new(200).set_body_json(&response_body))
            .expect(1)
            .mount(&server)
            .await;

        let client = build_client(&server.uri(), "col_paged");
        let list = client
            .list(Some("status=ready"), Some(10), Some("tok_page2"))
            .await
            .unwrap();

        assert_eq!(list.documents.len(), 1);
        assert_eq!(list.documents[0].file_id, "file_page2_001");
        // No pagination_token means last page
        assert!(list.pagination_token.is_none());
    }

    #[tokio::test]
    async fn get_document_wiremock() {
        let server = MockServer::start().await;

        let response_body = serde_json::json!({
            "file_id": "file_get_001",
            "status": "DOCUMENT_STATUS_READY",
            "name": "specific-doc.txt",
            "fields": {"category": "engineering"},
            "file_metadata": {
                "filename": "specific-doc.txt",
                "content_type": "text/plain",
                "size_bytes": 512
            },
            "created_at": "2026-04-05T00:00:00Z",
            "last_indexed_at": "2026-04-05T00:01:00Z"
        });

        Mock::given(method("GET"))
            .and(path("/v1/collections/col_get/documents/file_get_001"))
            .respond_with(ResponseTemplate::new(200).set_body_json(&response_body))
            .expect(1)
            .mount(&server)
            .await;

        let client = build_client(&server.uri(), "col_get");
        let doc = client.get("file_get_001").await.unwrap();

        assert_eq!(doc.file_id, "file_get_001");
        assert_eq!(doc.status, DocumentStatus::Ready);
        assert_eq!(doc.fields.get("category").unwrap(), "engineering");
        assert!(doc.file_metadata.is_some());
        assert!(doc.last_indexed_at.is_some());
    }

    #[tokio::test]
    async fn remove_document_wiremock() {
        let server = MockServer::start().await;

        Mock::given(method("DELETE"))
            .and(path("/v1/collections/col_rm/documents/file_rm_001"))
            .respond_with(ResponseTemplate::new(204))
            .expect(1)
            .mount(&server)
            .await;

        let client = build_client(&server.uri(), "col_rm");
        let result = client.remove("file_rm_001").await;
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn regenerate_document_wiremock() {
        let server = MockServer::start().await;

        let response_body = serde_json::json!({
            "file_id": "file_regen",
            "status": "DOCUMENT_STATUS_INDEXING",
            "name": "re-indexed.md",
            "created_at": "2026-04-01T00:00:00Z",
            "modified_at": "2026-04-05T15:00:00Z"
        });

        Mock::given(method("PATCH"))
            .and(path("/v1/collections/col_regen/documents/file_regen"))
            .respond_with(ResponseTemplate::new(200).set_body_json(&response_body))
            .expect(1)
            .mount(&server)
            .await;

        let client = build_client(&server.uri(), "col_regen");
        let doc = client.regenerate("file_regen").await.unwrap();

        assert_eq!(doc.file_id, "file_regen");
        assert_eq!(doc.status, DocumentStatus::Indexing);
    }
}
