use std::sync::Arc;

use reqwest::Method;

use crate::transport::client::HttpClient;
use crate::transport::error::TransportError;
use crate::types::batches::{
    AddBatchRequestsPayload, BatchEndpoint, BatchError, BatchList, BatchObject, BatchRequestItem,
    BatchResultItem, BatchResultsResponse, CreateBatchRequest,
};

use super::util::encode_path_segment;

/// API path prefix for the Batches endpoint.
const BATCHES_PATH: &str = "/v1/batches";

/// Upper-case hex digits for percent-encoding.
const HEX_UPPER: [u8; 16] = *b"0123456789ABCDEF";

/// Percent-encode a query parameter value.
///
/// Encodes characters that are unsafe in a query value per RFC 3986.
/// Unreserved characters (alphanumerics, `-`, `.`, `_`, `~`) are passed
/// through verbatim; everything else is percent-encoded. Notably, `&`, `=`,
/// `+`, `%`, `?`, `#`, and spaces are all encoded to prevent query string
/// injection.
fn encode_query_value(value: &str) -> String {
    let mut encoded = String::with_capacity(value.len());
    for byte in value.bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'.' | b'_' | b'~' => {
                encoded.push(byte as char);
            }
            _ => {
                encoded.push('%');
                encoded.push(char::from(HEX_UPPER[(byte >> 4) as usize]));
                encoded.push(char::from(HEX_UPPER[(byte & 0x0F) as usize]));
            }
        }
    }
    encoded
}

/// Maximum number of pagination pages to fetch in `collect_all_results`.
///
/// Prevents unbounded loops if the server keeps returning `has_more: true`
/// due to a bug or extremely large batch.
const MAX_PAGINATION_PAGES: usize = 100;

/// Client for the xAI Batch API.
///
/// Wraps an `Arc<HttpClient>` and exposes typed methods for batch lifecycle
/// operations: create, add requests, check status, retrieve results, cancel,
/// and list. Pagination for results is handled both at the single-page level
/// (`results`) and as a convenience helper (`collect_all_results`).
#[derive(Debug, Clone)]
pub struct BatchesClient {
    http: Arc<HttpClient>,
}

impl BatchesClient {
    /// Create a new `BatchesClient` from a shared `HttpClient`.
    #[must_use]
    pub fn new(http: Arc<HttpClient>) -> Self {
        Self { http }
    }

    /// Create a new batch — `POST /v1/batches`.
    ///
    /// Returns the newly created `BatchObject` with its assigned ID and
    /// initial status.
    ///
    /// # Errors
    ///
    /// Returns [`TransportError`] if the API request fails.
    pub async fn create(
        &self,
        request: &CreateBatchRequest,
    ) -> Result<BatchObject, TransportError> {
        self.http
            .send_json(Method::POST, BATCHES_PATH, request)
            .await
    }

    /// Create a batch from a previously uploaded JSONL file — `POST /v1/batches`.
    ///
    /// This is a convenience method that builds a [`CreateBatchRequest`] with
    /// `input_file_id` set to the given `file_id`. The file must have been
    /// uploaded via the Files API and contain one JSON request object per line.
    ///
    /// Returns the newly created `BatchObject` with its assigned ID and
    /// initial status.
    ///
    /// # Errors
    ///
    /// Returns [`TransportError`] if the API request fails.
    pub async fn create_from_file(
        &self,
        file_id: &str,
        endpoint: BatchEndpoint,
    ) -> Result<BatchObject, TransportError> {
        let request = CreateBatchRequest {
            endpoint,
            input_file_id: Some(file_id.to_owned()),
            metadata: None,
        };
        self.create(&request).await
    }

