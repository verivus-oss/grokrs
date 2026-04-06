//! Wire types for the xAI Video Generation API.
//!
//! These types map directly to the JSON request/response bodies of the
//! `/v1/videos/generations`, `/v1/videos/edits`, and `/v1/videos/extensions`
//! endpoints. Video generation is asynchronous: the submit endpoints return
//! a `request_id` which must be polled until the video is ready.

use std::fmt;

use serde::{Deserialize, Serialize};

use super::images::AspectRatio;
use crate::transport::error::TransportError;

// ---------------------------------------------------------------------------
// Errors
// ---------------------------------------------------------------------------

/// Errors from video type validation.
#[derive(Debug)]
pub enum VideoError {
    /// The requested generation duration is outside the valid range (1-15s).
    InvalidDuration {
        /// The duration value that was rejected.
        seconds: u32,
        /// The minimum allowed duration.
        min: u32,
        /// The maximum allowed duration.
        max: u32,
    },
    /// The requested extension duration is outside the valid range (1-10s).
    InvalidExtensionDuration {
        /// The duration value that was rejected.
        seconds: u32,
        /// The minimum allowed duration.
        min: u32,
        /// The maximum allowed duration.
        max: u32,
    },
    /// Polling exceeded the maximum number of attempts.
    PollTimeout {
        /// The number of polls that were attempted.
        polls: u32,
    },
    /// The video generation failed on the server side.
    GenerationFailed {
        /// The request ID of the failed generation.
        request_id: String,
    },
    /// An HTTP/API transport error occurred during a video operation.
    Transport(TransportError),
}

impl fmt::Display for VideoError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            VideoError::InvalidDuration { seconds, min, max } => {
                write!(
                    f,
                    "invalid video duration: {seconds}s (must be {min}-{max}s)"
                )
            }
            VideoError::InvalidExtensionDuration { seconds, min, max } => {
                write!(
                    f,
                    "invalid video extension duration: {seconds}s (must be {min}-{max}s)"
                )
            }
            VideoError::PollTimeout { polls } => {
                write!(
                    f,
                    "video poll timed out after {polls} attempts without completion"
                )
            }
            VideoError::GenerationFailed { request_id } => {
                write!(f, "video generation failed for request {request_id}")
            }
            VideoError::Transport(err) => {
                write!(f, "video transport error: {err}")
            }
        }
    }
}

impl std::error::Error for VideoError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            VideoError::Transport(err) => Some(err),
            _ => None,
        }
    }
}

impl From<TransportError> for VideoError {
    fn from(err: TransportError) -> Self {
        VideoError::Transport(err)
    }
}

// ---------------------------------------------------------------------------
// Duration newtypes
// ---------------------------------------------------------------------------

/// A validated video generation duration (1-15 seconds).
///
/// Constructed via `VideoDuration::new()`, which rejects out-of-range values.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize)]
#[serde(transparent)]
pub struct VideoDuration(u32);

impl VideoDuration {
    /// Minimum allowed generation duration in seconds.
    pub const MIN: u32 = 1;
    /// Maximum allowed generation duration in seconds.
    pub const MAX: u32 = 15;

    /// Create a new `VideoDuration`, validating that `seconds` is in [1, 15].
    pub fn new(seconds: u32) -> Result<Self, VideoError> {
        if !(Self::MIN..=Self::MAX).contains(&seconds) {
            return Err(VideoError::InvalidDuration {
                seconds,
                min: Self::MIN,
                max: Self::MAX,
            });
        }
        Ok(Self(seconds))
    }

    /// Return the duration in seconds.
    #[must_use]
    pub fn seconds(&self) -> u32 {
        self.0
    }
}

impl<'de> Deserialize<'de> for VideoDuration {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let value = u32::deserialize(deserializer)?;
        VideoDuration::new(value).map_err(serde::de::Error::custom)
    }
}

/// A validated video extension duration (1-10 seconds).
///
/// Constructed via `VideoExtensionDuration::new()`, which rejects out-of-range
/// values.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize)]
#[serde(transparent)]
pub struct VideoExtensionDuration(u32);

impl VideoExtensionDuration {
    /// Minimum allowed extension duration in seconds.
    pub const MIN: u32 = 1;
    /// Maximum allowed extension duration in seconds.
    pub const MAX: u32 = 10;

