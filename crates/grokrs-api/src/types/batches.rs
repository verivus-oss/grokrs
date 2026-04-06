use std::fmt;

use serde::{Deserialize, Serialize};

use crate::transport::error::TransportError;
#[allow(deprecated)]
use crate::types::chat::ChatCompletionRequest;
use crate::types::images::{ImageEditRequest, ImageGenerationRequest};
use crate::types::responses::CreateResponseRequest;
use crate::types::videos::VideoGenerationRequest;

/// Errors specific to batch operations.
///
/// Covers both transport-level errors (delegated from [`TransportError`]) and
/// batch-specific failures such as pagination limits being exceeded.
#[derive(Debug)]
pub enum BatchError {
    /// A transport-level error occurred during an HTTP request.
    Transport(TransportError),

    /// The pagination limit was reached while collecting batch results, but
    /// the server indicated more results exist. Returning partial data
    /// silently would violate fail-closed safety, so this error is raised
    /// instead.
    PaginationLimitExceeded {
        /// The number of result items collected before the limit was hit.
        items_collected: usize,
        /// The maximum number of pages that were fetched.
        max_pages: u32,
    },
}

impl fmt::Display for BatchError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            BatchError::Transport(err) => write!(f, "{err}"),
            BatchError::PaginationLimitExceeded {
                items_collected,
                max_pages,
            } => {
                write!(
                    f,
                    "pagination limit exceeded: collected {items_collected} items \
                     across {max_pages} pages but more results remain"
                )
            }
        }
    }
}

impl std::error::Error for BatchError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            BatchError::Transport(err) => Some(err),
            BatchError::PaginationLimitExceeded { .. } => None,
        }
    }
}

impl From<TransportError> for BatchError {
    fn from(err: TransportError) -> Self {
        BatchError::Transport(err)
    }
}

/// The API endpoint that a batch targets.
///
/// Each variant maps to the wire-format path string expected by the xAI batch
/// API (e.g., `/v1/chat/completions` or `/v1/responses`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum BatchEndpoint {
    /// Target the Chat Completions API — `/v1/chat/completions`.
    #[serde(rename = "/v1/chat/completions")]
    ChatCompletions,

    /// Target the Responses API — `/v1/responses`.
    #[serde(rename = "/v1/responses")]
    Responses,

    /// Target the Image Generations API — `/v1/images/generations`.
    #[serde(rename = "/v1/images/generations")]
    ImageGenerations,

    /// Target the Image Edits API — `/v1/images/edits`.
    #[serde(rename = "/v1/images/edits")]
    ImageEdits,

    /// Target the Video Generations API — `/v1/videos/generations`.
    #[serde(rename = "/v1/videos/generations")]
    VideoGenerations,
}

/// Request body for creating a new batch.
///
/// Sent as `POST /v1/batches`.
///
/// There are two creation flows:
/// 1. **Incremental**: Create the batch with just `endpoint`, then call
///    `add_requests()` to append individual request items.
/// 2. **JSONL upload**: Upload a JSONL file via the Files API, then pass its
///    `file_id` as `input_file_id` to create the batch in one shot.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CreateBatchRequest {
    /// The API endpoint that the batch requests will target.
    pub endpoint: BatchEndpoint,

    /// The ID of a previously uploaded JSONL file containing batch requests.
    ///
    /// When set, the batch is created from the file contents rather than
    /// requiring separate `add_requests()` calls. The file must have been
    /// uploaded via the Files API and contain one JSON request object per line,
    /// each with `method`, `url`, `custom_id`, and `body` fields.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub input_file_id: Option<String>,

    /// Optional metadata to attach to the batch.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub metadata: Option<serde_json::Value>,
}

/// The lifecycle status of a batch.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum BatchStatus {
    /// The batch has been created but processing has not started.
    #[serde(rename = "created")]
    Created,

    /// The batch is currently being processed.
    #[serde(rename = "in_progress")]
    InProgress,

    /// The batch has finished processing successfully.
    #[serde(rename = "completed")]
    Completed,

    /// The batch failed during processing.
    #[serde(rename = "failed")]
    Failed,

    /// The batch was cancelled before completion.
    #[serde(rename = "cancelled")]
    Cancelled,
}

