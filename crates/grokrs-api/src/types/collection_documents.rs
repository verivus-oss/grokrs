//! Wire types for document management within collections.
//!
//! Documents reference files uploaded via the inference Files API and are
//! processed (chunked, embedded) by the Management API when added to a
//! collection.

use std::collections::HashMap;

use serde::{Deserialize, Serialize};

/// Processing status of a document within a collection.
///
/// Wire values use the xAI `DOCUMENT_STATUS_*` convention (e.g.,
/// `"DOCUMENT_STATUS_PENDING"`). Rust variant names are idiomatic and serde
/// rename attributes bridge the gap.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum DocumentStatus {
    /// The document has been added but processing has not started.
    #[serde(rename = "DOCUMENT_STATUS_PENDING")]
    Pending,
    /// The document is being chunked and embedded.
    #[serde(rename = "DOCUMENT_STATUS_INDEXING")]
    Indexing,
    /// Processing is complete and the document is searchable.
    #[serde(rename = "DOCUMENT_STATUS_READY")]
    Ready,
    /// Processing failed. Check error details on the document.
    #[serde(rename = "DOCUMENT_STATUS_FAILED")]
    Failed,
}

/// File metadata returned by the Management API for a document.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct FileMetadata {
    /// Original file name.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub filename: Option<String>,
    /// MIME type of the file.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub content_type: Option<String>,
    /// File size in bytes.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub size_bytes: Option<u64>,
}

/// A document within a collection.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct CollectionDocument {
    /// The file ID referencing a file uploaded via the inference Files API.
    pub file_id: String,
    /// The processing status of this document.
    pub status: DocumentStatus,
    /// Human-readable name of the document.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    /// Structured metadata fields attached to this document.
    #[serde(default, skip_serializing_if = "HashMap::is_empty")]
    pub fields: HashMap<String, String>,
    /// Metadata about the underlying file (name, content type, size).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub file_metadata: Option<FileMetadata>,
    /// ISO 8601 timestamp when the document was added.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub created_at: Option<String>,
    /// ISO 8601 timestamp when the document was last modified.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub modified_at: Option<String>,
    /// ISO 8601 timestamp when the document was last indexed.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_indexed_at: Option<String>,
    /// Error message if status is `Failed`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error_message: Option<String>,
}

/// Request body for adding a document to a collection.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct AddDocumentRequest {
    /// Optional human-readable name for the document.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    /// Optional structured metadata fields.
    #[serde(default, skip_serializing_if = "HashMap::is_empty")]
    pub fields: HashMap<String, String>,
}

/// Response for listing documents in a collection.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct DocumentList {
    /// The list of documents.
    #[serde(default)]
    pub documents: Vec<CollectionDocument>,
    /// Opaque token for fetching the next page of results.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub pagination_token: Option<String>,
}

/// Request body for batch-getting documents.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct BatchGetDocumentsRequest {
    /// File IDs to retrieve.
    pub file_ids: Vec<String>,
}

