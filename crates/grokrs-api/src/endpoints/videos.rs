//! Video Generation API endpoint client.
//!
//! This module provides `VideosClient`, which wraps `HttpClient` to send
//! requests to the xAI `/v1/videos/generations`, `/v1/videos/edits`, and
//! `/v1/videos/extensions` endpoints. Video generation is asynchronous:
//! submit endpoints return a `request_id` which must be polled.

use std::sync::Arc;
use std::time::Duration;

use reqwest::Method;

use crate::transport::client::HttpClient;
use crate::transport::error::TransportError;
use crate::types::videos::{
    VideoEditRequest, VideoError, VideoExtensionRequest, VideoGenerationRequest, VideoPollResponse,
    VideoStatus, VideoSubmitResponse,
};

use super::util::encode_path_segment;

/// The path for the video generation endpoint.
const VIDEO_GENERATIONS_PATH: &str = "/v1/videos/generations";

/// The path for the video edit endpoint.
const VIDEO_EDITS_PATH: &str = "/v1/videos/edits";

/// The path for the video extension endpoint.
const VIDEO_EXTENSIONS_PATH: &str = "/v1/videos/extensions";

/// The path prefix for polling video results.
const VIDEO_POLL_PATH_PREFIX: &str = "/v1/videos/";

/// A client for the xAI Video Generation API.
///
/// Holds a shared reference to an `HttpClient` and provides typed methods for
/// generating, editing, and extending videos. All mutating operations are
/// asynchronous: they return a `request_id` that must be polled via `poll()`
/// or `poll_until_done()`.
#[derive(Debug, Clone)]
pub struct VideosClient {
    http: Arc<HttpClient>,
}

impl VideosClient {
    /// Create a new `VideosClient` from a shared `HttpClient`.
    #[must_use]
    pub fn new(http: Arc<HttpClient>) -> Self {
        Self { http }
    }

    /// Submit a video generation request.
    ///
    /// Issues `POST /v1/videos/generations` with the given request body.
    /// Returns a `VideoSubmitResponse` containing the `request_id` for
    /// subsequent polling.
    pub async fn generate(
        &self,
        request: &VideoGenerationRequest,
    ) -> Result<VideoSubmitResponse, TransportError> {
        self.http
            .send_json(Method::POST, VIDEO_GENERATIONS_PATH, request)
            .await
    }

    /// Submit a video edit request.
    ///
    /// Issues `POST /v1/videos/edits` with the given request body.
    /// Returns a `VideoSubmitResponse` containing the `request_id` for
    /// subsequent polling.
    pub async fn edit(
        &self,
        request: &VideoEditRequest,
    ) -> Result<VideoSubmitResponse, TransportError> {
        self.http
            .send_json(Method::POST, VIDEO_EDITS_PATH, request)
            .await
    }

    /// Submit a video extension request.
    ///
    /// Issues `POST /v1/videos/extensions` with the given request body.
    /// Returns a `VideoSubmitResponse` containing the `request_id` for
    /// subsequent polling.
    pub async fn extend(
        &self,
        request: &VideoExtensionRequest,
    ) -> Result<VideoSubmitResponse, TransportError> {
        self.http
            .send_json(Method::POST, VIDEO_EXTENSIONS_PATH, request)
            .await
    }

    /// Poll the status of a video generation job.
    ///
    /// Issues `GET /v1/videos/{request_id}`. The `request_id` is
    /// percent-encoded to prevent URL corruption.
    pub async fn poll(&self, request_id: &str) -> Result<VideoPollResponse, TransportError> {
        let path = format!(
            "{}{}",
            VIDEO_POLL_PATH_PREFIX,
            encode_path_segment(request_id)
        );
        self.http.send_no_body(Method::GET, &path).await
    }

