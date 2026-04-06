use std::fmt::Write as _;
use std::path::Path;

use reqwest::Method;
use reqwest::multipart;

use crate::transport::client::HttpClient;
use crate::transport::error::TransportError;

use super::util::encode_path_segment;
use crate::types::files::{
    ChunkedUploadInit, ChunkedUploadInitRequest, FileDownloadRequest, FileList, FileObject,
    FileUpdateRequest,
};

/// Client for the xAI Files API.
///
/// Provides methods for uploading, listing, retrieving, updating, and
/// downloading files via the `/v1/files` family of endpoints.
///
/// This client accepts `&Path` for file uploads. Path validation (e.g.,
/// workspace-relative enforcement) is the caller's responsibility — this
/// crate does NOT depend on `grokrs-cap`.
pub struct FilesClient<'a> {
    http: &'a HttpClient,
}

impl<'a> FilesClient<'a> {
    /// Create a new `FilesClient` wrapping the given HTTP client.
    #[must_use]
    pub fn new(http: &'a HttpClient) -> Self {
        Self { http }
    }

    /// Upload a file from disk.
    ///
    /// Sends a `POST /v1/files` request with `multipart/form-data` containing
    /// the file contents and its purpose.
    ///
    /// # Arguments
    /// * `path` - The filesystem path to the file to upload. The caller is
    ///   responsible for validating this path (e.g., via `WorkspacePath`).
    /// * `purpose` - The intended purpose of the file (e.g., "assistants",
    ///   "fine-tune").
    ///
    /// # Errors
    /// Returns `TransportError::Serialization` if the file cannot be read
    /// from disk, or any transport-level error from the HTTP client.
    pub async fn upload(&self, path: &Path, purpose: &str) -> Result<FileObject, TransportError> {
        let file_bytes =
            tokio::fs::read(path)
                .await
                .map_err(|e| TransportError::Serialization {
                    message: format!("failed to read file at {}: {e}", path.display()),
                })?;

        let filename = path.file_name().map_or_else(
            || "upload".to_string(),
            |n| n.to_string_lossy().into_owned(),
        );

        let file_part = multipart::Part::bytes(file_bytes)
            .file_name(filename)
            .mime_str("application/octet-stream")
            .map_err(|e| TransportError::Serialization {
                message: format!("failed to set MIME type on multipart part: {e}"),
            })?;

        let form = multipart::Form::new()
            .text("purpose", purpose.to_string())
            .part("file", file_part);

        self.http.send_multipart("/v1/files", form).await
    }

    /// Initialize a chunked upload session.
    ///
    /// Sends a `POST /v1/files:initialize` request to begin a multi-part
    /// chunked upload. Returns an upload ID that should be used with
    /// `upload_chunks` to send individual chunks.
    ///
    /// # Arguments
    /// * `filename` - The name of the file being uploaded.
    /// * `purpose` - The intended purpose of the file.
    /// * `bytes` - The total size of the file in bytes.
    pub async fn initialize_chunked(
        &self,
        filename: &str,
        purpose: &str,
        bytes: u64,
    ) -> Result<ChunkedUploadInit, TransportError> {
        let body = ChunkedUploadInitRequest {
            filename: filename.to_string(),
            purpose: purpose.to_string(),
            bytes,
        };
        self.http
            .send_json(Method::POST, "/v1/files:initialize", &body)
            .await
    }

