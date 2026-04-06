//! Wire types for the xAI Collections Management API.
//!
//! These types map 1:1 to the JSON shapes accepted and returned by the
//! Management API endpoints at `https://management-api.x.ai/v1/collections`.

use serde::{Deserialize, Serialize};

/// Strategy for splitting documents into chunks for embedding.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ChunkStrategy {
    /// Split by character count.
    Chars,
    /// Split by token count.
    Tokens,
    /// Split at markdown heading boundaries.
    Markdown,
    /// Split at code-aware boundaries (function, class, etc.).
    Code,
    /// Split by raw byte count.
    Bytes,
}

/// Metric space used for vector similarity search.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MetricSpace {
    /// Cosine similarity.
    Cosine,
    /// Euclidean (L2) distance.
    Euclidean,
    /// Inner product similarity.
    InnerProduct,
}

/// Configuration for how documents are chunked before embedding.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ChunkConfig {
    /// The chunking strategy to use.
    pub strategy: ChunkStrategy,
    /// Target size for each chunk (in the unit implied by `strategy`).
    pub chunk_size: u64,
    /// Number of units of overlap between consecutive chunks.
    pub chunk_overlap: u64,
}

/// Configuration for the vector index used by a collection.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct IndexConfiguration {
    /// The metric space used for similarity search.
    pub metric_space: MetricSpace,
}

/// A field definition attached to a collection.
///
/// Field definitions describe structured metadata fields that can be attached
/// to documents in the collection.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct FieldDefinition {
    /// The name of the field.
    pub name: String,
    /// Whether this field is required on every document.
    #[serde(default)]
    pub required: bool,
    /// Whether values for this field must be unique across the collection.
    #[serde(default)]
    pub unique: bool,
    /// Whether field values should be injected into the chunk text before
    /// embedding. This can improve retrieval relevance for metadata-heavy
    /// use cases.
    #[serde(default)]
    pub inject_into_chunk: bool,
}

/// A collection resource returned by the Management API.
///
/// Wire field names use the xAI convention (`collection_id`, `collection_name`,
/// `collection_description`, `documents_count`). Rust field names are idiomatic
/// where it makes sense and serde renames bridge the gap.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Collection {
    /// Unique identifier for the collection.
    #[serde(rename = "collection_id")]
    pub id: String,
    /// Human-readable name of the collection.
    #[serde(rename = "collection_name")]
    pub name: String,
    /// Optional description of the collection.
    #[serde(
        rename = "collection_description",
        default,
        skip_serializing_if = "Option::is_none"
    )]
    pub description: Option<String>,
    /// The embedding model used for this collection.
    pub embedding_model: String,
    /// Chunk configuration for document processing.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub chunk_configuration: Option<ChunkConfig>,
    /// Index configuration (metric space, etc.).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub index_configuration: Option<IndexConfiguration>,
    /// Field definitions for structured metadata.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub field_definitions: Vec<FieldDefinition>,
    /// Number of documents in the collection.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub documents_count: Option<u64>,
    /// ISO 8601 timestamp when the collection was created.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub created_at: Option<String>,
    /// ISO 8601 timestamp when the collection was last modified.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub modified_at: Option<String>,
}

/// Request body for creating a new collection.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct CreateCollectionRequest {
    /// Human-readable name for the collection.
    #[serde(rename = "collection_name")]
    pub name: String,
    /// Optional description for the collection.
    #[serde(
        rename = "collection_description",
        default,
        skip_serializing_if = "Option::is_none"
    )]
    pub description: Option<String>,
    /// The embedding model to use (e.g., `"grok-embedding-small"`).
    pub embedding_model: String,
    /// Optional chunk configuration. Uses server defaults if not specified.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub chunk_configuration: Option<ChunkConfig>,
    /// Optional index configuration. Uses server default (cosine) if not specified.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub index_configuration: Option<IndexConfiguration>,
    /// Optional field definitions for structured metadata.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub field_definitions: Vec<FieldDefinition>,
}

/// Request body for updating an existing collection.
///
/// All fields are optional; only provided fields are updated.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct UpdateCollectionRequest {
    /// New name for the collection.
    #[serde(
        rename = "collection_name",
        default,
        skip_serializing_if = "Option::is_none"
    )]
    pub name: Option<String>,
    /// New description for the collection.
    #[serde(
        rename = "collection_description",
        default,
        skip_serializing_if = "Option::is_none"
    )]
    pub description: Option<String>,
    /// Updated chunk configuration.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub chunk_configuration: Option<ChunkConfig>,
    /// Updated field definitions.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub field_definitions: Option<Vec<FieldDefinition>>,
}