    /// Create a new `VideoExtensionDuration`, validating that `seconds` is in
    /// [1, 10].
    pub fn new(seconds: u32) -> Result<Self, VideoError> {
        if !(Self::MIN..=Self::MAX).contains(&seconds) {
            return Err(VideoError::InvalidExtensionDuration {
                seconds,
                min: Self::MIN,
                max: Self::MAX,
            });
        }
        Ok(Self(seconds))
    }

    /// Return the duration in seconds.
    #[must_use]
    pub fn seconds(&self) -> u32 {
        self.0
    }
}

impl<'de> Deserialize<'de> for VideoExtensionDuration {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let value = u32::deserialize(deserializer)?;
        VideoExtensionDuration::new(value).map_err(serde::de::Error::custom)
    }
}

// ---------------------------------------------------------------------------
// Enums
// ---------------------------------------------------------------------------

/// Status of an asynchronous video generation job.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum VideoStatus {
    /// The job is queued or in progress.
    Pending,
    /// The job completed successfully.
    Done,
    /// The job failed.
    Failed,
}

// ---------------------------------------------------------------------------
// Request types
// ---------------------------------------------------------------------------

/// Request body for `POST /v1/videos/generations`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct VideoGenerationRequest {
    /// The text description of the desired video.
    pub prompt: String,

    /// The model ID to use.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,

    /// A source image to use as the first frame (URL or base64).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub image: Option<String>,

    /// Reference images that influence the style or content (URLs or base64).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reference_images: Option<Vec<String>>,

    /// Duration of the generated video in seconds (1-15).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub duration: Option<VideoDuration>,

    /// Aspect ratio for the generated video.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub aspect_ratio: Option<AspectRatio>,

    /// Resolution for the generated video (e.g., `"720p"`, `"1080p"`).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub resolution: Option<String>,
}

/// Request body for `POST /v1/videos/edits`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct VideoEditRequest {
    /// The text description of the desired edit.
    pub prompt: String,

    /// The source video to edit (URL or identifier).
    pub video: String,

    /// The model ID to use.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
}

/// Request body for `POST /v1/videos/extensions`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct VideoExtensionRequest {
    /// The text description guiding the extension.
    pub prompt: String,

    /// The source video to extend (URL or identifier).
    pub video: String,

    /// Duration to extend by in seconds (1-10).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub duration: Option<VideoExtensionDuration>,

    /// The model ID to use.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
}

// ---------------------------------------------------------------------------
// Response types
// ---------------------------------------------------------------------------

/// Response from an asynchronous video submit endpoint.
///
/// Contains the `request_id` used to poll for the result.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct VideoSubmitResponse {
    /// The opaque identifier for polling the video result.
    pub request_id: String,
}

/// Response from polling a video generation job.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct VideoPollResponse {
    /// Current status of the generation job.
    pub status: VideoStatus,

    /// The completed video result, present when `status` is `Done`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub video: Option<VideoResult>,

    /// Generation progress as a percentage (0-100).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub progress: Option<u32>,
}