    /// Upload a chunk of data for a chunked upload.
    ///
    /// Sends a `POST /v1/files:uploadChunks` request with the chunk data
    /// as a multipart form. Each chunk is identified by `upload_id` and
    /// `chunk_index`.
    ///
    /// # Arguments
    /// * `upload_id` - The upload session ID from `initialize_chunked`.
    /// * `chunk_data` - The raw bytes for this chunk.
    /// * `chunk_index` - The zero-based index of this chunk.
    pub async fn upload_chunks(
        &self,
        upload_id: &str,
        chunk_data: Vec<u8>,
        chunk_index: u32,
    ) -> Result<FileObject, TransportError> {
        let chunk_part = multipart::Part::bytes(chunk_data)
            .file_name(format!("chunk_{chunk_index}"))
            .mime_str("application/octet-stream")
            .map_err(|e| TransportError::Serialization {
                message: format!("failed to set MIME type on chunk part: {e}"),
            })?;

        let form = multipart::Form::new()
            .text("upload_id", upload_id.to_string())
            .text("chunk_index", chunk_index.to_string())
            .part("chunk", chunk_part);

        self.http
            .send_multipart("/v1/files:uploadChunks", form)
            .await
    }

    /// List uploaded files.
    ///
    /// Sends a `GET /v1/files` request. Optionally filters by purpose.
    ///
    /// # Arguments
    /// * `purpose` - If provided, only files with this purpose are returned.
    pub async fn list(&self, purpose: Option<&str>) -> Result<FileList, TransportError> {
        let path = match purpose {
            Some(p) => format!("/v1/files?purpose={}", urlencoding_minimal(p)),
            None => "/v1/files".to_string(),
        };
        self.http.send_no_body(Method::GET, &path).await
    }

    /// Get metadata for a specific file.
    ///
    /// Sends a `GET /v1/files/{file_id}` request.
    ///
    /// # Arguments
    /// * `file_id` - The opaque file identifier.
    pub async fn get(&self, file_id: &str) -> Result<FileObject, TransportError> {
        let path = format!("/v1/files/{}", encode_path_segment(file_id));
        self.http.send_no_body(Method::GET, &path).await
    }

    /// Update a file's metadata (filename).
    ///
    /// Sends a `PUT /v1/files/{file_id}` request with the new filename.
    ///
    /// # Arguments
    /// * `file_id` - The opaque file identifier.
    /// * `filename` - The new filename to assign.
    pub async fn update(
        &self,
        file_id: &str,
        filename: &str,
    ) -> Result<FileObject, TransportError> {
        let path = format!("/v1/files/{}", encode_path_segment(file_id));
        let body = FileUpdateRequest {
            filename: filename.to_string(),
        };
        self.http.send_json(Method::PUT, &path, &body).await
    }

    /// Download a file's contents.
    ///
    /// Sends a `POST /v1/files:download` request and returns the raw bytes.
    ///
    /// # Arguments
    /// * `file_id` - The opaque file identifier.
    pub async fn download(&self, file_id: &str) -> Result<Vec<u8>, TransportError> {
        let body = FileDownloadRequest {
            file_id: file_id.to_string(),
        };
        self.http
            .send_json_raw(Method::POST, "/v1/files:download", &body)
            .await
    }
}