/// Response for listing collections.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct CollectionList {
    /// The list of collections.
    #[serde(default)]
    pub collections: Vec<Collection>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn chunk_strategy_serde_round_trip() {
        for (variant, expected_json) in [
            (ChunkStrategy::Chars, "\"chars\""),
            (ChunkStrategy::Tokens, "\"tokens\""),
            (ChunkStrategy::Markdown, "\"markdown\""),
            (ChunkStrategy::Code, "\"code\""),
            (ChunkStrategy::Bytes, "\"bytes\""),
        ] {
            let json = serde_json::to_string(&variant).unwrap();
            assert_eq!(
                json, expected_json,
                "serialization mismatch for {variant:?}"
            );
            let parsed: ChunkStrategy = serde_json::from_str(&json).unwrap();
            assert_eq!(parsed, variant, "round-trip mismatch for {variant:?}");
        }
    }

    #[test]
    fn metric_space_serde_round_trip() {
        for (variant, expected_json) in [
            (MetricSpace::Cosine, "\"cosine\""),
            (MetricSpace::Euclidean, "\"euclidean\""),
            (MetricSpace::InnerProduct, "\"inner_product\""),
        ] {
            let json = serde_json::to_string(&variant).unwrap();
            assert_eq!(
                json, expected_json,
                "serialization mismatch for {variant:?}"
            );
            let parsed: MetricSpace = serde_json::from_str(&json).unwrap();
            assert_eq!(parsed, variant, "round-trip mismatch for {variant:?}");
        }
    }

    #[test]
    fn collection_serde_round_trip() {
        let collection = Collection {
            id: "col_abc123".into(),
            name: "My RAG Collection".into(),
            description: Some("A test collection for RAG workflows".into()),
            embedding_model: "grok-embedding-small".into(),
            chunk_configuration: Some(ChunkConfig {
                strategy: ChunkStrategy::Tokens,
                chunk_size: 512,
                chunk_overlap: 64,
            }),
            index_configuration: Some(IndexConfiguration {
                metric_space: MetricSpace::Cosine,
            }),
            field_definitions: vec![FieldDefinition {
                name: "source".into(),
                required: true,
                unique: false,
                inject_into_chunk: true,
            }],
            documents_count: Some(42),
            created_at: Some("2026-04-05T12:00:00Z".into()),
            modified_at: None,
        };
        let json = serde_json::to_string(&collection).unwrap();
        let parsed: Collection = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed, collection);
    }

    #[test]
    fn collection_wire_field_names() {
        let collection = Collection {
            id: "col_1".into(),
            name: "test".into(),
            description: Some("desc".into()),
            embedding_model: "grok-embedding-small".into(),
            chunk_configuration: None,
            index_configuration: None,
            field_definitions: vec![],
            documents_count: Some(5),
            created_at: None,
            modified_at: None,
        };
        let json = serde_json::to_string(&collection).unwrap();
        // Verify wire names
        assert!(json.contains("\"collection_id\""), "missing collection_id");
        assert!(
            json.contains("\"collection_name\""),
            "missing collection_name"
        );
        assert!(
            json.contains("\"collection_description\""),
            "missing collection_description"
        );
        assert!(
            json.contains("\"documents_count\""),
            "missing documents_count"
        );
        // Verify old names are NOT present
        assert!(
            !json.contains("\"id\""),
            "should not contain bare \"id\" key"
        );
        assert!(
            !json.contains("\"name\""),
            "should not contain bare \"name\" key"
        );
    }

    #[test]
    fn create_collection_request_serde_round_trip() {
        let req = CreateCollectionRequest {
            name: "test-collection".into(),
            description: Some("A test collection".into()),
            embedding_model: "grok-embedding-small".into(),
            chunk_configuration: Some(ChunkConfig {
                strategy: ChunkStrategy::Chars,
                chunk_size: 1024,
                chunk_overlap: 128,
            }),
            index_configuration: Some(IndexConfiguration {
                metric_space: MetricSpace::Euclidean,
            }),
            field_definitions: vec![],
        };
        let json = serde_json::to_string(&req).unwrap();
        let parsed: CreateCollectionRequest = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed, req);
        // Verify wire names on request
        assert!(
            json.contains("\"collection_name\""),
            "request should use collection_name"
        );
        assert!(
            json.contains("\"collection_description\""),
            "request should use collection_description"
        );
    }

    #[test]
    fn create_collection_request_minimal() {
        let req = CreateCollectionRequest {
            name: "minimal".into(),
            description: None,
            embedding_model: "grok-embedding-small".into(),
            chunk_configuration: None,
            index_configuration: None,
            field_definitions: vec![],
        };
        let json = serde_json::to_string(&req).unwrap();
        // Optional fields should be absent from JSON
        assert!(!json.contains("chunk_configuration"));
        assert!(!json.contains("index_configuration"));
        assert!(!json.contains("field_definitions"));
        assert!(!json.contains("collection_description"));
    }

    #[test]
    fn update_collection_request_serde_round_trip() {
        let req = UpdateCollectionRequest {
            name: Some("renamed".into()),
            description: Some("new description".into()),
            chunk_configuration: None,
            field_definitions: Some(vec![FieldDefinition {
                name: "author".into(),
                required: false,
                unique: false,
                inject_into_chunk: false,
            }]),
        };
        let json = serde_json::to_string(&req).unwrap();
        let parsed: UpdateCollectionRequest = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed, req);
        // Verify wire names
        assert!(
            json.contains("\"collection_name\""),
            "update should use collection_name"
        );
        assert!(
            json.contains("\"collection_description\""),
            "update should use collection_description"
        );
    }

    #[test]
    fn update_collection_request_empty_is_valid() {
        let req = UpdateCollectionRequest::default();
        let json = serde_json::to_string(&req).unwrap();
        assert_eq!(json, "{}");
    }

    #[test]
    fn collection_list_serde_round_trip() {
        let list = CollectionList {
            collections: vec![Collection {
                id: "col_1".into(),
                name: "first".into(),
                description: None,
                embedding_model: "grok-embedding-small".into(),
                chunk_configuration: None,
                index_configuration: None,
                field_definitions: vec![],
                documents_count: None,
                created_at: None,
                modified_at: None,
            }],
        };
        let json = serde_json::to_string(&list).unwrap();
        let parsed: CollectionList = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed, list);
    }

    #[test]
    fn collection_list_empty() {
        let json = r#"{"collections":[]}"#;
        let parsed: CollectionList = serde_json::from_str(json).unwrap();
        assert!(parsed.collections.is_empty());
    }

    #[test]
    fn field_definition_defaults() {
        let json = r#"{"name":"tag"}"#;
        let parsed: FieldDefinition = serde_json::from_str(json).unwrap();
        assert_eq!(parsed.name, "tag");
        assert!(!parsed.required);
        assert!(!parsed.unique);
        assert!(!parsed.inject_into_chunk);
    }

    #[test]
    fn collection_deserializes_from_api_shape() {
        // Simulate the shape the Management API actually returns.
        let json = r#"{
            "collection_id": "col_xyz789",
            "collection_name": "production-docs",
            "collection_description": "Production documentation collection",
            "embedding_model": "grok-embedding-small",
            "chunk_configuration": {
                "strategy": "markdown",
                "chunk_size": 2048,
                "chunk_overlap": 256
            },
            "index_configuration": {
                "metric_space": "inner_product"
            },
            "field_definitions": [
                {"name": "category", "required": true, "unique": false, "inject_into_chunk": true}
            ],
            "documents_count": 150,
            "created_at": "2026-04-01T00:00:00Z",
            "modified_at": "2026-04-05T10:30:00Z"
        }"#;
        let parsed: Collection = serde_json::from_str(json).unwrap();
        assert_eq!(parsed.id, "col_xyz789");
        assert_eq!(parsed.name, "production-docs");
        assert_eq!(
            parsed.description.as_deref(),
            Some("Production documentation collection")
        );
        assert_eq!(
            parsed.chunk_configuration.as_ref().unwrap().strategy,
            ChunkStrategy::Markdown
        );
        assert_eq!(
            parsed.index_configuration.as_ref().unwrap().metric_space,
            MetricSpace::InnerProduct
        );
        assert_eq!(parsed.field_definitions.len(), 1);
        assert!(parsed.field_definitions[0].inject_into_chunk);
        assert_eq!(parsed.documents_count, Some(150));
    }

    #[test]
    fn index_configuration_serde_round_trip() {
        let cfg = IndexConfiguration {
            metric_space: MetricSpace::Cosine,
        };
        let json = serde_json::to_string(&cfg).unwrap();
        let parsed: IndexConfiguration = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed, cfg);
    }
}