/// A completed video result.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct VideoResult {
    /// The URL to download the generated video.
    pub url: String,
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    // -- VideoDuration -------------------------------------------------------

    #[test]
    fn video_duration_valid_min() {
        let d = VideoDuration::new(1).unwrap();
        assert_eq!(d.seconds(), 1);
    }

    #[test]
    fn video_duration_valid_max() {
        let d = VideoDuration::new(15).unwrap();
        assert_eq!(d.seconds(), 15);
    }

    #[test]
    fn video_duration_valid_mid() {
        let d = VideoDuration::new(8).unwrap();
        assert_eq!(d.seconds(), 8);
    }

    #[test]
    fn video_duration_rejects_zero() {
        let err = VideoDuration::new(0).unwrap_err();
        match err {
            VideoError::InvalidDuration { seconds, min, max } => {
                assert_eq!(seconds, 0);
                assert_eq!(min, 1);
                assert_eq!(max, 15);
            }
            other => panic!("expected InvalidDuration, got: {other}"),
        }
    }

    #[test]
    fn video_duration_rejects_16() {
        let err = VideoDuration::new(16).unwrap_err();
        match err {
            VideoError::InvalidDuration { seconds, .. } => {
                assert_eq!(seconds, 16);
            }
            other => panic!("expected InvalidDuration, got: {other}"),
        }
    }

    #[test]
    fn video_duration_serializes_as_number() {
        let d = VideoDuration::new(5).unwrap();
        let json = serde_json::to_string(&d).unwrap();
        assert_eq!(json, "5");
    }

    #[test]
    fn video_duration_deserializes_from_number() {
        let d: VideoDuration = serde_json::from_str("10").unwrap();
        assert_eq!(d.seconds(), 10);
    }

    #[test]
    fn video_duration_deserialize_rejects_zero() {
        let result = serde_json::from_str::<VideoDuration>("0");
        assert!(result.is_err());
    }

    #[test]
    fn video_duration_deserialize_rejects_16() {
        let result = serde_json::from_str::<VideoDuration>("16");
        assert!(result.is_err());
    }

    #[test]
    fn video_duration_deserialize_valid_5() {
        let d = serde_json::from_str::<VideoDuration>("5").unwrap();
        assert_eq!(d.seconds(), 5);
    }

    #[test]
    fn video_duration_deserialize_valid_boundaries() {
        let d1 = serde_json::from_str::<VideoDuration>("1").unwrap();
        assert_eq!(d1.seconds(), 1);
        let d15 = serde_json::from_str::<VideoDuration>("15").unwrap();
        assert_eq!(d15.seconds(), 15);
    }

    // -- VideoExtensionDuration ----------------------------------------------

    #[test]
    fn video_extension_duration_valid_min() {
        let d = VideoExtensionDuration::new(1).unwrap();
        assert_eq!(d.seconds(), 1);
    }

    #[test]
    fn video_extension_duration_valid_max() {
        let d = VideoExtensionDuration::new(10).unwrap();
        assert_eq!(d.seconds(), 10);
    }

    #[test]
    fn video_extension_duration_rejects_zero() {
        let err = VideoExtensionDuration::new(0).unwrap_err();
        match err {
            VideoError::InvalidExtensionDuration { seconds, min, max } => {
                assert_eq!(seconds, 0);
                assert_eq!(min, 1);
                assert_eq!(max, 10);
            }
            other => panic!("expected InvalidExtensionDuration, got: {other}"),
        }
    }

    #[test]
    fn video_extension_duration_rejects_11() {
        let err = VideoExtensionDuration::new(11).unwrap_err();
        match err {
            VideoError::InvalidExtensionDuration { seconds, .. } => {
                assert_eq!(seconds, 11);
            }
            other => panic!("expected InvalidExtensionDuration, got: {other}"),
        }
    }

    #[test]
    fn video_extension_duration_deserialize_rejects_zero() {
        let result = serde_json::from_str::<VideoExtensionDuration>("0");
        assert!(result.is_err());
    }

    #[test]
    fn video_extension_duration_deserialize_rejects_11() {
        let result = serde_json::from_str::<VideoExtensionDuration>("11");
        assert!(result.is_err());
    }

    #[test]
    fn video_extension_duration_deserialize_valid_5() {
        let d = serde_json::from_str::<VideoExtensionDuration>("5").unwrap();
        assert_eq!(d.seconds(), 5);
    }

    #[test]
    fn video_extension_duration_deserialize_valid_boundaries() {
        let d1 = serde_json::from_str::<VideoExtensionDuration>("1").unwrap();
        assert_eq!(d1.seconds(), 1);
        let d10 = serde_json::from_str::<VideoExtensionDuration>("10").unwrap();
        assert_eq!(d10.seconds(), 10);
    }

    // -- VideoStatus ---------------------------------------------------------

    #[test]
    fn video_status_serializes_pending() {
        let json = serde_json::to_string(&VideoStatus::Pending).unwrap();
        assert_eq!(json, "\"pending\"");
    }

    #[test]
    fn video_status_serializes_done() {
        let json = serde_json::to_string(&VideoStatus::Done).unwrap();
        assert_eq!(json, "\"done\"");
    }

    #[test]
    fn video_status_serializes_failed() {
        let json = serde_json::to_string(&VideoStatus::Failed).unwrap();
        assert_eq!(json, "\"failed\"");
    }

    // -- Poll response -------------------------------------------------------

    #[test]
    fn video_poll_response_pending_with_progress() {
        let json = r#"{
            "status": "pending",
            "progress": 42
        }"#;
        let resp: VideoPollResponse = serde_json::from_str(json).unwrap();
        assert_eq!(resp.status, VideoStatus::Pending);
        assert_eq!(resp.progress, Some(42));
        assert!(resp.video.is_none());
    }

    #[test]
    fn video_poll_response_done_with_video_url() {
        let json = r#"{
            "status": "done",
            "video": {
                "url": "https://example.com/video.mp4"
            }
        }"#;
        let resp: VideoPollResponse = serde_json::from_str(json).unwrap();
        assert_eq!(resp.status, VideoStatus::Done);
        let video = resp.video.unwrap();
        assert_eq!(video.url, "https://example.com/video.mp4");
        assert!(resp.progress.is_none());
    }

    #[test]
    fn video_poll_response_failed() {
        let json = r#"{"status": "failed"}"#;
        let resp: VideoPollResponse = serde_json::from_str(json).unwrap();
        assert_eq!(resp.status, VideoStatus::Failed);
    }

    #[test]
    fn video_poll_response_tolerates_unknown_fields() {
        let json = r#"{
            "status": "pending",
            "progress": 10,
            "extra_field": "ignored"
        }"#;
        let resp: VideoPollResponse = serde_json::from_str(json).unwrap();
        assert_eq!(resp.status, VideoStatus::Pending);
    }

    // -- Submit response -----------------------------------------------------

    #[test]
    fn video_submit_response_deserializes() {
        let json = r#"{"request_id": "vid_abc123"}"#;
        let resp: VideoSubmitResponse = serde_json::from_str(json).unwrap();
        assert_eq!(resp.request_id, "vid_abc123");
    }

    // -- Request serialization -----------------------------------------------

    #[test]
    fn video_generation_request_skips_none_fields() {
        let req = VideoGenerationRequest {
            prompt: "A sunset over the ocean".to_string(),
            model: None,
            image: None,
            reference_images: None,
            duration: None,
            aspect_ratio: None,
            resolution: None,
        };
        let json = serde_json::to_string(&req).unwrap();
        assert!(json.contains("\"prompt\""));
        assert!(!json.contains("\"model\""));
        assert!(!json.contains("\"image\""));
        assert!(!json.contains("\"duration\""));
        assert!(!json.contains("\"aspect_ratio\""));
        assert!(!json.contains("\"resolution\""));
    }

    #[test]
    fn video_generation_request_with_duration() {
        let req = VideoGenerationRequest {
            prompt: "A bird flying".to_string(),
            model: Some("grok-2-video".to_string()),
            image: None,
            reference_images: None,
            duration: Some(VideoDuration::new(5).unwrap()),
            aspect_ratio: Some(AspectRatio::Wide),
            resolution: None,
        };
        let json = serde_json::to_string(&req).unwrap();
        assert!(json.contains("\"duration\":5"));
        assert!(json.contains("\"aspect_ratio\":\"16:9\""));
    }

    #[test]
    fn video_edit_request_serializes() {
        let req = VideoEditRequest {
            prompt: "Add rain".to_string(),
            video: "https://example.com/video.mp4".to_string(),
            model: Some("grok-2-video".to_string()),
        };
        let json = serde_json::to_string(&req).unwrap();
        assert!(json.contains("\"prompt\":\"Add rain\""));
        assert!(json.contains("\"video\":\"https://example.com/video.mp4\""));
    }

    #[test]
    fn video_extension_request_serializes() {
        let req = VideoExtensionRequest {
            prompt: "Continue the scene".to_string(),
            video: "https://example.com/video.mp4".to_string(),
            duration: Some(VideoExtensionDuration::new(5).unwrap()),
            model: None,
        };
        let json = serde_json::to_string(&req).unwrap();
        assert!(json.contains("\"duration\":5"));
        assert!(!json.contains("\"model\""));
    }

    // -- Error display -------------------------------------------------------

    #[test]
    fn video_error_display_invalid_duration() {
        let err = VideoError::InvalidDuration {
            seconds: 20,
            min: 1,
            max: 15,
        };
        let display = format!("{err}");
        assert!(display.contains("20s"));
        assert!(display.contains("1-15s"));
    }

    #[test]
    fn video_error_display_invalid_extension_duration() {
        let err = VideoError::InvalidExtensionDuration {
            seconds: 12,
            min: 1,
            max: 10,
        };
        let display = format!("{err}");
        assert!(display.contains("12s"));
        assert!(display.contains("1-10s"));
    }

    #[test]
    fn video_error_display_poll_timeout() {
        let err = VideoError::PollTimeout { polls: 100 };
        let display = format!("{err}");
        assert!(display.contains("100"));
    }

    #[test]
    fn video_error_display_generation_failed() {
        let err = VideoError::GenerationFailed {
            request_id: "vid_xyz".to_string(),
        };
        let display = format!("{err}");
        assert!(display.contains("vid_xyz"));
    }

    #[test]
    fn video_error_is_std_error() {
        let err = VideoError::PollTimeout { polls: 1 };
        let _: &dyn std::error::Error = &err;
    }
}