    /// Add requests to an existing batch — `POST /v1/batches/{id}/requests`.
    ///
    /// The `batch_id` is URL-encoded to prevent path traversal. The requests
    /// are sent as a JSON array wrapped in `AddBatchRequestsPayload`.
    ///
    /// The server may return 200 with an empty body, 204 No Content, or a JSON
    /// acknowledgment. We ignore the response body entirely via `send_json_empty`.
    ///
    /// # Errors
    ///
    /// Returns [`TransportError`] if the API request fails.
    pub async fn add_requests(
        &self,
        batch_id: &str,
        requests: &[BatchRequestItem],
    ) -> Result<(), TransportError> {
        let path = format!(
            "{}/{}/requests",
            BATCHES_PATH,
            encode_path_segment(batch_id)
        );
        let payload = AddBatchRequestsPayload {
            requests: requests.to_vec(),
        };
        self.http
            .send_json_empty(Method::POST, &path, &payload)
            .await
    }

    /// Get the status of a batch — `GET /v1/batches/{id}`.
    ///
    /// Returns the `BatchObject` with current progress counts and status.
    ///
    /// # Errors
    ///
    /// Returns [`TransportError`] if the API request fails.
    pub async fn status(&self, batch_id: &str) -> Result<BatchObject, TransportError> {
        let path = format!("{}/{}", BATCHES_PATH, encode_path_segment(batch_id));
        self.http.send_no_body(Method::GET, &path).await
    }

    /// Retrieve a single page of results — `GET /v1/batches/{id}/results`.
    ///
    /// If `pagination_token` is provided, it is appended as a URL-encoded
    /// query parameter to fetch the next page. Check `has_more` and
    /// `pagination_token` on the response to determine whether more pages exist.
    ///
    /// # Errors
    ///
    /// Returns [`TransportError`] if the API request fails.
    pub async fn results(
        &self,
        batch_id: &str,
        pagination_token: Option<&str>,
    ) -> Result<BatchResultsResponse, TransportError> {
        let encoded_id = encode_path_segment(batch_id);
        let path = match pagination_token {
            Some(token) => {
                let encoded_token = encode_query_value(token);
                format!("{BATCHES_PATH}/{encoded_id}/results?pagination_token={encoded_token}")
            }
            None => format!("{BATCHES_PATH}/{encoded_id}/results"),
        };
        self.http.send_no_body(Method::GET, &path).await
    }

    /// Cancel a batch — `POST /v1/batches/{id}:cancel`.
    ///
    /// Returns the updated `BatchObject` reflecting the cancelled state.
    ///
    /// # Errors
    ///
    /// Returns [`TransportError`] if the API request fails.
    pub async fn cancel(&self, batch_id: &str) -> Result<BatchObject, TransportError> {
        let path = format!(
            "{}{}:cancel",
            BATCHES_PATH,
            // The colon-suffixed action `:cancel` sits after the encoded ID,
            // so we build: /v1/batches/<encoded_id>:cancel
            format_args!("/{}", encode_path_segment(batch_id))
        );
        // Cancel is a POST with no request body. We send an empty JSON object
        // to satisfy the `send_json` signature.
        self.http
            .send_json(Method::POST, &path, &serde_json::json!({}))
            .await
    }

    /// List all batches — `GET /v1/batches`.
    ///
    /// Returns a `BatchList` containing all batches visible to the caller.
    ///
    /// # Errors
    ///
    /// Returns [`TransportError`] if the API request fails.
    pub async fn list(&self) -> Result<BatchList, TransportError> {
        self.http.send_no_body(Method::GET, BATCHES_PATH).await
    }