/// A batch object returned by the xAI Batch API.
///
/// Represents the state of a batch at a point in time, including progress
/// counts and timestamps. All optional fields use `skip_serializing_if` to
/// keep serialized output clean. Unknown fields from the server are silently
/// ignored (no `deny_unknown_fields`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BatchObject {
    /// The unique batch identifier (opaque string).
    pub id: String,

    /// The object type (e.g., `"batch"`).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub object: Option<String>,

    /// The API endpoint that the batch targets.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub endpoint: Option<String>,

    /// The current lifecycle status of the batch.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub status: Option<BatchStatus>,

    /// Unix timestamp (seconds) when the batch was created.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub created_at: Option<i64>,

    /// Unix timestamp (seconds) when the batch finished processing.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub completed_at: Option<i64>,

    /// Number of requests still pending.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub num_pending: Option<u64>,

    /// Number of requests that succeeded.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub num_success: Option<u64>,

    /// Number of requests that errored.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub num_error: Option<u64>,

    /// Arbitrary metadata attached to the batch.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub metadata: Option<serde_json::Value>,
}

/// A single request item to add to a batch.
///
/// The `body` is an opaque JSON value because it can be either a
/// `ChatCompletionRequest` or a `CreateResponseRequest` payload, depending
/// on the batch's `endpoint`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BatchRequestItem {
    /// An optional caller-defined identifier for correlating results.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub custom_id: Option<String>,

    /// The HTTP method (e.g., `"POST"`).
    pub method: String,

    /// The API endpoint URL path (e.g., `/v1/chat/completions`).
    pub url: String,

    /// The request body as an opaque JSON value.
    pub body: serde_json::Value,
}

impl BatchRequestItem {
    /// Create a batch request item targeting the Chat Completions API.
    ///
    /// Enforces `method: "POST"` and `url: "/v1/chat/completions"` so that
    /// callers cannot accidentally mis-specify the endpoint.
    ///
    /// # Panics
    ///
    /// Panics if the request body cannot be serialized to JSON. This is
    /// infallible for known request structs and indicates a programming error.
    #[allow(deprecated)] // ChatCompletionRequest is deprecated but still needed for batch support
    #[must_use]
    pub fn chat_completion(custom_id: Option<String>, body: &ChatCompletionRequest) -> Self {
        Self {
            custom_id,
            method: "POST".into(),
            url: "/v1/chat/completions".into(),
            body: serde_json::to_value(body)
                .expect("ChatCompletionRequest serialization is infallible"),
        }
    }

    /// Create a batch request item targeting the Responses API.
    ///
    /// Enforces `method: "POST"` and `url: "/v1/responses"` so that callers
    /// cannot accidentally mis-specify the endpoint.
    ///
    /// # Panics
    ///
    /// Panics if the request body cannot be serialized to JSON. This is
    /// infallible for known request structs and indicates a programming error.
    #[must_use]
    pub fn response(custom_id: Option<String>, body: &CreateResponseRequest) -> Self {
        Self {
            custom_id,
            method: "POST".into(),
            url: "/v1/responses".into(),
            body: serde_json::to_value(body)
                .expect("CreateResponseRequest serialization is infallible"),
        }
    }

    /// Create a batch request item targeting the Image Generations API.
    ///
    /// Enforces `method: "POST"` and `url: "/v1/images/generations"` so that
    /// callers cannot accidentally mis-specify the endpoint.
    ///
    /// # Panics
    ///
    /// Panics if the request body cannot be serialized to JSON. This is
    /// infallible for known request structs and indicates a programming error.
    #[must_use]
    pub fn image_generation(custom_id: Option<String>, body: &ImageGenerationRequest) -> Self {
        Self {
            custom_id,
            method: "POST".into(),
            url: "/v1/images/generations".into(),
            body: serde_json::to_value(body)
                .expect("ImageGenerationRequest serialization is infallible"),
        }
    }

    /// Create a batch request item targeting the Image Edits API.
    ///
    /// Enforces `method: "POST"` and `url: "/v1/images/edits"` so that
    /// callers cannot accidentally mis-specify the endpoint.
    ///
    /// # Panics
    ///
    /// Panics if the request body cannot be serialized to JSON. This is
    /// infallible for known request structs and indicates a programming error.
    #[must_use]
    pub fn image_edit(custom_id: Option<String>, body: &ImageEditRequest) -> Self {
        Self {
            custom_id,
            method: "POST".into(),
            url: "/v1/images/edits".into(),
            body: serde_json::to_value(body).expect("ImageEditRequest serialization is infallible"),
        }
    }

