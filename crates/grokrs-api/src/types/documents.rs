//! Wire types for the xAI Document Search API.
//!
//! These types map directly to the JSON request/response bodies of the
//! `POST /v1/documents/search` endpoint. Document search is the primary
//! consumer-facing Collections endpoint, enabling semantic RAG workflows
//! using the standard inference API key.

use std::collections::HashMap;

use serde::{Deserialize, Serialize};

// ---------------------------------------------------------------------------
// Enums
// ---------------------------------------------------------------------------

/// Ranking metric used to score document matches.
///
/// Controls the distance function applied to embedding vectors when ranking
/// search results. Each variant serializes to the exact lowercase string
/// expected by the xAI API.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum RankingMetric {
    /// L2 (Euclidean) distance.
    #[serde(rename = "l2")]
    L2,
    /// Cosine similarity.
    #[serde(rename = "cosine")]
    Cosine,
}

// ---------------------------------------------------------------------------
// Request types
// ---------------------------------------------------------------------------

/// Source specification for document search, containing collection IDs.
///
/// The xAI API expects collection IDs nested under a `source` object rather
/// than as a flat top-level array.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DocumentsSource {
    /// IDs of the collections to search. Must contain at least one entry.
    pub collection_ids: Vec<String>,
}

/// Retrieval mode specification for document search.
///
/// The xAI API expects retrieval mode as an object `{"type": "hybrid"}`
/// rather than a bare string.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct RetrievalMode {
    /// The retrieval mode type (e.g., `"semantic"`, `"keyword"`, `"hybrid"`).
    #[serde(rename = "type")]
    pub mode_type: String,
}

/// Request body for `POST /v1/documents/search`.
///
/// Sends a semantic query against one or more document collections. The query
/// is embedded server-side and matched against chunked document vectors.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DocumentSearchRequest {
    /// The natural-language search query.
    pub query: String,

    /// Source specification containing the collection IDs to search.
    pub source: DocumentsSource,

    /// AIP-160 filter expression for metadata-based filtering.
    ///
    /// The filter grammar is complex; typed parsing is deferred. The string
    /// is passed through to the API verbatim.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub filter: Option<String>,

    /// Field name to group results by.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub group_by: Option<String>,

    /// The distance metric to use for ranking matches.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ranking_metric: Option<RankingMetric>,

    /// Retrieval mode specification (e.g., `{"type": "hybrid"}`).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub retrieval_mode: Option<RetrievalMode>,

    /// Maximum number of matches to return.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub limit: Option<u32>,
}

// ---------------------------------------------------------------------------
// Response types
// ---------------------------------------------------------------------------

/// Response body from `POST /v1/documents/search`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DocumentSearchResponse {
    /// Ranked list of matching document chunks.
    pub matches: Vec<SearchMatch>,
}

