use serde::{Deserialize, Serialize};

/// Represents a file object stored on the xAI platform.
///
/// Returned by upload, get, update, and list operations. All metadata fields
/// are optional because different endpoints may return partial representations.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FileObject {
    /// The unique identifier for this file (opaque string).
    pub id: String,

    /// The object type (e.g., "file").
    #[serde(skip_serializing_if = "Option::is_none")]
    pub object: Option<String>,

    /// File size in bytes.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub bytes: Option<u64>,

    /// Unix timestamp of when the file was created.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub created_at: Option<i64>,

    /// The original filename.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub filename: Option<String>,

    /// The intended purpose of the file (e.g., "assistants", "fine-tune").
    #[serde(skip_serializing_if = "Option::is_none")]
    pub purpose: Option<String>,
}

/// Response from the list files endpoint.
///
/// Returned by `GET /v1/files`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FileList {
    /// The list of file objects.
    pub data: Vec<FileObject>,

    /// Whether there are more files available beyond this page.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub has_more: Option<bool>,
}

/// Response from the chunked upload initialization endpoint.
///
/// Returned by `POST /v1/files:initialize`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ChunkedUploadInit {
    /// The upload session identifier, used to upload subsequent chunks.
    pub upload_id: String,

    /// The file identifier, if assigned at initialization time.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub file_id: Option<String>,
}

/// Request body for initializing a chunked upload.
#[derive(Debug, Clone, Serialize)]
pub(crate) struct ChunkedUploadInitRequest {
    /// The filename for the file being uploaded.
    pub filename: String,

    /// The intended purpose of the file.
    pub purpose: String,

    /// Total size of the file in bytes.
    pub bytes: u64,
}

/// Request body for the file download endpoint.
#[derive(Debug, Clone, Serialize)]
pub(crate) struct FileDownloadRequest {
    /// The file identifier to download.
    pub file_id: String,
}

/// Request body for updating a file's metadata.
#[derive(Debug, Clone, Serialize)]
pub(crate) struct FileUpdateRequest {
    /// The new filename.
    pub filename: String,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn file_object_round_trips() {
        let file = FileObject {
            id: "file-abc123".into(),
            object: Some("file".into()),
            bytes: Some(12345),
            created_at: Some(1700000000),
            filename: Some("data.jsonl".into()),
            purpose: Some("assistants".into()),
        };
        let json = serde_json::to_string(&file).unwrap();
        let back: FileObject = serde_json::from_str(&json).unwrap();
        assert_eq!(file, back);
    }

    #[test]
    fn file_object_minimal_round_trips() {
        let file = FileObject {
            id: "file-xyz".into(),
            object: None,
            bytes: None,
            created_at: None,
            filename: None,
            purpose: None,
        };
        let json = serde_json::to_string(&file).unwrap();
        // Optional fields should be absent from JSON
        assert!(!json.contains("object"));
        assert!(!json.contains("bytes"));
        assert!(!json.contains("created_at"));
        assert!(!json.contains("filename"));
        assert!(!json.contains("purpose"));
        let back: FileObject = serde_json::from_str(&json).unwrap();
        assert_eq!(file, back);
    }

    #[test]
    fn file_object_deserializes_with_unknown_fields() {
        let json = r#"{
            "id": "file-abc",
            "object": "file",
            "bytes": 999,
            "some_future_field": "should be ignored",
            "another_field": 42
        }"#;
        let file: FileObject = serde_json::from_str(json).unwrap();
        assert_eq!(file.id, "file-abc");
        assert_eq!(file.bytes, Some(999));
    }

    #[test]
    fn file_object_id_is_opaque_string() {
        // file_id should accept any string format, not just "file-*"
        let json = r#"{"id": "completely-arbitrary-id-format-12345"}"#;
        let file: FileObject = serde_json::from_str(json).unwrap();
        assert_eq!(file.id, "completely-arbitrary-id-format-12345");
    }

    #[test]
    fn file_list_round_trips() {
        let list = FileList {
            data: vec![
                FileObject {
                    id: "file-1".into(),
                    object: Some("file".into()),
                    bytes: Some(100),
                    created_at: Some(1700000000),
                    filename: Some("a.txt".into()),
                    purpose: Some("assistants".into()),
                },
                FileObject {
                    id: "file-2".into(),
                    object: None,
                    bytes: None,
                    created_at: None,
                    filename: None,
                    purpose: None,
                },
            ],
            has_more: Some(true),
        };
        let json = serde_json::to_string(&list).unwrap();
        let back: FileList = serde_json::from_str(&json).unwrap();
        assert_eq!(list, back);
    }

    #[test]
    fn file_list_empty_data_no_has_more() {
        let list = FileList {
            data: vec![],
            has_more: None,
        };
        let json = serde_json::to_string(&list).unwrap();
        assert!(json.contains("\"data\":[]"));
        assert!(!json.contains("has_more"));
        let back: FileList = serde_json::from_str(&json).unwrap();
        assert_eq!(back.data.len(), 0);
        assert!(back.has_more.is_none());
    }

    #[test]
    fn file_list_empty_data_with_has_more_false() {
        let json = r#"{"data":[],"has_more":false}"#;
        let list: FileList = serde_json::from_str(json).unwrap();
        assert!(list.data.is_empty());
        assert_eq!(list.has_more, Some(false));
    }

    #[test]
    fn chunked_upload_init_round_trips() {
        let init = ChunkedUploadInit {
            upload_id: "upload-abc".into(),
            file_id: Some("file-pending".into()),
        };
        let json = serde_json::to_string(&init).unwrap();
        let back: ChunkedUploadInit = serde_json::from_str(&json).unwrap();
        assert_eq!(init, back);
    }

    #[test]
    fn chunked_upload_init_without_file_id() {
        let json = r#"{"upload_id":"upload-xyz"}"#;
        let init: ChunkedUploadInit = serde_json::from_str(json).unwrap();
        assert_eq!(init.upload_id, "upload-xyz");
        assert!(init.file_id.is_none());
    }

    #[test]
    fn file_object_deserializes_from_wire_format() {
        let json = r#"{
            "id": "file-abc123",
            "object": "file",
            "bytes": 12345,
            "created_at": 1700000000,
            "filename": "training_data.jsonl",
            "purpose": "fine-tune"
        }"#;
        let file: FileObject = serde_json::from_str(json).unwrap();
        assert_eq!(file.id, "file-abc123");
        assert_eq!(file.object.as_deref(), Some("file"));
        assert_eq!(file.bytes, Some(12345));
        assert_eq!(file.created_at, Some(1700000000));
        assert_eq!(file.filename.as_deref(), Some("training_data.jsonl"));
        assert_eq!(file.purpose.as_deref(), Some("fine-tune"));
    }
}