    /// Create a batch request item targeting the Video Generations API.
    ///
    /// Enforces `method: "POST"` and `url: "/v1/videos/generations"` so that
    /// callers cannot accidentally mis-specify the endpoint.
    ///
    /// # Panics
    ///
    /// Panics if the request body cannot be serialized to JSON. This is
    /// infallible for known request structs and indicates a programming error.
    #[must_use]
    pub fn video_generation(custom_id: Option<String>, body: &VideoGenerationRequest) -> Self {
        Self {
            custom_id,
            method: "POST".into(),
            url: "/v1/videos/generations".into(),
            body: serde_json::to_value(body)
                .expect("VideoGenerationRequest serialization is infallible"),
        }
    }
}

/// Payload for adding requests to an existing batch.
///
/// Sent as `POST /v1/batches/{id}/requests`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AddBatchRequestsPayload {
    /// The list of request items to add.
    pub requests: Vec<BatchRequestItem>,
}

/// A single result item from a batch execution.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BatchResultItem {
    /// The caller-defined identifier that was provided in the request.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub custom_id: Option<String>,

    /// The response body from the API, if the request succeeded.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub response: Option<serde_json::Value>,

    /// The error object, if the request failed.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<serde_json::Value>,

    /// The HTTP status code returned for this individual request.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub status_code: Option<u16>,

    /// Per-request cost in USD ticks (micro-cents or similar granularity).
    ///
    /// Present when the API reports cost data for individual batch items.
    /// The exact unit is defined by the xAI API documentation.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cost_in_usd_ticks: Option<i64>,
}

/// Paginated response from the batch results endpoint.
///
/// Returned by `GET /v1/batches/{id}/results`. When `has_more` is `true`,
/// use `pagination_token` to fetch the next page.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BatchResultsResponse {
    /// The result items in this page.
    pub data: Vec<BatchResultItem>,

    /// Whether there are more results beyond this page.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub has_more: Option<bool>,

    /// Token to pass as a query parameter to fetch the next page.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub pagination_token: Option<String>,
}