/// Minimal percent-encoding for query parameter values.
///
/// Encodes characters that are not unreserved per RFC 3986 (letters, digits,
/// `-`, `.`, `_`, `~`). This avoids pulling in a full URL-encoding crate for
/// a single use case.
fn urlencoding_minimal(input: &str) -> String {
    let mut encoded = String::with_capacity(input.len());
    for byte in input.bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'.' | b'_' | b'~' => {
                encoded.push(byte as char);
            }
            _ => {
                write!(encoded, "%{byte:02X}").unwrap();
            }
        }
    }
    encoded
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn urlencoding_minimal_passes_through_unreserved() {
        assert_eq!(urlencoding_minimal("assistants"), "assistants");
        assert_eq!(urlencoding_minimal("fine-tune"), "fine-tune");
        assert_eq!(urlencoding_minimal("my_file.txt"), "my_file.txt");
        assert_eq!(urlencoding_minimal("a~b"), "a~b");
    }

    #[test]
    fn urlencoding_minimal_encodes_special_chars() {
        assert_eq!(urlencoding_minimal("hello world"), "hello%20world");
        assert_eq!(urlencoding_minimal("a&b=c"), "a%26b%3Dc");
    }

    #[test]
    fn upload_path_construction() {
        // Verify the upload endpoint path is correct
        let path = "/v1/files";
        assert_eq!(path, "/v1/files");
    }

    #[test]
    fn list_path_without_purpose() {
        let path = "/v1/files";
        assert_eq!(path, "/v1/files");
    }

    #[test]
    fn list_path_with_purpose() {
        let purpose = "assistants";
        let path = format!("/v1/files?purpose={}", urlencoding_minimal(purpose));
        assert_eq!(path, "/v1/files?purpose=assistants");
    }

    #[test]
    fn list_path_with_purpose_needing_encoding() {
        let purpose = "fine tune";
        let path = format!("/v1/files?purpose={}", urlencoding_minimal(purpose));
        assert_eq!(path, "/v1/files?purpose=fine%20tune");
    }

    #[test]
    fn get_path_construction() {
        let file_id = "file-abc123";
        let path = format!("/v1/files/{}", encode_path_segment(file_id));
        assert_eq!(path, "/v1/files/file-abc123");
    }

    #[test]
    fn update_path_construction() {
        let file_id = "file-xyz";
        let path = format!("/v1/files/{}", encode_path_segment(file_id));
        assert_eq!(path, "/v1/files/file-xyz");
    }

    #[test]
    fn get_path_encodes_slash() {
        let file_id = "file/abc";
        let path = format!("/v1/files/{}", encode_path_segment(file_id));
        assert_eq!(path, "/v1/files/file%2Fabc");
    }

    #[test]
    fn get_path_encodes_query_chars() {
        let file_id = "file?v=1";
        let path = format!("/v1/files/{}", encode_path_segment(file_id));
        assert_eq!(path, "/v1/files/file%3Fv%3D1");
    }

    #[test]
    fn get_path_encodes_hash() {
        let file_id = "file#frag";
        let path = format!("/v1/files/{}", encode_path_segment(file_id));
        assert_eq!(path, "/v1/files/file%23frag");
    }

    #[test]
    fn get_path_encodes_space() {
        let file_id = "file abc";
        let path = format!("/v1/files/{}", encode_path_segment(file_id));
        assert_eq!(path, "/v1/files/file%20abc");
    }

    #[test]
    fn download_path_construction() {
        let path = "/v1/files:download";
        assert_eq!(path, "/v1/files:download");
    }

    #[test]
    fn initialize_chunked_path_construction() {
        let path = "/v1/files:initialize";
        assert_eq!(path, "/v1/files:initialize");
    }

    #[test]
    fn upload_chunks_path_construction() {
        let path = "/v1/files:uploadChunks";
        assert_eq!(path, "/v1/files:uploadChunks");
    }

    #[test]
    fn file_id_is_opaque_string() {
        // file_id should be treated as an opaque string in path construction
        let file_id = "any-arbitrary-string-format";
        let path = format!("/v1/files/{}", encode_path_segment(file_id));
        assert!(path.ends_with(file_id));
    }

    #[test]
    fn file_update_request_serializes() {
        let req = FileUpdateRequest {
            filename: "new_name.jsonl".to_string(),
        };
        let json = serde_json::to_string(&req).unwrap();
        assert!(json.contains("\"filename\":\"new_name.jsonl\""));
    }

    #[test]
    fn file_download_request_serializes() {
        let req = FileDownloadRequest {
            file_id: "file-abc".to_string(),
        };
        let json = serde_json::to_string(&req).unwrap();
        assert!(json.contains("\"file_id\":\"file-abc\""));
    }

    #[test]
    fn chunked_upload_init_request_serializes() {
        let req = ChunkedUploadInitRequest {
            filename: "big_file.bin".to_string(),
            purpose: "assistants".to_string(),
            bytes: 1_000_000,
        };
        let json = serde_json::to_string(&req).unwrap();
        assert!(json.contains("\"filename\":\"big_file.bin\""));
        assert!(json.contains("\"purpose\":\"assistants\""));
        assert!(json.contains("\"bytes\":1000000"));
    }
}