    /// Poll a video generation job until it completes or the poll limit is
    /// reached.
    ///
    /// Sleeps for `poll_interval` between each poll. Returns when:
    /// - The status is `Done` (returns the final `VideoPollResponse`)
    /// - The status is `Failed` (returns `VideoError::GenerationFailed`)
    /// - `max_polls` is exceeded (returns `VideoError::PollTimeout`)
    ///
    /// This method has a bounded loop: it will never poll more than
    /// `max_polls` times, preventing runaway polling.
    pub async fn poll_until_done(
        &self,
        request_id: &str,
        poll_interval: Duration,
        max_polls: u32,
    ) -> Result<VideoPollResponse, VideoError> {
        for poll_count in 1..=max_polls {
            let response = self.poll(request_id).await.map_err(VideoError::Transport)?;

            match response.status {
                VideoStatus::Done => return Ok(response),
                VideoStatus::Failed => {
                    return Err(VideoError::GenerationFailed {
                        request_id: request_id.to_string(),
                    });
                }
                VideoStatus::Pending => {
                    if poll_count < max_polls {
                        tokio::time::sleep(poll_interval).await;
                    }
                }
            }
        }

        Err(VideoError::PollTimeout { polls: max_polls })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::transport::auth::ApiKeySecret;
    use crate::transport::client::HttpClientConfig;
    use crate::transport::policy_gate::AllowAllGate;
    use crate::types::images::AspectRatio;
    use crate::types::videos::{VideoDuration, VideoExtensionDuration};
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

    // -- Path construction ---------------------------------------------------

    #[test]
    fn generate_path_is_correct() {
        assert_eq!(VIDEO_GENERATIONS_PATH, "/v1/videos/generations");
    }

    #[test]
    fn edit_path_is_correct() {
        assert_eq!(VIDEO_EDITS_PATH, "/v1/videos/edits");
    }

    #[test]
    fn extend_path_is_correct() {
        assert_eq!(VIDEO_EXTENSIONS_PATH, "/v1/videos/extensions");
    }

    #[test]
    fn poll_path_encodes_normal_id() {
        let request_id = "vid_abc123";
        let path = format!(
            "{}{}",
            VIDEO_POLL_PATH_PREFIX,
            encode_path_segment(request_id)
        );
        assert_eq!(path, "/v1/videos/vid_abc123");
    }

    #[test]
    fn poll_path_encodes_slash() {
        let request_id = "vid/abc";
        let path = format!(
            "{}{}",
            VIDEO_POLL_PATH_PREFIX,
            encode_path_segment(request_id)
        );
        assert_eq!(path, "/v1/videos/vid%2Fabc");
    }

    #[test]
    fn poll_path_encodes_query_chars() {
        let request_id = "vid?v=1";
        let path = format!(
            "{}{}",
            VIDEO_POLL_PATH_PREFIX,
            encode_path_segment(request_id)
        );
        assert_eq!(path, "/v1/videos/vid%3Fv%3D1");
    }

    #[test]
    fn poll_path_encodes_space() {
        let request_id = "vid abc";
        let path = format!(
            "{}{}",
            VIDEO_POLL_PATH_PREFIX,
            encode_path_segment(request_id)
        );
        assert_eq!(path, "/v1/videos/vid%20abc");
    }

    #[test]
    fn poll_path_encodes_hash() {
        let request_id = "vid#frag";
        let path = format!(
            "{}{}",
            VIDEO_POLL_PATH_PREFIX,
            encode_path_segment(request_id)
        );
        assert_eq!(path, "/v1/videos/vid%23frag");
    }

    // -- Endpoint integration tests ------------------------------------------

    #[tokio::test]
    async fn generate_sends_post_and_returns_request_id() {
        let server = MockServer::start().await;
        let response_body = serde_json::json!({
            "request_id": "vid_gen_001"
        });

        Mock::given(method("POST"))
            .and(path("/v1/videos/generations"))
            .and(header("authorization", "Bearer test-key"))
            .and(body_partial_json(serde_json::json!({
                "prompt": "A sunset over the ocean"
            })))
            .respond_with(ResponseTemplate::new(200).set_body_json(&response_body))
            .expect(1)
            .mount(&server)
            .await;

        let http = mock_http_client(&server.uri());
        let client = VideosClient::new(http);

        let request = VideoGenerationRequest {
            prompt: "A sunset over the ocean".to_string(),
            model: Some("grok-2-video".to_string()),
            image: None,
            reference_images: None,
            duration: Some(VideoDuration::new(5).unwrap()),
            aspect_ratio: Some(AspectRatio::Wide),
            resolution: None,
        };

        let resp = client.generate(&request).await.unwrap();
        assert_eq!(resp.request_id, "vid_gen_001");
    }

    #[tokio::test]
    async fn edit_sends_post_and_returns_request_id() {
        let server = MockServer::start().await;
        let response_body = serde_json::json!({
            "request_id": "vid_edit_001"
        });

        Mock::given(method("POST"))
            .and(path("/v1/videos/edits"))
            .and(header("authorization", "Bearer test-key"))
            .respond_with(ResponseTemplate::new(200).set_body_json(&response_body))
            .expect(1)
            .mount(&server)
            .await;

        let http = mock_http_client(&server.uri());
        let client = VideosClient::new(http);

        let request = VideoEditRequest {
            prompt: "Add rain effect".to_string(),
            video: "https://example.com/video.mp4".to_string(),
            model: None,
        };

        let resp = client.edit(&request).await.unwrap();
        assert_eq!(resp.request_id, "vid_edit_001");
    }

    #[tokio::test]
    async fn extend_sends_post_and_returns_request_id() {
        let server = MockServer::start().await;
        let response_body = serde_json::json!({
            "request_id": "vid_ext_001"
        });

        Mock::given(method("POST"))
            .and(path("/v1/videos/extensions"))
            .and(header("authorization", "Bearer test-key"))
            .respond_with(ResponseTemplate::new(200).set_body_json(&response_body))
            .expect(1)
            .mount(&server)
            .await;

        let http = mock_http_client(&server.uri());
        let client = VideosClient::new(http);

        let request = VideoExtensionRequest {
            prompt: "Continue the scene".to_string(),
            video: "https://example.com/video.mp4".to_string(),
            duration: Some(VideoExtensionDuration::new(5).unwrap()),
            model: None,
        };

        let resp = client.extend(&request).await.unwrap();
        assert_eq!(resp.request_id, "vid_ext_001");
    }

    #[tokio::test]
    async fn poll_sends_get_and_returns_pending() {
        let server = MockServer::start().await;
        let response_body = serde_json::json!({
            "status": "pending",
            "progress": 35
        });

        Mock::given(method("GET"))
            .and(path("/v1/videos/vid_gen_001"))
            .and(header("authorization", "Bearer test-key"))
            .respond_with(ResponseTemplate::new(200).set_body_json(&response_body))
            .expect(1)
            .mount(&server)
            .await;

        let http = mock_http_client(&server.uri());
        let client = VideosClient::new(http);

        let resp = client.poll("vid_gen_001").await.unwrap();
        assert_eq!(resp.status, VideoStatus::Pending);
        assert_eq!(resp.progress, Some(35));
        assert!(resp.video.is_none());
    }

    #[tokio::test]
    async fn poll_sends_get_and_returns_done() {
        let server = MockServer::start().await;
        let response_body = serde_json::json!({
            "status": "done",
            "video": {
                "url": "https://example.com/result.mp4"
            }
        });

        Mock::given(method("GET"))
            .and(path("/v1/videos/vid_gen_001"))
            .respond_with(ResponseTemplate::new(200).set_body_json(&response_body))
            .expect(1)
            .mount(&server)
            .await;

        let http = mock_http_client(&server.uri());
        let client = VideosClient::new(http);

        let resp = client.poll("vid_gen_001").await.unwrap();
        assert_eq!(resp.status, VideoStatus::Done);
        let video = resp.video.unwrap();
        assert_eq!(video.url, "https://example.com/result.mp4");
    }

    #[tokio::test]
    async fn poll_until_done_returns_on_done() {
        let server = MockServer::start().await;

        // First poll returns pending, second returns done.
        Mock::given(method("GET"))
            .and(path("/v1/videos/vid_poll_ok"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "status": "pending",
                "progress": 50
            })))
            .up_to_n_times(1)
            .mount(&server)
            .await;

