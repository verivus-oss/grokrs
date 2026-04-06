use serde::{Deserialize, Serialize};

/// Roles that a message participant can assume in the xAI API.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Role {
    System,
    Developer,
    User,
    Assistant,
    Tool,
}

/// A single content block within a message.
///
/// The xAI API uses `"type"` as the discriminator tag. Each variant maps to
/// the corresponding wire-format object. Both Chat Completions and Responses
/// API content block formats are supported.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum ContentBlock {
    // -- Chat Completions style --
    /// Plain text content (Chat Completions API).
    #[serde(rename = "text")]
    Text {
        /// The text content.
        text: String,
    },

    /// An image referenced by URL (Chat Completions API).
    #[serde(rename = "image_url")]
    ImageUrl {
        /// The image URL and optional detail level.
        image_url: ImageUrlDetail,
    },

    // -- Responses API style --
    /// Plain text content (Responses API).
    ///
    /// Wire format: `{"type":"input_text","text":"..."}`.
    #[serde(rename = "input_text")]
    InputText {
        /// The text content.
        text: String,
    },

    /// An image referenced by URL or inline data URI (Responses API).
    ///
    /// Wire format: `{"type":"input_image","image_url":"...","detail":"high"}`.
    /// For inline base64 images, encode as a data URI in `image_url`:
    /// `"data:<media_type>;base64,<data>"`.
    #[serde(rename = "input_image")]
    InputImage {
        /// The image URL or data URI (`data:<media_type>;base64,<data>`).
        image_url: String,
        /// Optional detail level for processing (e.g., "auto", "low", "high").
        #[serde(skip_serializing_if = "Option::is_none")]
        detail: Option<String>,
    },
}

impl ContentBlock {
    /// Construct an `InputImage` from inline base64-encoded data.
    ///
    /// Encodes the data as a data URI: `data:<media_type>;base64,<data>`.
    ///
    /// # Arguments
    /// * `data` - Base64-encoded image data.
    /// * `media_type` - MIME type, e.g. `"image/png"`.
    /// * `detail` - Optional detail level (e.g. `"auto"`, `"low"`, `"high"`).
    #[must_use]
    pub fn input_image_base64(data: &str, media_type: &str, detail: Option<String>) -> Self {
        ContentBlock::InputImage {
            image_url: format!("data:{media_type};base64,{data}"),
            detail,
        }
    }
}

/// Detail information for an image URL content block.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ImageUrlDetail {
    /// The URL of the image.
    pub url: String,
    /// Optional detail level (e.g., "auto", "low", "high").
    #[serde(skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
}

