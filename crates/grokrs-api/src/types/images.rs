//! Wire types for the xAI Image Generation API.
//!
//! These types map directly to the JSON request/response bodies of the
//! `/v1/images/generations` and `/v1/images/edits` endpoints.

use serde::{Deserialize, Serialize};

// ---------------------------------------------------------------------------
// Enums
// ---------------------------------------------------------------------------

/// Aspect ratio for generated images.
///
/// Each variant serializes to the exact string expected by the xAI API.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum AspectRatio {
    /// 1:1 — square.
    #[serde(rename = "1:1")]
    Square,
    /// 16:9 — wide landscape.
    #[serde(rename = "16:9")]
    Wide,
    /// 9:16 — tall portrait.
    #[serde(rename = "9:16")]
    Tall,
    /// 4:3 — standard landscape.
    #[serde(rename = "4:3")]
    Landscape,
    /// 3:4 — standard portrait.
    #[serde(rename = "3:4")]
    Portrait,
    /// 3:2 — classic landscape.
    #[serde(rename = "3:2")]
    ClassicLandscape,
    /// 2:3 — classic portrait.
    #[serde(rename = "2:3")]
    ClassicPortrait,
}

/// Image resolution for generated images.
///
/// Each variant serializes to the exact pixel dimensions string expected by the
/// xAI API.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum ImageResolution {
    /// 1024x1024 — square.
    #[serde(rename = "1024x1024")]
    Res1024x1024,
    /// 1024x1792 — tall portrait.
    #[serde(rename = "1024x1792")]
    Res1024x1792,
    /// 1792x1024 — wide landscape.
    #[serde(rename = "1792x1024")]
    Res1792x1024,
}

/// Response format for generated images.
///
/// Controls whether the API returns a URL to download the image or the raw
/// base64-encoded image data inline.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum ImageResponseFormat {
    /// Return a temporary URL to download the image.
    #[serde(rename = "url")]
    #[default]
    Url,
    /// Return the image data as a base64-encoded JSON string.
    #[serde(rename = "b64_json")]
    B64Json,
}

// ---------------------------------------------------------------------------
// Request types
// ---------------------------------------------------------------------------

/// Request body for `POST /v1/images/generations`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ImageGenerationRequest {
    /// The text description of the desired image.
    pub prompt: String,

    /// The model ID to use (e.g., `"grok-2-image"`).
    pub model: String,

    /// Number of images to generate (1-10).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub n: Option<u32>,

    /// Aspect ratio for the generated image.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub aspect_ratio: Option<AspectRatio>,

    /// Quality level for the generated image (e.g., `"standard"`, `"hd"`).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub quality: Option<String>,

    /// Pixel resolution for the generated image.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub resolution: Option<ImageResolution>,

    /// The format in which generated images are returned.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub response_format: Option<ImageResponseFormat>,
}

/// Request body for `POST /v1/images/edits`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ImageEditRequest {
    /// The text description of the desired edit.
    pub prompt: String,

    /// The model ID to use.
    pub model: String,

    /// A single source image (URL or base64).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub image: Option<String>,

    /// Multiple source images (URLs or base64).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub images: Option<Vec<String>>,

    /// A mask image indicating which regions to edit (URL or base64).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub mask: Option<String>,
}

// ---------------------------------------------------------------------------
// Response types
// ---------------------------------------------------------------------------

/// Response body from image generation or edit endpoints.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ImageResponse {
    /// Unix timestamp of when the response was created.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub created: Option<i64>,

    /// The generated image data entries.
    pub data: Vec<ImageData>,
}