/// A single matching document chunk returned by a search query.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SearchMatch {
    /// The file ID containing the matched chunk.
    pub file_id: String,

    /// The chunk ID within the file.
    pub chunk_id: String,

    /// The textual content of the matched chunk.
    pub chunk_content: String,

    /// The similarity/distance score. This is a similarity score, not a
    /// monetary value, so `f64` is appropriate.
    pub score: f64,

    /// IDs of the collections this chunk belongs to.
    pub collection_ids: Vec<String>,

    /// User-defined metadata fields associated with this chunk.
    ///
    /// Uses `serde_json::Value` to accommodate arbitrary field types without
    /// requiring a typed schema.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub fields: Option<HashMap<String, serde_json::Value>>,

    /// The page number within the source document, if applicable.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub page_number: Option<u32>,
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ranking_metric_l2_serializes() {
        let json = serde_json::to_string(&RankingMetric::L2).unwrap();
        assert_eq!(json, "\"l2\"");
    }

    #[test]
    fn ranking_metric_cosine_serializes() {
        let json = serde_json::to_string(&RankingMetric::Cosine).unwrap();
        assert_eq!(json, "\"cosine\"");
    }

    #[test]
    fn ranking_metric_l2_deserializes() {
        let metric: RankingMetric = serde_json::from_str("\"l2\"").unwrap();
        assert_eq!(metric, RankingMetric::L2);
    }

    #[test]
    fn ranking_metric_cosine_deserializes() {
        let metric: RankingMetric = serde_json::from_str("\"cosine\"").unwrap();
        assert_eq!(metric, RankingMetric::Cosine);
    }

    #[test]
    fn request_round_trip_all_fields() {
        let req = DocumentSearchRequest {
            query: "What is the capital of France?".to_string(),
            source: DocumentsSource {
                collection_ids: vec!["col-1".to_string(), "col-2".to_string()],
            },
            filter: Some("metadata.language = \"en\"".to_string()),
            group_by: Some("file_id".to_string()),
            ranking_metric: Some(RankingMetric::Cosine),
            retrieval_mode: Some(RetrievalMode {
                mode_type: "hybrid".to_string(),
            }),
            limit: Some(10),
        };
        let json = serde_json::to_string(&req).unwrap();
        let deserialized: DocumentSearchRequest = serde_json::from_str(&json).unwrap();
        assert_eq!(req, deserialized);
    }

    #[test]
    fn request_round_trip_minimal() {
        let req = DocumentSearchRequest {
            query: "search query".to_string(),
            source: DocumentsSource {
                collection_ids: vec!["col-1".to_string()],
            },
            filter: None,
            group_by: None,
            ranking_metric: None,
            retrieval_mode: None,
            limit: None,
        };
        let json = serde_json::to_string(&req).unwrap();
        let deserialized: DocumentSearchRequest = serde_json::from_str(&json).unwrap();
        assert_eq!(req, deserialized);
    }

    #[test]
    fn request_skips_none_fields() {
        let req = DocumentSearchRequest {
            query: "test".to_string(),
            source: DocumentsSource {
                collection_ids: vec!["col-1".to_string()],
            },
            filter: None,
            group_by: None,
            ranking_metric: None,
            retrieval_mode: None,
            limit: None,
        };
        let json = serde_json::to_string(&req).unwrap();
        assert!(!json.contains("\"filter\""));
        assert!(!json.contains("\"group_by\""));
        assert!(!json.contains("\"ranking_metric\""));
        assert!(!json.contains("\"retrieval_mode\""));
        assert!(!json.contains("\"limit\""));
    }

    #[test]
    fn response_round_trip() {
        let mut fields = HashMap::new();
        fields.insert(
            "source".to_string(),
            serde_json::Value::String("wiki".to_string()),
        );
        fields.insert("relevance".to_string(), serde_json::json!(0.95));

        let resp = DocumentSearchResponse {
            matches: vec![
                SearchMatch {
                    file_id: "file-abc".to_string(),
                    chunk_id: "chunk-001".to_string(),
                    chunk_content: "Paris is the capital of France.".to_string(),
                    score: 0.987,
                    collection_ids: vec!["col-1".to_string()],
                    fields: Some(fields),
                    page_number: Some(42),
                },
                SearchMatch {
                    file_id: "file-def".to_string(),
                    chunk_id: "chunk-002".to_string(),
                    chunk_content: "France is a country in Europe.".to_string(),
                    score: 0.654,
                    collection_ids: vec!["col-1".to_string(), "col-2".to_string()],
                    fields: None,
                    page_number: None,
                },
            ],
        };
        let json = serde_json::to_string(&resp).unwrap();
        let deserialized: DocumentSearchResponse = serde_json::from_str(&json).unwrap();
        assert_eq!(resp, deserialized);
    }

    #[test]
    fn response_empty_matches() {
        let resp = DocumentSearchResponse { matches: vec![] };
        let json = serde_json::to_string(&resp).unwrap();
        let deserialized: DocumentSearchResponse = serde_json::from_str(&json).unwrap();
        assert!(deserialized.matches.is_empty());
    }

    #[test]
    fn search_match_skips_none_fields() {
        let m = SearchMatch {
            file_id: "f1".to_string(),
            chunk_id: "c1".to_string(),
            chunk_content: "text".to_string(),
            score: 0.5,
            collection_ids: vec!["col-1".to_string()],
            fields: None,
            page_number: None,
        };
        let json = serde_json::to_string(&m).unwrap();
        assert!(!json.contains("\"fields\""));
        assert!(!json.contains("\"page_number\""));
    }

    #[test]
    fn search_match_with_all_fields() {
        let mut fields = HashMap::new();
        fields.insert("key".to_string(), serde_json::json!("value"));

        let m = SearchMatch {
            file_id: "f1".to_string(),
            chunk_id: "c1".to_string(),
            chunk_content: "text".to_string(),
            score: 0.99,
            collection_ids: vec!["col-1".to_string()],
            fields: Some(fields),
            page_number: Some(1),
        };
        let json = serde_json::to_string(&m).unwrap();
        assert!(json.contains("\"fields\""));
        assert!(json.contains("\"page_number\""));
    }

    #[test]
    fn response_deserializes_from_api_like_json() {
        let json = r#"{
            "matches": [
                {
                    "file_id": "file-123",
                    "chunk_id": "chunk-456",
                    "chunk_content": "The quick brown fox jumps over the lazy dog.",
                    "score": 0.876,
                    "collection_ids": ["col-abc"],
                    "fields": {"author": "Jane Doe", "year": 2024},
                    "page_number": 7
                }
            ]
        }"#;
        let resp: DocumentSearchResponse = serde_json::from_str(json).unwrap();
        assert_eq!(resp.matches.len(), 1);
        let m = &resp.matches[0];
        assert_eq!(m.file_id, "file-123");
        assert_eq!(m.chunk_id, "chunk-456");
        assert!((m.score - 0.876).abs() < f64::EPSILON);
        assert_eq!(m.page_number, Some(7));
        let fields = m.fields.as_ref().unwrap();
        assert_eq!(fields["author"], serde_json::json!("Jane Doe"));
        assert_eq!(fields["year"], serde_json::json!(2024));
    }

    #[test]
    fn response_tolerates_unknown_fields() {
        let json = r#"{
            "matches": [
                {
                    "file_id": "f1",
                    "chunk_id": "c1",
                    "chunk_content": "text",
                    "score": 0.5,
                    "collection_ids": ["col-1"],
                    "unknown_field": "should be ignored"
                }
            ],
            "extra_top_level": true
        }"#;
        let resp: DocumentSearchResponse = serde_json::from_str(json).unwrap();
        assert_eq!(resp.matches.len(), 1);
    }

    #[test]
    fn ranking_metric_rejects_invalid_string() {
        let result = serde_json::from_str::<RankingMetric>("\"euclidean\"");
        assert!(result.is_err());
    }

    #[test]
    fn request_serializes_source_with_collection_ids() {
        let req = DocumentSearchRequest {
            query: "test".to_string(),
            source: DocumentsSource {
                collection_ids: vec!["a".to_string(), "b".to_string(), "c".to_string()],
            },
            filter: None,
            group_by: None,
            ranking_metric: None,
            retrieval_mode: None,
            limit: None,
        };
        let json = serde_json::to_string(&req).unwrap();
        // collection_ids must be nested under "source"
        assert!(json.contains("\"source\":{\"collection_ids\":[\"a\",\"b\",\"c\"]}"));
    }

    #[test]
    fn retrieval_mode_serializes_as_object_with_type() {
        let mode = RetrievalMode {
            mode_type: "hybrid".to_string(),
        };
        let json = serde_json::to_string(&mode).unwrap();
        assert_eq!(json, r#"{"type":"hybrid"}"#);
    }

    #[test]
    fn retrieval_mode_round_trips() {
        let mode = RetrievalMode {
            mode_type: "semantic".to_string(),
        };
        let json = serde_json::to_string(&mode).unwrap();
        let deserialized: RetrievalMode = serde_json::from_str(&json).unwrap();
        assert_eq!(mode, deserialized);
    }

    #[test]
    fn documents_source_round_trips() {
        let source = DocumentsSource {
            collection_ids: vec!["col-1".to_string(), "col-2".to_string()],
        };
        let json = serde_json::to_string(&source).unwrap();
        let deserialized: DocumentsSource = serde_json::from_str(&json).unwrap();
        assert_eq!(source, deserialized);
    }

    #[test]
    fn score_preserves_f64_precision() {
        let m = SearchMatch {
            file_id: "f1".to_string(),
            chunk_id: "c1".to_string(),
            chunk_content: "text".to_string(),
            score: 0.123_456_789_012_345_6,
            collection_ids: vec!["col-1".to_string()],
            fields: None,
            page_number: None,
        };
        let json = serde_json::to_string(&m).unwrap();
        let deserialized: SearchMatch = serde_json::from_str(&json).unwrap();
        assert!((m.score - deserialized.score).abs() < f64::EPSILON);
    }
}