    /// Collect all result items across all pages for a batch.
    ///
    /// Calls `results()` repeatedly, following `pagination_token` until
    /// `has_more` is `false` or the token is `None`. Bounded to
    /// [`MAX_PAGINATION_PAGES`] pages to prevent unbounded loops.
    ///
    /// Returns the concatenated list of all `BatchResultItem`s, or a
    /// [`BatchError::PaginationLimitExceeded`] error if the page limit is
    /// reached while more results remain. This fail-closed behavior ensures
    /// callers cannot accidentally treat partial data as complete.
    ///
    /// # Errors
    ///
    /// Returns [`BatchError::Transport`] if any underlying API request fails.
    /// Returns [`BatchError::PaginationLimitExceeded`] if the maximum page
    /// count is reached while more results remain.
    pub async fn collect_all_results(
        &self,
        batch_id: &str,
    ) -> Result<Vec<BatchResultItem>, BatchError> {
        let mut all_items = Vec::new();
        let mut token: Option<String> = None;

        // RATIONALE: MAX_PAGINATION_PAGES is a compile-time constant (100),
        // well within u32 range.
        #[allow(clippy::cast_possible_truncation)]
        let max_pages_u32 = MAX_PAGINATION_PAGES as u32;

        for page_num in 0..MAX_PAGINATION_PAGES {
            let page = self.results(batch_id, token.as_deref()).await?;

            all_items.extend(page.data);

            match (page.has_more, page.pagination_token) {
                (Some(true), Some(next_token)) if !next_token.is_empty() => {
                    // If this is the last allowed page and there are still more
                    // results, return an error rather than silent truncation.
                    if page_num + 1 >= MAX_PAGINATION_PAGES {
                        return Err(BatchError::PaginationLimitExceeded {
                            items_collected: all_items.len(),
                            max_pages: max_pages_u32,
                        });
                    }
                    token = Some(next_token);
                }
                // Fail-closed: if the server says there are more results but
                // doesn't provide a valid pagination token, return an error
                // rather than silently returning partial data.
                (Some(true), _) => {
                    return Err(BatchError::PaginationLimitExceeded {
                        items_collected: all_items.len(),
                        max_pages: max_pages_u32,
                    });
                }
                // No more results — we have the complete set.
                _ => break,
            }
        }

        Ok(all_items)
    }