/// A single generated image entry.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ImageData {
    /// A temporary URL to download the image.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,

    /// The image data as a base64-encoded string.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub b64_json: Option<String>,

    /// The prompt as revised by the model (may differ from the input prompt).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub revised_prompt: Option<String>,
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn image_generation_request_round_trips() {
        let req = ImageGenerationRequest {
            prompt: "A cat wearing a top hat".to_string(),
            model: "grok-2-image".to_string(),
            n: Some(2),
            aspect_ratio: Some(AspectRatio::Wide),
            quality: Some("hd".to_string()),
            resolution: Some(ImageResolution::Res1792x1024),
            response_format: Some(ImageResponseFormat::B64Json),
        };
        let json = serde_json::to_string(&req).unwrap();
        let deserialized: ImageGenerationRequest = serde_json::from_str(&json).unwrap();
        assert_eq!(req, deserialized);
    }

    #[test]
    fn image_generation_request_skips_none_fields() {
        let req = ImageGenerationRequest {
            prompt: "A dog".to_string(),
            model: "grok-2-image".to_string(),
            n: None,
            aspect_ratio: None,
            quality: None,
            resolution: None,
            response_format: None,
        };
        let json = serde_json::to_string(&req).unwrap();
        assert!(!json.contains("\"n\""));
        assert!(!json.contains("\"aspect_ratio\""));
        assert!(!json.contains("\"quality\""));
        assert!(!json.contains("\"resolution\""));
        assert!(!json.contains("\"response_format\""));
    }

    #[test]
    fn aspect_ratio_serializes_square() {
        let json = serde_json::to_string(&AspectRatio::Square).unwrap();
        assert_eq!(json, "\"1:1\"");
    }

    #[test]
    fn aspect_ratio_serializes_wide() {
        let json = serde_json::to_string(&AspectRatio::Wide).unwrap();
        assert_eq!(json, "\"16:9\"");
    }

    #[test]
    fn aspect_ratio_serializes_tall() {
        let json = serde_json::to_string(&AspectRatio::Tall).unwrap();
        assert_eq!(json, "\"9:16\"");
    }

    #[test]
    fn aspect_ratio_serializes_landscape() {
        let json = serde_json::to_string(&AspectRatio::Landscape).unwrap();
        assert_eq!(json, "\"4:3\"");
    }

    #[test]
    fn aspect_ratio_serializes_portrait() {
        let json = serde_json::to_string(&AspectRatio::Portrait).unwrap();
        assert_eq!(json, "\"3:4\"");
    }

    #[test]
    fn aspect_ratio_serializes_classic_landscape() {
        let json = serde_json::to_string(&AspectRatio::ClassicLandscape).unwrap();
        assert_eq!(json, "\"3:2\"");
    }

    #[test]
    fn aspect_ratio_serializes_classic_portrait() {
        let json = serde_json::to_string(&AspectRatio::ClassicPortrait).unwrap();
        assert_eq!(json, "\"2:3\"");
    }

    #[test]
    fn aspect_ratio_deserializes_from_string() {
        let ar: AspectRatio = serde_json::from_str("\"16:9\"").unwrap();
        assert_eq!(ar, AspectRatio::Wide);
    }

    #[test]
    fn image_resolution_serializes_square() {
        let json = serde_json::to_string(&ImageResolution::Res1024x1024).unwrap();
        assert_eq!(json, "\"1024x1024\"");
    }

    #[test]
    fn image_resolution_serializes_tall() {
        let json = serde_json::to_string(&ImageResolution::Res1024x1792).unwrap();
        assert_eq!(json, "\"1024x1792\"");
    }

    #[test]
    fn image_resolution_serializes_wide() {
        let json = serde_json::to_string(&ImageResolution::Res1792x1024).unwrap();
        assert_eq!(json, "\"1792x1024\"");
    }

    #[test]
    fn image_response_format_default_is_url() {
        let fmt = ImageResponseFormat::default();
        assert_eq!(fmt, ImageResponseFormat::Url);
    }

    #[test]
    fn image_response_format_url_serializes() {
        let json = serde_json::to_string(&ImageResponseFormat::Url).unwrap();
        assert_eq!(json, "\"url\"");
    }

    #[test]
    fn image_response_format_b64_json_serializes() {
        let json = serde_json::to_string(&ImageResponseFormat::B64Json).unwrap();
        assert_eq!(json, "\"b64_json\"");
    }

    #[test]
    fn image_response_deserializes_with_url() {
        let json = r#"{
            "created": 1700000000,
            "data": [
                {
                    "url": "https://example.com/image.png",
                    "revised_prompt": "A refined description"
                }
            ]
        }"#;
        let resp: ImageResponse = serde_json::from_str(json).unwrap();
        assert_eq!(resp.created, Some(1700000000));
        assert_eq!(resp.data.len(), 1);
        assert_eq!(
            resp.data[0].url.as_deref(),
            Some("https://example.com/image.png")
        );
        assert_eq!(
            resp.data[0].revised_prompt.as_deref(),
            Some("A refined description")
        );
        assert!(resp.data[0].b64_json.is_none());
    }

    #[test]
    fn image_response_deserializes_with_b64() {
        let json = r#"{
            "data": [
                {
                    "b64_json": "aGVsbG8="
                }
            ]
        }"#;
        let resp: ImageResponse = serde_json::from_str(json).unwrap();
        assert!(resp.created.is_none());
        assert_eq!(resp.data[0].b64_json.as_deref(), Some("aGVsbG8="));
    }

    #[test]
    fn image_response_tolerates_unknown_fields() {
        let json = r#"{
            "data": [{"url": "https://example.com/img.png", "unknown_field": 42}],
            "extra_top_level": true
        }"#;
        let resp: ImageResponse = serde_json::from_str(json).unwrap();
        assert_eq!(resp.data.len(), 1);
    }

    #[test]
    fn image_edit_request_serializes() {
        let req = ImageEditRequest {
            prompt: "Remove background".to_string(),
            model: "grok-2-image".to_string(),
            image: Some("https://example.com/photo.jpg".to_string()),
            images: None,
            mask: Some("https://example.com/mask.png".to_string()),
        };
        let json = serde_json::to_string(&req).unwrap();
        assert!(json.contains("\"prompt\":\"Remove background\""));
        assert!(json.contains("\"image\":\"https://example.com/photo.jpg\""));
        assert!(json.contains("\"mask\":\"https://example.com/mask.png\""));
        assert!(!json.contains("\"images\""));
    }
}