/// Response for batch-getting documents.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct BatchGetDocumentsResponse {
    /// The retrieved documents.
    #[serde(default)]
    pub documents: Vec<CollectionDocument>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn document_status_serde_round_trip() {
        for (variant, expected_json) in [
            (DocumentStatus::Pending, "\"DOCUMENT_STATUS_PENDING\""),
            (DocumentStatus::Indexing, "\"DOCUMENT_STATUS_INDEXING\""),
            (DocumentStatus::Ready, "\"DOCUMENT_STATUS_READY\""),
            (DocumentStatus::Failed, "\"DOCUMENT_STATUS_FAILED\""),
        ] {
            let json = serde_json::to_string(&variant).unwrap();
            assert_eq!(
                json, expected_json,
                "serialization mismatch for {variant:?}"
            );
            let parsed: DocumentStatus = serde_json::from_str(&json).unwrap();
            assert_eq!(parsed, variant, "round-trip mismatch for {variant:?}");
        }
    }

    #[test]
    fn collection_document_serde_round_trip() {
        let doc = CollectionDocument {
            file_id: "file_abc123".into(),
            status: DocumentStatus::Ready,
            name: Some("README.md".into()),
            fields: {
                let mut m = HashMap::new();
                m.insert("source".into(), "github".into());
                m
            },
            file_metadata: Some(FileMetadata {
                filename: Some("README.md".into()),
                content_type: Some("text/markdown".into()),
                size_bytes: Some(4096),
            }),
            created_at: Some("2026-04-05T12:00:00Z".into()),
            modified_at: None,
            last_indexed_at: Some("2026-04-05T12:05:00Z".into()),
            error_message: None,
        };
        let json = serde_json::to_string(&doc).unwrap();
        let parsed: CollectionDocument = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed, doc);
    }

    #[test]
    fn collection_document_minimal() {
        let json = r#"{"file_id":"file_1","status":"DOCUMENT_STATUS_PENDING"}"#;
        let parsed: CollectionDocument = serde_json::from_str(json).unwrap();
        assert_eq!(parsed.file_id, "file_1");
        assert_eq!(parsed.status, DocumentStatus::Pending);
        assert!(parsed.name.is_none());
        assert!(parsed.fields.is_empty());
        assert!(parsed.file_metadata.is_none());
        assert!(parsed.created_at.is_none());
        assert!(parsed.last_indexed_at.is_none());
        assert!(parsed.error_message.is_none());
    }

    #[test]
    fn collection_document_failed_with_error() {
        let json = r#"{
            "file_id": "file_bad",
            "status": "DOCUMENT_STATUS_FAILED",
            "error_message": "unsupported file format"
        }"#;
        let parsed: CollectionDocument = serde_json::from_str(json).unwrap();
        assert_eq!(parsed.status, DocumentStatus::Failed);
        assert_eq!(
            parsed.error_message.as_deref(),
            Some("unsupported file format")
        );
    }

    #[test]
    fn add_document_request_serde_round_trip() {
        let req = AddDocumentRequest {
            name: Some("report.pdf".into()),
            fields: {
                let mut m = HashMap::new();
                m.insert("category".into(), "finance".into());
                m
            },
        };
        let json = serde_json::to_string(&req).unwrap();
        let parsed: AddDocumentRequest = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed, req);
    }

    #[test]
    fn add_document_request_empty() {
        let req = AddDocumentRequest {
            name: None,
            fields: HashMap::new(),
        };
        let json = serde_json::to_string(&req).unwrap();
        // Both optional/empty fields should be absent
        assert_eq!(json, "{}");
    }

    #[test]
    fn document_list_serde_round_trip() {
        let list = DocumentList {
            documents: vec![CollectionDocument {
                file_id: "file_1".into(),
                status: DocumentStatus::Ready,
                name: None,
                fields: HashMap::new(),
                file_metadata: None,
                created_at: None,
                modified_at: None,
                last_indexed_at: None,
                error_message: None,
            }],
            pagination_token: Some("next_page_abc".into()),
        };
        let json = serde_json::to_string(&list).unwrap();
        let parsed: DocumentList = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed, list);
        assert!(json.contains("\"pagination_token\""));
    }

    #[test]
    fn document_list_empty() {
        let json = r#"{"documents":[]}"#;
        let parsed: DocumentList = serde_json::from_str(json).unwrap();
        assert!(parsed.documents.is_empty());
        assert!(parsed.pagination_token.is_none());
    }

    #[test]
    fn document_list_with_pagination_token() {
        let json = r#"{"documents":[],"pagination_token":"tok_abc123"}"#;
        let parsed: DocumentList = serde_json::from_str(json).unwrap();
        assert!(parsed.documents.is_empty());
        assert_eq!(parsed.pagination_token.as_deref(), Some("tok_abc123"));
    }

    #[test]
    fn batch_get_documents_request_serde() {
        let req = BatchGetDocumentsRequest {
            file_ids: vec!["file_1".into(), "file_2".into()],
        };
        let json = serde_json::to_string(&req).unwrap();
        let parsed: BatchGetDocumentsRequest = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed, req);
    }

    #[test]
    fn batch_get_documents_response_serde() {
        let resp = BatchGetDocumentsResponse {
            documents: vec![CollectionDocument {
                file_id: "file_1".into(),
                status: DocumentStatus::Indexing,
                name: None,
                fields: HashMap::new(),
                file_metadata: None,
                created_at: None,
                modified_at: None,
                last_indexed_at: None,
                error_message: None,
            }],
        };
        let json = serde_json::to_string(&resp).unwrap();
        let parsed: BatchGetDocumentsResponse = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed, resp);
    }

    #[test]
    fn file_metadata_serde_round_trip() {
        let meta = FileMetadata {
            filename: Some("data.csv".into()),
            content_type: Some("text/csv".into()),
            size_bytes: Some(1048576),
        };
        let json = serde_json::to_string(&meta).unwrap();
        let parsed: FileMetadata = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed, meta);
    }

    #[test]
    fn file_metadata_minimal() {
        let json = "{}";
        let parsed: FileMetadata = serde_json::from_str(json).unwrap();
        assert!(parsed.filename.is_none());
        assert!(parsed.content_type.is_none());
        assert!(parsed.size_bytes.is_none());
    }

    #[test]
    fn collection_document_from_api_shape() {
        let json = r#"{
            "file_id": "file_xyz",
            "status": "DOCUMENT_STATUS_READY",
            "name": "quarterly-report.pdf",
            "fields": {"department": "finance"},
            "file_metadata": {
                "filename": "quarterly-report.pdf",
                "content_type": "application/pdf",
                "size_bytes": 2048000
            },
            "created_at": "2026-04-01T00:00:00Z",
            "modified_at": "2026-04-02T00:00:00Z",
            "last_indexed_at": "2026-04-02T00:05:00Z"
        }"#;
        let parsed: CollectionDocument = serde_json::from_str(json).unwrap();
        assert_eq!(parsed.file_id, "file_xyz");
        assert_eq!(parsed.status, DocumentStatus::Ready);
        assert_eq!(parsed.name.as_deref(), Some("quarterly-report.pdf"));
        assert_eq!(parsed.fields.get("department").unwrap(), "finance");
        let meta = parsed.file_metadata.unwrap();
        assert_eq!(meta.filename.as_deref(), Some("quarterly-report.pdf"));
        assert_eq!(meta.content_type.as_deref(), Some("application/pdf"));
        assert_eq!(meta.size_bytes, Some(2048000));
        assert_eq!(
            parsed.last_indexed_at.as_deref(),
            Some("2026-04-02T00:05:00Z")
        );
        assert!(parsed.error_message.is_none());
    }
}