        Mock::given(method("GET"))
            .and(path("/v1/videos/vid_poll_ok"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "status": "done",
                "video": {"url": "https://example.com/done.mp4"}
            })))
            .mount(&server)
            .await;

        let http = mock_http_client(&server.uri());
        let client = VideosClient::new(http);

        let resp = client
            .poll_until_done("vid_poll_ok", Duration::from_millis(10), 5)
            .await
            .unwrap();
        assert_eq!(resp.status, VideoStatus::Done);
        assert_eq!(resp.video.unwrap().url, "https://example.com/done.mp4");
    }

    #[tokio::test]
    async fn poll_until_done_returns_error_on_failed() {
        let server = MockServer::start().await;

        Mock::given(method("GET"))
            .and(path("/v1/videos/vid_poll_fail"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "status": "failed"
            })))
            .mount(&server)
            .await;

        let http = mock_http_client(&server.uri());
        let client = VideosClient::new(http);

        let err = client
            .poll_until_done("vid_poll_fail", Duration::from_millis(10), 5)
            .await
            .unwrap_err();
        match err {
            VideoError::GenerationFailed { request_id } => {
                assert_eq!(request_id, "vid_poll_fail");
            }
            other => panic!("expected GenerationFailed, got: {other}"),
        }
    }

    #[tokio::test]
    async fn poll_until_done_returns_timeout_when_max_polls_exceeded() {
        let server = MockServer::start().await;

        // Always return pending.
        Mock::given(method("GET"))
            .and(path("/v1/videos/vid_poll_timeout"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "status": "pending",
                "progress": 10
            })))
            .mount(&server)
            .await;

        let http = mock_http_client(&server.uri());
        let client = VideosClient::new(http);

        let err = client
            .poll_until_done("vid_poll_timeout", Duration::from_millis(10), 3)
            .await
            .unwrap_err();
        match err {
            VideoError::PollTimeout { polls } => {
                assert_eq!(polls, 3);
            }
            other => panic!("expected PollTimeout, got: {other}"),
        }
    }

    #[tokio::test]
    async fn generate_returns_error_on_4xx() {
        let server = MockServer::start().await;
        let error_body = serde_json::json!({
            "error": {
                "message": "Invalid video request",
                "type": "invalid_request_error"
            }
        });

        Mock::given(method("POST"))
            .and(path("/v1/videos/generations"))
            .respond_with(ResponseTemplate::new(400).set_body_json(&error_body))
            .expect(1)
            .mount(&server)
            .await;

        let http = mock_http_client(&server.uri());
        let client = VideosClient::new(http);

        let request = VideoGenerationRequest {
            prompt: String::new(),
            model: None,
            image: None,
            reference_images: None,
            duration: None,
            aspect_ratio: None,
            resolution: None,
        };

        let err = client.generate(&request).await.unwrap_err();
        let display = format!("{err}");
        assert!(display.contains("Invalid video request"));
    }
}