/// Message content that can be either a plain string or an array of content blocks.
///
/// The xAI API accepts both forms. This enum deserializes from either
/// representation using serde's untagged enum support.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum MessageContent {
    /// A plain text string.
    Text(String),
    /// An array of structured content blocks.
    Blocks(Vec<ContentBlock>),
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn role_serializes_lowercase() {
        assert_eq!(serde_json::to_string(&Role::System).unwrap(), "\"system\"");
        assert_eq!(serde_json::to_string(&Role::User).unwrap(), "\"user\"");
        assert_eq!(
            serde_json::to_string(&Role::Assistant).unwrap(),
            "\"assistant\""
        );
        assert_eq!(serde_json::to_string(&Role::Tool).unwrap(), "\"tool\"");
        assert_eq!(
            serde_json::to_string(&Role::Developer).unwrap(),
            "\"developer\""
        );
    }

    #[test]
    fn role_round_trips() {
        for role in [
            Role::System,
            Role::Developer,
            Role::User,
            Role::Assistant,
            Role::Tool,
        ] {
            let json = serde_json::to_string(&role).unwrap();
            let back: Role = serde_json::from_str(&json).unwrap();
            assert_eq!(role, back);
        }
    }

    #[test]
    fn content_block_text_round_trips() {
        let block = ContentBlock::Text {
            text: "hello".into(),
        };
        let json = serde_json::to_string(&block).unwrap();
        assert!(json.contains("\"type\":\"text\""));
        let back: ContentBlock = serde_json::from_str(&json).unwrap();
        assert_eq!(block, back);
    }

    #[test]
    fn content_block_image_url_round_trips() {
        let block = ContentBlock::ImageUrl {
            image_url: ImageUrlDetail {
                url: "https://example.com/img.png".into(),
                detail: Some("high".into()),
            },
        };
        let json = serde_json::to_string(&block).unwrap();
        assert!(json.contains("\"type\":\"image_url\""));
        let back: ContentBlock = serde_json::from_str(&json).unwrap();
        assert_eq!(block, back);
    }

    #[test]
    fn content_block_input_image_url_round_trips() {
        let block = ContentBlock::InputImage {
            image_url: "https://example.com/img.png".into(),
            detail: Some("high".into()),
        };
        let json = serde_json::to_string(&block).unwrap();
        assert!(json.contains("\"type\":\"input_image\""));
        assert!(json.contains("\"image_url\":\"https://example.com/img.png\""));
        assert!(json.contains("\"detail\":\"high\""));
        let back: ContentBlock = serde_json::from_str(&json).unwrap();
        assert_eq!(block, back);
    }

    #[test]
    fn content_block_input_image_data_uri_round_trips() {
        let block =
            ContentBlock::input_image_base64("base64data", "image/png", Some("high".into()));
        let json = serde_json::to_string(&block).unwrap();
        assert!(json.contains("\"type\":\"input_image\""));
        assert!(json.contains("\"image_url\":\"data:image/png;base64,base64data\""));
        assert!(json.contains("\"detail\":\"high\""));
        let back: ContentBlock = serde_json::from_str(&json).unwrap();
        assert_eq!(block, back);
    }

    #[test]
    fn content_block_input_image_base64_no_detail() {
        let block = ContentBlock::input_image_base64("abc123", "image/jpeg", None);
        let json = serde_json::to_string(&block).unwrap();
        assert!(json.contains("\"image_url\":\"data:image/jpeg;base64,abc123\""));
        assert!(!json.contains("\"detail\""));
        let back: ContentBlock = serde_json::from_str(&json).unwrap();
        assert_eq!(block, back);
    }

    #[test]
    fn content_block_input_image_url_no_detail_round_trips() {
        let block = ContentBlock::InputImage {
            image_url: "https://example.com/photo.jpg".into(),
            detail: None,
        };
        let json = serde_json::to_string(&block).unwrap();
        assert!(!json.contains("\"detail\""));
        let back: ContentBlock = serde_json::from_str(&json).unwrap();
        assert_eq!(block, back);
    }

    #[test]
    fn content_block_input_text_round_trips() {
        let block = ContentBlock::InputText {
            text: "hello responses".into(),
        };
        let json = serde_json::to_string(&block).unwrap();
        assert!(json.contains("\"type\":\"input_text\""));
        assert!(json.contains("\"text\":\"hello responses\""));
        let back: ContentBlock = serde_json::from_str(&json).unwrap();
        assert_eq!(block, back);
    }

    #[test]
    fn content_block_input_text_deserializes_from_wire() {
        let json = r#"{"type":"input_text","text":"wire format"}"#;
        let block: ContentBlock = serde_json::from_str(json).unwrap();
        assert_eq!(
            block,
            ContentBlock::InputText {
                text: "wire format".into()
            }
        );
    }

    #[test]
    fn content_block_input_image_deserializes_responses_wire() {
        let json =
            r#"{"type":"input_image","image_url":"https://example.com/img.png","detail":"high"}"#;
        let block: ContentBlock = serde_json::from_str(json).unwrap();
        assert_eq!(
            block,
            ContentBlock::InputImage {
                image_url: "https://example.com/img.png".into(),
                detail: Some("high".into()),
            }
        );
    }

    #[test]
    fn content_block_input_image_deserializes_data_uri_wire() {
        let json =
            r#"{"type":"input_image","image_url":"data:image/png;base64,abc123","detail":"low"}"#;
        let block: ContentBlock = serde_json::from_str(json).unwrap();
        assert_eq!(
            block,
            ContentBlock::InputImage {
                image_url: "data:image/png;base64,abc123".into(),
                detail: Some("low".into()),
            }
        );
    }

    #[test]
    fn image_url_detail_skips_none_detail() {
        let detail = ImageUrlDetail {
            url: "https://example.com/img.png".into(),
            detail: None,
        };
        let json = serde_json::to_string(&detail).unwrap();
        assert!(!json.contains("\"detail\""));
    }

    #[test]
    fn message_content_text_form() {
        let content = MessageContent::Text("hello world".into());
        let json = serde_json::to_string(&content).unwrap();
        assert_eq!(json, "\"hello world\"");
        let back: MessageContent = serde_json::from_str(&json).unwrap();
        assert_eq!(content, back);
    }

    #[test]
    fn message_content_blocks_form() {
        let content = MessageContent::Blocks(vec![ContentBlock::Text {
            text: "hello".into(),
        }]);
        let json = serde_json::to_string(&content).unwrap();
        assert!(json.starts_with('['));
        let back: MessageContent = serde_json::from_str(&json).unwrap();
        assert_eq!(content, back);
    }

    #[test]
    fn message_content_deserializes_plain_string() {
        let json = "\"just a string\"";
        let content: MessageContent = serde_json::from_str(json).unwrap();
        assert_eq!(content, MessageContent::Text("just a string".into()));
    }

    #[test]
    fn message_content_deserializes_array() {
        let json = r#"[{"type":"text","text":"hello"}]"#;
        let content: MessageContent = serde_json::from_str(json).unwrap();
        match content {
            MessageContent::Blocks(blocks) => {
                assert_eq!(blocks.len(), 1);
                assert_eq!(
                    blocks[0],
                    ContentBlock::Text {
                        text: "hello".into()
                    }
                );
            }
            _ => panic!("expected Blocks variant"),
        }
    }
}