/// Response from the list batches endpoint.
///
/// Returned by `GET /v1/batches`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BatchList {
    /// The list of batch objects.
    pub data: Vec<BatchObject>,

    /// Whether there are more batches beyond this page.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub has_more: Option<bool>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn create_batch_request_round_trips() {
        let req = CreateBatchRequest {
            endpoint: BatchEndpoint::Responses,
            input_file_id: None,
            metadata: Some(serde_json::json!({"project": "test"})),
        };
        let json = serde_json::to_string(&req).unwrap();
        let back: CreateBatchRequest = serde_json::from_str(&json).unwrap();
        assert_eq!(back.endpoint, BatchEndpoint::Responses);
        assert!(back.input_file_id.is_none());
        assert_eq!(
            back.metadata.as_ref().unwrap()["project"],
            serde_json::json!("test")
        );
    }

    #[test]
    fn create_batch_request_without_metadata_round_trips() {
        let req = CreateBatchRequest {
            endpoint: BatchEndpoint::ChatCompletions,
            input_file_id: None,
            metadata: None,
        };
        let json = serde_json::to_string(&req).unwrap();
        assert!(!json.contains("metadata"));
        assert!(!json.contains("input_file_id"));
        let back: CreateBatchRequest = serde_json::from_str(&json).unwrap();
        assert_eq!(back.endpoint, BatchEndpoint::ChatCompletions);
        assert!(back.input_file_id.is_none());
        assert!(back.metadata.is_none());
    }

    #[test]
    fn create_batch_request_with_input_file_id_round_trips() {
        let req = CreateBatchRequest {
            endpoint: BatchEndpoint::Responses,
            input_file_id: Some("file-abc123".into()),
            metadata: Some(serde_json::json!({"source": "jsonl"})),
        };
        let json = serde_json::to_string(&req).unwrap();
        assert!(json.contains("\"input_file_id\":\"file-abc123\""));
        let back: CreateBatchRequest = serde_json::from_str(&json).unwrap();
        assert_eq!(back.endpoint, BatchEndpoint::Responses);
        assert_eq!(back.input_file_id.as_deref(), Some("file-abc123"));
        assert_eq!(
            back.metadata.as_ref().unwrap()["source"],
            serde_json::json!("jsonl")
        );
    }

    #[test]
    fn create_batch_request_input_file_id_skipped_when_none() {
        let req = CreateBatchRequest {
            endpoint: BatchEndpoint::ChatCompletions,
            input_file_id: None,
            metadata: None,
        };
        let json = serde_json::to_string(&req).unwrap();
        assert!(!json.contains("input_file_id"));
    }

    #[test]
    fn batch_endpoint_serializes_to_correct_path_strings() {
        let chat = serde_json::to_string(&BatchEndpoint::ChatCompletions).unwrap();
        assert_eq!(chat, r#""/v1/chat/completions""#);

        let responses = serde_json::to_string(&BatchEndpoint::Responses).unwrap();
        assert_eq!(responses, r#""/v1/responses""#);
    }

    #[test]
    fn batch_endpoint_deserializes_from_path_strings() {
        let chat: BatchEndpoint = serde_json::from_str(r#""/v1/chat/completions""#).unwrap();
        assert_eq!(chat, BatchEndpoint::ChatCompletions);

        let responses: BatchEndpoint = serde_json::from_str(r#""/v1/responses""#).unwrap();
        assert_eq!(responses, BatchEndpoint::Responses);
    }

    #[test]
    fn batch_status_created_round_trips() {
        let status = BatchStatus::Created;
        let json = serde_json::to_string(&status).unwrap();
        assert_eq!(json, r#""created""#);
        let back: BatchStatus = serde_json::from_str(&json).unwrap();
        assert_eq!(back, BatchStatus::Created);
    }

    #[test]
    fn batch_status_in_progress_round_trips() {
        let status = BatchStatus::InProgress;
        let json = serde_json::to_string(&status).unwrap();
        assert_eq!(json, r#""in_progress""#);
        let back: BatchStatus = serde_json::from_str(&json).unwrap();
        assert_eq!(back, BatchStatus::InProgress);
    }

    #[test]
    fn batch_status_completed_round_trips() {
        let status = BatchStatus::Completed;
        let json = serde_json::to_string(&status).unwrap();
        assert_eq!(json, r#""completed""#);
        let back: BatchStatus = serde_json::from_str(&json).unwrap();
        assert_eq!(back, BatchStatus::Completed);
    }

    #[test]
    fn batch_status_failed_round_trips() {
        let status = BatchStatus::Failed;
        let json = serde_json::to_string(&status).unwrap();
        assert_eq!(json, r#""failed""#);
        let back: BatchStatus = serde_json::from_str(&json).unwrap();
        assert_eq!(back, BatchStatus::Failed);
    }

    #[test]
    fn batch_status_cancelled_round_trips() {
        let status = BatchStatus::Cancelled;
        let json = serde_json::to_string(&status).unwrap();
        assert_eq!(json, r#""cancelled""#);
        let back: BatchStatus = serde_json::from_str(&json).unwrap();
        assert_eq!(back, BatchStatus::Cancelled);
    }

    #[test]
    fn batch_object_deserializes_with_unknown_fields() {
        let json = r#"{
            "id": "batch_abc123",
            "object": "batch",
            "status": "completed",
            "future_field": "should be ignored",
            "another_unknown": 42
        }"#;
        let batch: BatchObject = serde_json::from_str(json).unwrap();
        assert_eq!(batch.id, "batch_abc123");
        assert_eq!(batch.object.as_deref(), Some("batch"));
        assert_eq!(batch.status, Some(BatchStatus::Completed));
    }

    #[test]
    fn batch_object_full_round_trips() {
        let batch = BatchObject {
            id: "batch_xyz".into(),
            object: Some("batch".into()),
            endpoint: Some("/v1/responses".into()),
            status: Some(BatchStatus::InProgress),
            created_at: Some(1_700_000_000),
            completed_at: None,
            num_pending: Some(5),
            num_success: Some(10),
            num_error: Some(2),
            metadata: Some(serde_json::json!({"tag": "experiment"})),
        };
        let json = serde_json::to_string(&batch).unwrap();
        let back: BatchObject = serde_json::from_str(&json).unwrap();
        assert_eq!(batch, back);
    }

    #[test]
    fn batch_object_minimal_round_trips() {
        let batch = BatchObject {
            id: "batch_min".into(),
            object: None,
            endpoint: None,
            status: None,
            created_at: None,
            completed_at: None,
            num_pending: None,
            num_success: None,
            num_error: None,
            metadata: None,
        };
        let json = serde_json::to_string(&batch).unwrap();
        assert!(!json.contains("object"));
        assert!(!json.contains("endpoint"));
        assert!(!json.contains("status"));
        assert!(!json.contains("created_at"));
        assert!(!json.contains("completed_at"));
        assert!(!json.contains("num_pending"));
        assert!(!json.contains("num_success"));
        assert!(!json.contains("num_error"));
        assert!(!json.contains("metadata"));
        let back: BatchObject = serde_json::from_str(&json).unwrap();
        assert_eq!(batch, back);
    }

    #[test]
    fn batch_results_response_with_pagination_token() {
        let resp = BatchResultsResponse {
            data: vec![BatchResultItem {
                custom_id: Some("req-1".into()),
                response: Some(serde_json::json!({"id": "resp_1"})),
                error: None,
                status_code: Some(200),
                cost_in_usd_ticks: None,
            }],
            has_more: Some(true),
            pagination_token: Some("tok_next_page".into()),
        };
        let json = serde_json::to_string(&resp).unwrap();
        let back: BatchResultsResponse = serde_json::from_str(&json).unwrap();
        assert_eq!(resp, back);
        assert_eq!(back.pagination_token.as_deref(), Some("tok_next_page"));
        assert_eq!(back.has_more, Some(true));
    }

    #[test]
    fn batch_results_response_without_pagination() {
        let resp = BatchResultsResponse {
            data: vec![],
            has_more: Some(false),
            pagination_token: None,
        };
        let json = serde_json::to_string(&resp).unwrap();
        assert!(!json.contains("pagination_token"));
        let back: BatchResultsResponse = serde_json::from_str(&json).unwrap();
        assert_eq!(back.has_more, Some(false));
        assert!(back.pagination_token.is_none());
    }

    #[test]
    fn batch_request_item_with_chat_payload() {
        let item = BatchRequestItem {
            custom_id: Some("chat-req-1".into()),
            method: "POST".into(),
            url: "/v1/chat/completions".into(),
            body: serde_json::json!({
                "model": "grok-4",
                "messages": [{"role": "user", "content": "Hello"}]
            }),
        };
        let json = serde_json::to_string(&item).unwrap();
        let back: BatchRequestItem = serde_json::from_str(&json).unwrap();
        assert_eq!(back.custom_id.as_deref(), Some("chat-req-1"));
        assert_eq!(back.method, "POST");
        assert_eq!(back.url, "/v1/chat/completions");
        assert_eq!(back.body["model"], serde_json::json!("grok-4"));
    }

    #[test]
    fn batch_request_item_with_responses_payload() {
        let item = BatchRequestItem {
            custom_id: Some("resp-req-1".into()),
            method: "POST".into(),
            url: "/v1/responses".into(),
            body: serde_json::json!({
                "model": "grok-4",
                "input": "What is 2+2?"
            }),
        };
        let json = serde_json::to_string(&item).unwrap();
        let back: BatchRequestItem = serde_json::from_str(&json).unwrap();
        assert_eq!(back.custom_id.as_deref(), Some("resp-req-1"));
        assert_eq!(back.url, "/v1/responses");
        assert_eq!(back.body["input"], serde_json::json!("What is 2+2?"));
    }

    #[test]
    fn batch_request_item_without_custom_id() {
        let item = BatchRequestItem {
            custom_id: None,
            method: "POST".into(),
            url: "/v1/responses".into(),
            body: serde_json::json!({"model": "grok-4", "input": "Hi"}),
        };
        let json = serde_json::to_string(&item).unwrap();
        assert!(!json.contains("custom_id"));
    }

    #[test]
    fn add_batch_requests_payload_serializes() {
        let payload = AddBatchRequestsPayload {
            requests: vec![
                BatchRequestItem {
                    custom_id: Some("r1".into()),
                    method: "POST".into(),
                    url: "/v1/responses".into(),
                    body: serde_json::json!({"model": "grok-4", "input": "a"}),
                },
                BatchRequestItem {
                    custom_id: Some("r2".into()),
                    method: "POST".into(),
                    url: "/v1/responses".into(),
                    body: serde_json::json!({"model": "grok-4", "input": "b"}),
                },
            ],
        };
        let json = serde_json::to_string(&payload).unwrap();
        assert!(json.contains("\"requests\""));
        let back: AddBatchRequestsPayload = serde_json::from_str(&json).unwrap();
        assert_eq!(back.requests.len(), 2);
    }

    #[test]
    fn batch_result_item_with_error() {
        let item = BatchResultItem {
            custom_id: Some("err-req".into()),
            response: None,
            error: Some(serde_json::json!({"message": "rate limited", "type": "rate_limit_error"})),
            status_code: Some(429),
            cost_in_usd_ticks: None,
        };
        let json = serde_json::to_string(&item).unwrap();
        let back: BatchResultItem = serde_json::from_str(&json).unwrap();
        assert_eq!(back.status_code, Some(429));
        assert!(back.error.is_some());
        assert!(back.response.is_none());
    }

    #[test]
    fn batch_list_round_trips() {
        let list = BatchList {
            data: vec![
                BatchObject {
                    id: "batch_1".into(),
                    object: Some("batch".into()),
                    endpoint: None,
                    status: Some(BatchStatus::Completed),
                    created_at: Some(1_700_000_000),
                    completed_at: Some(1_700_000_100),
                    num_pending: Some(0),
                    num_success: Some(10),
                    num_error: Some(0),
                    metadata: None,
                },
                BatchObject {
                    id: "batch_2".into(),
                    object: Some("batch".into()),
                    endpoint: None,
                    status: Some(BatchStatus::Created),
                    created_at: Some(1_700_000_200),
                    completed_at: None,
                    num_pending: None,
                    num_success: None,
                    num_error: None,
                    metadata: None,
                },
            ],
            has_more: Some(false),
        };
        let json = serde_json::to_string(&list).unwrap();
        let back: BatchList = serde_json::from_str(&json).unwrap();
        assert_eq!(list, back);
        assert_eq!(back.data.len(), 2);
    }

    #[test]
    fn batch_list_empty() {
        let list = BatchList {
            data: vec![],
            has_more: None,
        };
        let json = serde_json::to_string(&list).unwrap();
        assert!(json.contains("\"data\":[]"));
        assert!(!json.contains("has_more"));
        let back: BatchList = serde_json::from_str(&json).unwrap();
        assert!(back.data.is_empty());
    }

    #[test]
    fn batch_result_item_deserializes_with_unknown_fields() {
        let json = r#"{
            "custom_id": "r1",
            "response": {"id": "resp_1"},
            "status_code": 200,
            "future_field": true
        }"#;
        let item: BatchResultItem = serde_json::from_str(json).unwrap();
        assert_eq!(item.custom_id.as_deref(), Some("r1"));
        assert_eq!(item.status_code, Some(200));
    }

    #[test]
    fn batch_results_response_deserializes_with_unknown_fields() {
        let json = r#"{
            "data": [],
            "has_more": false,
            "pagination_token": null,
            "some_extra_field": "ignored"
        }"#;
        let resp: BatchResultsResponse = serde_json::from_str(json).unwrap();
        assert!(resp.data.is_empty());
        assert_eq!(resp.has_more, Some(false));
    }

    // -----------------------------------------------------------------------
    // BatchError tests
    // -----------------------------------------------------------------------

    #[test]
    fn batch_error_display_transport() {
        let err = BatchError::Transport(TransportError::Timeout);
        let display = format!("{err}");
        assert!(display.contains("timed out"));
    }

    #[test]
    fn batch_error_display_pagination_limit() {
        let err = BatchError::PaginationLimitExceeded {
            items_collected: 500,
            max_pages: 100,
        };
        let display = format!("{err}");
        assert!(display.contains("500"));
        assert!(display.contains("100"));
        assert!(display.contains("pagination limit exceeded"));
    }

    #[test]
    fn batch_error_from_transport_error() {
        let transport_err = TransportError::Timeout;
        let batch_err: BatchError = transport_err.into();
        match batch_err {
            BatchError::Transport(TransportError::Timeout) => {}
            other => panic!("expected Transport(Timeout), got: {other}"),
        }
    }

    // -----------------------------------------------------------------------
    // Typed constructor tests (Issue 4)
    // -----------------------------------------------------------------------

    #[test]
    #[allow(deprecated)]
    fn chat_completion_constructor_sets_method_and_url() {
        use crate::types::chat::ChatCompletionRequest;
        use crate::types::common::Role;
        use crate::types::message::Message;

        #[allow(deprecated)]
        let req = ChatCompletionRequest {
            model: "grok-4".into(),
            messages: vec![Message::text(Role::User, "Hello")],
            tools: None,
            tool_choice: None,
            stream: None,
            temperature: None,
            top_p: None,
            max_completion_tokens: None,
            n: None,
            stop: None,
            seed: None,
            frequency_penalty: None,
            presence_penalty: None,
            response_format: None,
            reasoning_effort: None,
            search_parameters: None,
            deferred: None,
            stream_options: None,
            parallel_tool_calls: None,
        };

        let item = BatchRequestItem::chat_completion(Some("chat-1".into()), &req);
        assert_eq!(item.method, "POST");
        assert_eq!(item.url, "/v1/chat/completions");
        assert_eq!(item.custom_id.as_deref(), Some("chat-1"));
        assert_eq!(item.body["model"], "grok-4");
    }

    #[test]
    fn response_constructor_sets_method_and_url() {
        use crate::types::responses::{CreateResponseRequest, ResponseInput};

        let req = CreateResponseRequest {
            model: "grok-4".into(),
            input: ResponseInput::Text("What is 2+2?".into()),
            instructions: None,
            tools: None,
            tool_choice: None,
            previous_response_id: None,
            store: None,
            stream: None,
            temperature: None,
            top_p: None,
            max_output_tokens: None,
            max_turns: None,
            reasoning: None,
            text: None,
            search_parameters: None,
            metadata: None,
            parallel_tool_calls: None,
            include: None,
            context_management: None,
            prompt_cache_key: None,
        };

        let item = BatchRequestItem::response(Some("resp-1".into()), &req);
        assert_eq!(item.method, "POST");
        assert_eq!(item.url, "/v1/responses");
        assert_eq!(item.custom_id.as_deref(), Some("resp-1"));
        assert_eq!(item.body["model"], "grok-4");
    }

    #[test]
    fn response_constructor_without_custom_id() {
        use crate::types::responses::{CreateResponseRequest, ResponseInput};

        let req = CreateResponseRequest {
            model: "grok-4".into(),
            input: ResponseInput::Text("Hi".into()),
            instructions: None,
            tools: None,
            tool_choice: None,
            previous_response_id: None,
            store: None,
            stream: None,
            temperature: None,
            top_p: None,
            max_output_tokens: None,
            max_turns: None,
            reasoning: None,
            text: None,
            search_parameters: None,
            metadata: None,
            parallel_tool_calls: None,
            include: None,
            context_management: None,
            prompt_cache_key: None,
        };

        let item = BatchRequestItem::response(None, &req);
        assert!(item.custom_id.is_none());
        assert_eq!(item.method, "POST");
        assert_eq!(item.url, "/v1/responses");
    }

    // -----------------------------------------------------------------------
    // cost_in_usd_ticks tests (Issue 5)
    // -----------------------------------------------------------------------

    #[test]
    fn batch_result_item_with_cost_in_usd_ticks() {
        let json = r#"{
            "custom_id": "r1",
            "response": {"id": "resp_1"},
            "status_code": 200,
            "cost_in_usd_ticks": 42000
        }"#;
        let item: BatchResultItem = serde_json::from_str(json).unwrap();
        assert_eq!(item.cost_in_usd_ticks, Some(42000));
    }

    #[test]
    fn batch_result_item_without_cost_in_usd_ticks() {
        let json = r#"{
            "custom_id": "r1",
            "response": {"id": "resp_1"},
            "status_code": 200
        }"#;
        let item: BatchResultItem = serde_json::from_str(json).unwrap();
        assert!(item.cost_in_usd_ticks.is_none());
    }

    #[test]
    fn batch_result_item_cost_in_usd_ticks_skips_none_on_serialize() {
        let item = BatchResultItem {
            custom_id: Some("r1".into()),
            response: None,
            error: None,
            status_code: Some(200),
            cost_in_usd_ticks: None,
        };
        let json = serde_json::to_string(&item).unwrap();
        assert!(!json.contains("cost_in_usd_ticks"));
    }

    #[test]
    fn batch_result_item_cost_in_usd_ticks_round_trips() {
        let item = BatchResultItem {
            custom_id: Some("r1".into()),
            response: Some(serde_json::json!({"id": "resp_1"})),
            error: None,
            status_code: Some(200),
            cost_in_usd_ticks: Some(12345),
        };
        let json = serde_json::to_string(&item).unwrap();
        let back: BatchResultItem = serde_json::from_str(&json).unwrap();
        assert_eq!(back.cost_in_usd_ticks, Some(12345));
        assert_eq!(item, back);
    }

    #[test]
    fn batch_result_item_negative_cost_in_usd_ticks() {
        // i64 allows negative values; ensure deserialization handles it
        let json = r#"{
            "custom_id": "r1",
            "status_code": 200,
            "cost_in_usd_ticks": -100
        }"#;
        let item: BatchResultItem = serde_json::from_str(json).unwrap();
        assert_eq!(item.cost_in_usd_ticks, Some(-100));
    }

    // -----------------------------------------------------------------------
    // U91: BatchEndpoint image/video variants
    // -----------------------------------------------------------------------

    #[test]
    fn batch_endpoint_image_generations_round_trips() {
        let ep = BatchEndpoint::ImageGenerations;
        let json = serde_json::to_string(&ep).unwrap();
        assert_eq!(json, r#""/v1/images/generations""#);
        let back: BatchEndpoint = serde_json::from_str(&json).unwrap();
        assert_eq!(back, BatchEndpoint::ImageGenerations);
    }

    #[test]
    fn batch_endpoint_image_edits_round_trips() {
        let ep = BatchEndpoint::ImageEdits;
        let json = serde_json::to_string(&ep).unwrap();
        assert_eq!(json, r#""/v1/images/edits""#);
        let back: BatchEndpoint = serde_json::from_str(&json).unwrap();
        assert_eq!(back, BatchEndpoint::ImageEdits);
    }

    #[test]
    fn batch_endpoint_video_generations_round_trips() {
        let ep = BatchEndpoint::VideoGenerations;
        let json = serde_json::to_string(&ep).unwrap();
        assert_eq!(json, r#""/v1/videos/generations""#);
        let back: BatchEndpoint = serde_json::from_str(&json).unwrap();
        assert_eq!(back, BatchEndpoint::VideoGenerations);
    }

    // -----------------------------------------------------------------------
    // U91: BatchRequestItem image/video constructors
    // -----------------------------------------------------------------------

    #[test]
    fn image_generation_constructor_sets_method_and_url() {
        use crate::types::images::ImageGenerationRequest;

        let req = ImageGenerationRequest {
            prompt: "A cat in space".into(),
            model: "grok-2-image".into(),
            n: Some(1),
            aspect_ratio: None,
            quality: None,
            resolution: None,
            response_format: None,
        };

        let item = BatchRequestItem::image_generation(Some("img-1".into()), &req);
        assert_eq!(item.method, "POST");
        assert_eq!(item.url, "/v1/images/generations");
        assert_eq!(item.custom_id.as_deref(), Some("img-1"));
        assert_eq!(item.body["prompt"], "A cat in space");
        assert_eq!(item.body["model"], "grok-2-image");
    }

    #[test]
    fn image_edit_constructor_sets_method_and_url() {
        use crate::types::images::ImageEditRequest;

        let req = ImageEditRequest {
            prompt: "Remove background".into(),
            model: "grok-2-image".into(),
            image: Some("https://example.com/photo.jpg".into()),
            images: None,
            mask: None,
        };

        let item = BatchRequestItem::image_edit(Some("edit-1".into()), &req);
        assert_eq!(item.method, "POST");
        assert_eq!(item.url, "/v1/images/edits");
        assert_eq!(item.custom_id.as_deref(), Some("edit-1"));
        assert_eq!(item.body["prompt"], "Remove background");
        assert_eq!(item.body["model"], "grok-2-image");
    }

    #[test]
    fn video_generation_constructor_sets_method_and_url() {
        use crate::types::videos::VideoGenerationRequest;

        let req = VideoGenerationRequest {
            prompt: "A sunset timelapse".into(),
            model: Some("grok-2-video".into()),
            image: None,
            reference_images: None,
            duration: None,
            aspect_ratio: None,
            resolution: None,
        };

        let item = BatchRequestItem::video_generation(Some("vid-1".into()), &req);
        assert_eq!(item.method, "POST");
        assert_eq!(item.url, "/v1/videos/generations");
        assert_eq!(item.custom_id.as_deref(), Some("vid-1"));
        assert_eq!(item.body["prompt"], "A sunset timelapse");
        assert_eq!(item.body["model"], "grok-2-video");
    }

    #[test]
    fn image_generation_constructor_without_custom_id() {
        use crate::types::images::ImageGenerationRequest;

        let req = ImageGenerationRequest {
            prompt: "A dog".into(),
            model: "grok-2-image".into(),
            n: None,
            aspect_ratio: None,
            quality: None,
            resolution: None,
            response_format: None,
        };

        let item = BatchRequestItem::image_generation(None, &req);
        assert!(item.custom_id.is_none());
        assert_eq!(item.method, "POST");
        assert_eq!(item.url, "/v1/images/generations");
    }

    #[test]
    fn video_generation_constructor_without_custom_id() {
        use crate::types::videos::VideoGenerationRequest;

        let req = VideoGenerationRequest {
            prompt: "Waves crashing".into(),
            model: None,
            image: None,
            reference_images: None,
            duration: None,
            aspect_ratio: None,
            resolution: None,
        };

        let item = BatchRequestItem::video_generation(None, &req);
        assert!(item.custom_id.is_none());
        assert_eq!(item.method, "POST");
        assert_eq!(item.url, "/v1/videos/generations");
    }
}