    /// Return the HTTP path used for create and list operations.
    ///
    /// Useful for testing and diagnostics.
    #[must_use]
    pub fn base_path() -> &'static str {
        BATCHES_PATH
    }

    /// Return the HTTP path for a specific batch resource.
    ///
    /// Useful for testing and diagnostics.
    #[must_use]
    pub fn resource_path(batch_id: &str) -> String {
        format!("{}/{}", BATCHES_PATH, encode_path_segment(batch_id))
    }

    /// Return the HTTP path for adding requests to a batch.
    ///
    /// Useful for testing and diagnostics.
    #[must_use]
    pub fn requests_path(batch_id: &str) -> String {
        format!(
            "{}/{}/requests",
            BATCHES_PATH,
            encode_path_segment(batch_id)
        )
    }

    /// Return the HTTP path for retrieving batch results.
    ///
    /// Useful for testing and diagnostics.
    #[must_use]
    pub fn results_path(batch_id: &str) -> String {
        format!("{}/{}/results", BATCHES_PATH, encode_path_segment(batch_id))
    }

    /// Return the HTTP path for cancelling a batch.
    ///
    /// Useful for testing and diagnostics.
    #[must_use]
    pub fn cancel_path(batch_id: &str) -> String {
        format!("{}/{}:cancel", BATCHES_PATH, encode_path_segment(batch_id))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // -----------------------------------------------------------------------
    // Path construction tests
    // -----------------------------------------------------------------------

    #[test]
    fn base_path_is_correct() {
        assert_eq!(BatchesClient::base_path(), "/v1/batches");
    }

    #[test]
    fn resource_path_simple_id() {
        assert_eq!(
            BatchesClient::resource_path("batch_abc123"),
            "/v1/batches/batch_abc123"
        );
    }

    #[test]
    fn resource_path_encodes_slash() {
        assert_eq!(
            BatchesClient::resource_path("batch/abc"),
            "/v1/batches/batch%2Fabc"
        );
    }

    #[test]
    fn resource_path_encodes_space() {
        assert_eq!(
            BatchesClient::resource_path("batch abc"),
            "/v1/batches/batch%20abc"
        );
    }

    #[test]
    fn resource_path_encodes_query_chars() {
        assert_eq!(
            BatchesClient::resource_path("batch?v=1"),
            "/v1/batches/batch%3Fv%3D1"
        );
    }

    #[test]
    fn resource_path_encodes_hash() {
        assert_eq!(
            BatchesClient::resource_path("batch#frag"),
            "/v1/batches/batch%23frag"
        );
    }

    #[test]
    fn requests_path_construction() {
        assert_eq!(
            BatchesClient::requests_path("batch_xyz"),
            "/v1/batches/batch_xyz/requests"
        );
    }

    #[test]
    fn requests_path_encodes_id() {
        assert_eq!(
            BatchesClient::requests_path("batch/special"),
            "/v1/batches/batch%2Fspecial/requests"
        );
    }

    #[test]
    fn results_path_construction() {
        assert_eq!(
            BatchesClient::results_path("batch_res"),
            "/v1/batches/batch_res/results"
        );
    }

    #[test]
    fn results_path_encodes_id() {
        assert_eq!(
            BatchesClient::results_path("batch?id"),
            "/v1/batches/batch%3Fid/results"
        );
    }

    #[test]
    fn cancel_path_construction() {
        assert_eq!(
            BatchesClient::cancel_path("batch_cancel"),
            "/v1/batches/batch_cancel:cancel"
        );
    }

    #[test]
    fn cancel_path_encodes_id() {
        assert_eq!(
            BatchesClient::cancel_path("batch/cancel"),
            "/v1/batches/batch%2Fcancel:cancel"
        );
    }

    // -----------------------------------------------------------------------
    // Wiremock integration tests
    // -----------------------------------------------------------------------

    use crate::transport::auth::ApiKeySecret;
    use crate::transport::client::HttpClientConfig;
    use crate::transport::policy_gate::AllowAllGate;
    use crate::types::batches::{BatchEndpoint, BatchStatus};
    use wiremock::matchers::{method, path, query_param};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    /// Helper to create a `BatchesClient` backed by a wiremock server.
    fn make_client(server: &MockServer) -> BatchesClient {
        let config = HttpClientConfig {
            base_url: server.uri(),
            ..Default::default()
        };
        let http = HttpClient::new(
            config,
            ApiKeySecret::new("test-key"),
            Some(Arc::new(AllowAllGate)),
        )
        .unwrap();
        BatchesClient::new(Arc::new(http))
    }

    #[tokio::test]
    async fn create_returns_batch_object() {
        let server = MockServer::start().await;

        let body = serde_json::json!({
            "id": "batch_new_1",
            "object": "batch",
            "status": "created",
            "endpoint": "/v1/responses",
            "created_at": 1_700_000_000
        });

        Mock::given(method("POST"))
            .and(path("/v1/batches"))
            .respond_with(ResponseTemplate::new(200).set_body_json(&body))
            .expect(1)
            .mount(&server)
            .await;

        let client = make_client(&server);
        let req = CreateBatchRequest {
            endpoint: BatchEndpoint::Responses,
            input_file_id: None,
            metadata: None,
        };
        let result = client.create(&req).await.unwrap();
        assert_eq!(result.id, "batch_new_1");
        assert_eq!(result.status, Some(BatchStatus::Created));
        assert_eq!(result.endpoint.as_deref(), Some("/v1/responses"));
    }

    #[tokio::test]
    async fn status_returns_batch_object_with_counts() {
        let server = MockServer::start().await;

        let body = serde_json::json!({
            "id": "batch_status_1",
            "object": "batch",
            "status": "in_progress",
            "num_pending": 3,
            "num_success": 7,
            "num_error": 1,
            "created_at": 1_700_000_000
        });

        Mock::given(method("GET"))
            .and(path("/v1/batches/batch_status_1"))
            .respond_with(ResponseTemplate::new(200).set_body_json(&body))
            .expect(1)
            .mount(&server)
            .await;

        let client = make_client(&server);
        let result = client.status("batch_status_1").await.unwrap();
        assert_eq!(result.id, "batch_status_1");
        assert_eq!(result.status, Some(BatchStatus::InProgress));
        assert_eq!(result.num_pending, Some(3));
        assert_eq!(result.num_success, Some(7));
        assert_eq!(result.num_error, Some(1));
    }

    #[tokio::test]
    async fn results_with_pagination() {
        let server = MockServer::start().await;

        // Page 1: has_more = true
        let page1 = serde_json::json!({
            "data": [
                {"custom_id": "r1", "status_code": 200, "response": {"id": "resp_1"}}
            ],
            "has_more": true,
            "pagination_token": "tok_page2"
        });

        // Mount the more specific mock (with query_param) LAST so wiremock
        // checks it first, preventing the less-specific mock from greedily
        // matching paginated requests.
        Mock::given(method("GET"))
            .and(path("/v1/batches/batch_pag/results"))
            .respond_with(ResponseTemplate::new(200).set_body_json(&page1))
            .up_to_n_times(1)
            .mount(&server)
            .await;

        // Page 2: has_more = false
        let page2 = serde_json::json!({
            "data": [
                {"custom_id": "r2", "status_code": 200, "response": {"id": "resp_2"}}
            ],
            "has_more": false
        });

        let client = make_client(&server);

        // Fetch first page manually (no pagination token)
        let first = client.results("batch_pag", None).await.unwrap();
        assert_eq!(first.data.len(), 1);
        assert_eq!(first.has_more, Some(true));
        assert_eq!(first.pagination_token.as_deref(), Some("tok_page2"));

        // Mount page 2 mock after page 1 is consumed
        Mock::given(method("GET"))
            .and(path("/v1/batches/batch_pag/results"))
            .and(query_param("pagination_token", "tok_page2"))
            .respond_with(ResponseTemplate::new(200).set_body_json(&page2))
            .expect(1)
            .mount(&server)
            .await;

        // Fetch second page
        let second = client
            .results("batch_pag", Some("tok_page2"))
            .await
            .unwrap();
        assert_eq!(second.data.len(), 1);
        assert_eq!(second.has_more, Some(false));
    }

    #[tokio::test]
    async fn collect_all_results_paginates() {
        let server = MockServer::start().await;

        // Page 1
        let page1 = serde_json::json!({
            "data": [
                {"custom_id": "a1", "status_code": 200, "response": {}}
            ],
            "has_more": true,
            "pagination_token": "next1"
        });

        Mock::given(method("GET"))
            .and(path("/v1/batches/batch_all/results"))
            .respond_with(ResponseTemplate::new(200).set_body_json(&page1))
            .up_to_n_times(1)
            .mount(&server)
            .await;

        // Page 2 (final)
        let page2 = serde_json::json!({
            "data": [
                {"custom_id": "a2", "status_code": 200, "response": {}},
                {"custom_id": "a3", "status_code": 200, "response": {}}
            ],
            "has_more": false
        });

        Mock::given(method("GET"))
            .and(path("/v1/batches/batch_all/results"))
            .and(query_param("pagination_token", "next1"))
            .respond_with(ResponseTemplate::new(200).set_body_json(&page2))
            .expect(1)
            .mount(&server)
            .await;

        let client = make_client(&server);
        let all = client.collect_all_results("batch_all").await.unwrap();
        assert_eq!(all.len(), 3);
        assert_eq!(all[0].custom_id.as_deref(), Some("a1"));
        assert_eq!(all[1].custom_id.as_deref(), Some("a2"));
        assert_eq!(all[2].custom_id.as_deref(), Some("a3"));
    }

    #[tokio::test]
    async fn cancel_returns_batch_object() {
        let server = MockServer::start().await;

        let body = serde_json::json!({
            "id": "batch_cancel_1",
            "object": "batch",
            "status": "cancelled"
        });

        Mock::given(method("POST"))
            .and(path("/v1/batches/batch_cancel_1:cancel"))
            .respond_with(ResponseTemplate::new(200).set_body_json(&body))
            .expect(1)
            .mount(&server)
            .await;

        let client = make_client(&server);
        let result = client.cancel("batch_cancel_1").await.unwrap();
        assert_eq!(result.id, "batch_cancel_1");
        assert_eq!(result.status, Some(BatchStatus::Cancelled));
    }

    #[tokio::test]
    async fn list_returns_batch_list() {
        let server = MockServer::start().await;

        let body = serde_json::json!({
            "data": [
                {"id": "batch_l1", "status": "completed"},
                {"id": "batch_l2", "status": "in_progress"}
            ],
            "has_more": false
        });

        Mock::given(method("GET"))
            .and(path("/v1/batches"))
            .respond_with(ResponseTemplate::new(200).set_body_json(&body))
            .expect(1)
            .mount(&server)
            .await;

        let client = make_client(&server);
        let result = client.list().await.unwrap();
        assert_eq!(result.data.len(), 2);
        assert_eq!(result.data[0].id, "batch_l1");
        assert_eq!(result.data[0].status, Some(BatchStatus::Completed));
        assert_eq!(result.data[1].id, "batch_l2");
        assert_eq!(result.data[1].status, Some(BatchStatus::InProgress));
        assert_eq!(result.has_more, Some(false));
    }

    #[tokio::test]
    async fn add_requests_sends_post() {
        let server = MockServer::start().await;

        Mock::given(method("POST"))
            .and(path("/v1/batches/batch_add_1/requests"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({})))
            .expect(1)
            .mount(&server)
            .await;

        let client = make_client(&server);
        let items = vec![
            BatchRequestItem {
                custom_id: Some("req-1".into()),
                method: "POST".into(),
                url: "/v1/responses".into(),
                body: serde_json::json!({"model": "grok-4", "input": "Hi"}),
            },
            BatchRequestItem {
                custom_id: Some("req-2".into()),
                method: "POST".into(),
                url: "/v1/responses".into(),
                body: serde_json::json!({"model": "grok-4", "input": "Hello"}),
            },
        ];

        let result = client.add_requests("batch_add_1", &items).await;
        assert!(result.is_ok());

        // Verify the request body
        let received = server.received_requests().await.unwrap();
        assert_eq!(received.len(), 1);
        let body: serde_json::Value = serde_json::from_slice(&received[0].body).unwrap();
        assert_eq!(body["requests"].as_array().unwrap().len(), 2);
        assert_eq!(body["requests"][0]["custom_id"], "req-1");
    }

    #[tokio::test]
    async fn create_returns_error_on_4xx() {
        let server = MockServer::start().await;

        let error_body = serde_json::json!({
            "error": {
                "message": "Invalid endpoint",
                "type": "invalid_request_error"
            }
        });

        Mock::given(method("POST"))
            .and(path("/v1/batches"))
            .respond_with(ResponseTemplate::new(400).set_body_json(&error_body))
            .expect(1)
            .mount(&server)
            .await;

        let client = make_client(&server);
        let req = CreateBatchRequest {
            endpoint: BatchEndpoint::ChatCompletions,
            input_file_id: None,
            metadata: None,
        };
        let result = client.create(&req).await;
        assert!(result.is_err());
        match result.unwrap_err() {
            TransportError::Api(api_err) => {
                assert_eq!(api_err.status_code, 400);
                assert!(api_err.message.contains("Invalid endpoint"));
            }
            other => panic!("expected Api error, got: {other}"),
        }
    }

    #[tokio::test]
    async fn status_returns_error_on_404() {
        let server = MockServer::start().await;

        let error_body = serde_json::json!({
            "error": {
                "message": "Batch not found",
                "type": "not_found_error"
            }
        });

        Mock::given(method("GET"))
            .and(path("/v1/batches/nonexistent"))
            .respond_with(ResponseTemplate::new(404).set_body_json(&error_body))
            .expect(1)
            .mount(&server)
            .await;

        let client = make_client(&server);
        let result = client.status("nonexistent").await;
        assert!(result.is_err());
        match result.unwrap_err() {
            TransportError::Api(api_err) => {
                assert_eq!(api_err.status_code, 404);
                assert!(api_err.message.contains("Batch not found"));
            }
            other => panic!("expected Api error, got: {other}"),
        }
    }

    #[tokio::test]
    async fn cancel_returns_error_on_5xx() {
        let server = MockServer::start().await;

        let error_body = serde_json::json!({
            "error": {
                "message": "Internal server error",
                "type": "server_error"
            }
        });

        Mock::given(method("POST"))
            .and(path("/v1/batches/batch_err:cancel"))
            .respond_with(ResponseTemplate::new(500).set_body_json(&error_body))
            .expect(1)
            .mount(&server)
            .await;

        let client = make_client(&server);
        let result = client.cancel("batch_err").await;
        assert!(result.is_err());
        match result.unwrap_err() {
            TransportError::Api(api_err) => {
                assert_eq!(api_err.status_code, 500);
            }
            other => panic!("expected Api error, got: {other}"),
        }
    }

    #[tokio::test]
    async fn add_requests_url_encodes_batch_id() {
        let server = MockServer::start().await;

        Mock::given(method("POST"))
            .and(path("/v1/batches/batch%2Fwith%2Fslashes/requests"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({})))
            .expect(1)
            .mount(&server)
            .await;

        let client = make_client(&server);
        let result = client
            .add_requests(
                "batch/with/slashes",
                &[BatchRequestItem {
                    custom_id: None,
                    method: "POST".into(),
                    url: "/v1/responses".into(),
                    body: serde_json::json!({"model": "grok-4", "input": "test"}),
                }],
            )
            .await;
        assert!(result.is_ok());
    }

    // -----------------------------------------------------------------------
    // Issue 1: add_requests handles 204 No Content
    // -----------------------------------------------------------------------

    #[tokio::test]
    async fn add_requests_handles_204_no_content() {
        let server = MockServer::start().await;

        Mock::given(method("POST"))
            .and(path("/v1/batches/batch_204/requests"))
            .respond_with(ResponseTemplate::new(204))
            .expect(1)
            .mount(&server)
            .await;

        let client = make_client(&server);
        let result = client
            .add_requests(
                "batch_204",
                &[BatchRequestItem {
                    custom_id: Some("req-1".into()),
                    method: "POST".into(),
                    url: "/v1/responses".into(),
                    body: serde_json::json!({"model": "grok-4", "input": "Hi"}),
                }],
            )
            .await;
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn add_requests_handles_200_empty_body() {
        let server = MockServer::start().await;

        Mock::given(method("POST"))
            .and(path("/v1/batches/batch_empty/requests"))
            .respond_with(ResponseTemplate::new(200))
            .expect(1)
            .mount(&server)
            .await;

        let client = make_client(&server);
        let result = client
            .add_requests(
                "batch_empty",
                &[BatchRequestItem {
                    custom_id: None,
                    method: "POST".into(),
                    url: "/v1/responses".into(),
                    body: serde_json::json!({"model": "grok-4", "input": "test"}),
                }],
            )
            .await;
        assert!(result.is_ok());
    }

    // -----------------------------------------------------------------------
    // Issue 2: pagination_token URL encoding
    // -----------------------------------------------------------------------

    #[tokio::test]
    async fn results_url_encodes_pagination_token() {
        let server = MockServer::start().await;

        // The token contains characters that need URL encoding: &, =, +, space
        let raw_token = "tok+with&special=chars space";

        let page = serde_json::json!({
            "data": [
                {"custom_id": "r1", "status_code": 200, "response": {}}
            ],
            "has_more": false
        });

        Mock::given(method("GET"))
            .and(path("/v1/batches/batch_enc/results"))
            .and(query_param(
                "pagination_token",
                "tok+with&special=chars space",
            ))
            .respond_with(ResponseTemplate::new(200).set_body_json(&page))
            .expect(1)
            .mount(&server)
            .await;

        let client = make_client(&server);
        let result = client.results("batch_enc", Some(raw_token)).await.unwrap();
        assert_eq!(result.data.len(), 1);
    }

    // -----------------------------------------------------------------------
    // Issue 2: encode_query_value unit tests
    // -----------------------------------------------------------------------

    #[test]
    fn encode_query_value_preserves_alphanumerics() {
        assert_eq!(encode_query_value("abc123"), "abc123");
    }

    #[test]
    fn encode_query_value_encodes_ampersand() {
        assert_eq!(encode_query_value("a&b"), "a%26b");
    }

    #[test]
    fn encode_query_value_encodes_equals() {
        assert_eq!(encode_query_value("a=b"), "a%3Db");
    }

    #[test]
    fn encode_query_value_encodes_plus() {
        assert_eq!(encode_query_value("a+b"), "a%2Bb");
    }

    #[test]
    fn encode_query_value_encodes_percent() {
        assert_eq!(encode_query_value("a%b"), "a%25b");
    }

    #[test]
    fn encode_query_value_encodes_space() {
        assert_eq!(encode_query_value("a b"), "a%20b");
    }

    #[test]
    fn encode_query_value_encodes_question_mark() {
        assert_eq!(encode_query_value("a?b"), "a%3Fb");
    }

    #[test]
    fn encode_query_value_encodes_hash() {
        assert_eq!(encode_query_value("a#b"), "a%23b");
    }

    #[test]
    fn encode_query_value_preserves_unreserved() {
        assert_eq!(encode_query_value("a-b.c_d~e"), "a-b.c_d~e");
    }

    #[test]
    fn encode_query_value_handles_empty() {
        assert_eq!(encode_query_value(""), "");
    }

    // -----------------------------------------------------------------------
    // Issue 3: collect_all_results returns BatchError
    // -----------------------------------------------------------------------

    #[tokio::test]
    async fn collect_all_results_returns_batch_error_on_transport_failure() {
        let server = MockServer::start().await;

        let error_body = serde_json::json!({
            "error": {
                "message": "not found",
                "type": "not_found_error"
            }
        });

        Mock::given(method("GET"))
            .and(path("/v1/batches/batch_fail/results"))
            .respond_with(ResponseTemplate::new(404).set_body_json(&error_body))
            .expect(1)
            .mount(&server)
            .await;

        let client = make_client(&server);
        let result = client.collect_all_results("batch_fail").await;
        assert!(result.is_err());
        match result.unwrap_err() {
            crate::types::batches::BatchError::Transport(TransportError::Api(api_err)) => {
                assert_eq!(api_err.status_code, 404);
            }
            other => panic!("expected Transport(Api) error, got: {other}"),
        }
    }

    // -----------------------------------------------------------------------
    // Issue U90: create_from_file
    // -----------------------------------------------------------------------

    #[tokio::test]
    async fn create_from_file_sends_input_file_id() {
        let server = MockServer::start().await;

        let body = serde_json::json!({
            "id": "batch_file_1",
            "object": "batch",
            "status": "created",
            "endpoint": "/v1/responses",
            "created_at": 1_700_000_000
        });

        Mock::given(method("POST"))
            .and(path("/v1/batches"))
            .respond_with(ResponseTemplate::new(200).set_body_json(&body))
            .expect(1)
            .mount(&server)
            .await;

        let client = make_client(&server);
        let result = client
            .create_from_file("file-upload-abc123", BatchEndpoint::Responses)
            .await
            .unwrap();
        assert_eq!(result.id, "batch_file_1");
        assert_eq!(result.status, Some(BatchStatus::Created));

        // Verify the request body contains input_file_id
        let received = server.received_requests().await.unwrap();
        assert_eq!(received.len(), 1);
        let req_body: serde_json::Value = serde_json::from_slice(&received[0].body).unwrap();
        assert_eq!(req_body["input_file_id"], "file-upload-abc123");
        assert_eq!(req_body["endpoint"], "/v1/responses");
    }

    #[tokio::test]
    async fn create_from_file_with_chat_completions_endpoint() {
        let server = MockServer::start().await;

        let body = serde_json::json!({
            "id": "batch_file_chat",
            "object": "batch",
            "status": "created",
            "endpoint": "/v1/chat/completions"
        });

        Mock::given(method("POST"))
            .and(path("/v1/batches"))
            .respond_with(ResponseTemplate::new(200).set_body_json(&body))
            .expect(1)
            .mount(&server)
            .await;

        let client = make_client(&server);
        let result = client
            .create_from_file("file-chat-xyz", BatchEndpoint::ChatCompletions)
            .await
            .unwrap();
        assert_eq!(result.id, "batch_file_chat");

        let received = server.received_requests().await.unwrap();
        let req_body: serde_json::Value = serde_json::from_slice(&received[0].body).unwrap();
        assert_eq!(req_body["input_file_id"], "file-chat-xyz");
        assert_eq!(req_body["endpoint"], "/v1/chat/completions");
    }
}
