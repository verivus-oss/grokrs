use serde::{Deserialize, Serialize};

/// A minimal model object from the `/v1/models` endpoint.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Model {
    /// The model identifier (e.g., "grok-4").
    pub id: String,
    /// Unix timestamp of model creation.
    pub created: i64,
    /// The organization that owns the model.
    pub owned_by: String,
    /// The object type (e.g., "model"). Present in `/v1/models` responses.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub object: Option<String>,
}

/// Response from the `/v1/models` list endpoint.
///
/// The `/v1/models` endpoint returns `{"object":"list","data":[...]}`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ModelList {
    /// The object type (e.g., "list").
    #[serde(skip_serializing_if = "Option::is_none")]
    pub object: Option<String>,
    /// The list of models.
    pub data: Vec<Model>,
}

/// Extended language model information from the `/v1/language-models` endpoint.
///
/// Pricing fields are integer values (e.g., cents per 100M tokens). NEVER
/// use floating-point for pricing.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LanguageModel {
    /// The model identifier.
    pub id: String,
    /// Unix timestamp of model creation.
    pub created: i64,
    /// The organization that owns the model.
    pub owned_by: String,
    /// The object type (e.g., "model").
    #[serde(skip_serializing_if = "Option::is_none")]
    pub object: Option<String>,
    /// Alternative names for this model.
    #[serde(default)]
    pub aliases: Vec<String>,
    /// Supported input modalities (e.g., ["text", "image"]).
    #[serde(default)]
    pub input_modalities: Vec<String>,
    /// Supported output modalities (e.g., ["text"]).
    #[serde(default)]
    pub output_modalities: Vec<String>,
    /// Price per prompt text token in integer ticks (cents per 100M tokens).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub prompt_text_token_price: Option<i64>,
    /// Price per completion text token in integer ticks.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub completion_text_token_price: Option<i64>,
    /// Price per cached prompt text token in integer ticks.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cached_prompt_text_token_price: Option<i64>,
    /// Price per prompt image token in integer ticks.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub prompt_image_token_price: Option<i64>,
    /// Price for search functionality in integer ticks.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub search_price: Option<i64>,
    /// Price for image generation in integer ticks.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub image_price: Option<i64>,
    /// Maximum prompt length in tokens.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_prompt_length: Option<u64>,
    /// A fingerprint for this model version.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub fingerprint: Option<String>,
    /// The version string for this model.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub version: Option<String>,
}

/// Response from the `/v1/language-models` list endpoint.
///
/// The `/v1/language-models` endpoint returns `{"models":[...]}`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LanguageModelList {
    /// The list of language models.
    pub models: Vec<LanguageModel>,
}

/// Image model metadata from the `/v1/image-models` endpoint.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ImageModel {
    /// The model identifier.
    pub id: String,
    /// Unix timestamp of model creation.
    pub created: i64,
    /// The organization that owns the model.
    pub owned_by: String,
    /// Supported input modalities.
    #[serde(default)]
    pub input_modalities: Vec<String>,
    /// Supported output modalities.
    #[serde(default)]
    pub output_modalities: Vec<String>,
    /// Price per image generation in integer ticks.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub per_image_price: Option<i64>,
    /// A fingerprint for this model version.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub fingerprint: Option<String>,
    /// The version string for this model.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub version: Option<String>,
}

/// Response from the `/v1/image-models` list endpoint.
///
/// The `/v1/image-models` endpoint returns `{"models":[...]}`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ImageModelList {
    /// The list of image models.
    pub models: Vec<ImageModel>,
}

/// Video model metadata from the `/v1/video-models` endpoint.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct VideoModel {
    /// The model identifier.
    pub id: String,
    /// Unix timestamp of model creation.
    pub created: i64,
    /// The organization that owns the model.
    pub owned_by: String,
    /// Supported input modalities.
    #[serde(default)]
    pub input_modalities: Vec<String>,
    /// Supported output modalities.
    #[serde(default)]
    pub output_modalities: Vec<String>,
    /// Price per second of video in integer ticks.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub per_second_price: Option<i64>,
    /// A fingerprint for this model version.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub fingerprint: Option<String>,
    /// The version string for this model.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub version: Option<String>,
}

/// Response from the `/v1/video-models` list endpoint.
///
/// The `/v1/video-models` endpoint returns `{"models":[...]}`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct VideoModelList {
    /// The list of video models.
    pub models: Vec<VideoModel>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn model_round_trips() {
        let model = Model {
            id: "grok-4".into(),
            created: 1_700_000_000,
            owned_by: "xai".into(),
            object: Some("model".into()),
        };
        let json = serde_json::to_string(&model).unwrap();
        let back: Model = serde_json::from_str(&json).unwrap();
        assert_eq!(model, back);
    }

    #[test]
    fn model_without_object_field() {
        let json = r#"{"id":"grok-4","created":1700000000,"owned_by":"xai"}"#;
        let model: Model = serde_json::from_str(json).unwrap();
        assert_eq!(model.id, "grok-4");
        assert!(model.object.is_none());
    }

    #[test]
    fn model_list_round_trips() {
        let list = ModelList {
            object: Some("list".into()),
            data: vec![
                Model {
                    id: "grok-4".into(),
                    created: 1_700_000_000,
                    owned_by: "xai".into(),
                    object: Some("model".into()),
                },
                Model {
                    id: "grok-4-mini".into(),
                    created: 1_700_000_001,
                    owned_by: "xai".into(),
                    object: None,
                },
            ],
        };
        let json = serde_json::to_string(&list).unwrap();
        let back: ModelList = serde_json::from_str(&json).unwrap();
        assert_eq!(list, back);
    }

    #[test]
    fn model_list_deserializes_v1_models_wire() {
        let json = r#"{"object":"list","data":[{"id":"grok-4","created":1700000000,"owned_by":"xai","object":"model"}]}"#;
        let list: ModelList = serde_json::from_str(json).unwrap();
        assert_eq!(list.object.as_deref(), Some("list"));
        assert_eq!(list.data.len(), 1);
        assert_eq!(list.data[0].object.as_deref(), Some("model"));
    }

    #[test]
    fn language_model_round_trips() {
        let lm = LanguageModel {
            id: "grok-4".into(),
            created: 1_700_000_000,
            owned_by: "xai".into(),
            object: Some("model".into()),
            aliases: vec!["grok-latest".into()],
            input_modalities: vec!["text".into(), "image".into()],
            output_modalities: vec!["text".into()],
            prompt_text_token_price: Some(500),
            completion_text_token_price: Some(1500),
            cached_prompt_text_token_price: Some(250),
            prompt_image_token_price: Some(800),
            search_price: Some(5000),
            image_price: Some(10_000),
            max_prompt_length: Some(131_072),
            fingerprint: Some("fp_abc123".into()),
            version: Some("2025-01-01".into()),
        };
        let json = serde_json::to_string(&lm).unwrap();
        let back: LanguageModel = serde_json::from_str(&json).unwrap();
        assert_eq!(lm, back);
    }

    #[test]
    fn language_model_pricing_is_integer() {
        let json = r#"{
            "id": "grok-4",
            "created": 1700000000,
            "owned_by": "xai",
            "prompt_text_token_price": 500,
            "completion_text_token_price": 1500,
            "cached_prompt_text_token_price": 250,
            "prompt_image_token_price": 800,
            "search_price": 5000,
            "image_price": 10000
        }"#;
        let lm: LanguageModel = serde_json::from_str(json).unwrap();
        assert_eq!(lm.prompt_text_token_price, Some(500i64));
        assert_eq!(lm.completion_text_token_price, Some(1500i64));
        assert_eq!(lm.cached_prompt_text_token_price, Some(250i64));
        assert_eq!(lm.prompt_image_token_price, Some(800i64));
        assert_eq!(lm.search_price, Some(5000i64));
        assert_eq!(lm.image_price, Some(10_000i64));
    }

    #[test]
    fn language_model_defaults_empty_vecs() {
        let json = r#"{"id":"m","created":0,"owned_by":"x"}"#;
        let lm: LanguageModel = serde_json::from_str(json).unwrap();
        assert!(lm.aliases.is_empty());
        assert!(lm.input_modalities.is_empty());
        assert!(lm.output_modalities.is_empty());
    }

    #[test]
    fn language_model_deserializes_with_unknown_fields() {
        let json = r#"{
            "id": "grok-4",
            "created": 1700000000,
            "owned_by": "xai",
            "some_new_pricing_field": 999
        }"#;
        let lm: LanguageModel = serde_json::from_str(json).unwrap();
        assert_eq!(lm.id, "grok-4");
    }

    #[test]
    fn language_model_list_uses_models_key() {
        let json = r#"{"models":[{"id":"grok-4","created":1700000000,"owned_by":"xai"}]}"#;
        let list: LanguageModelList = serde_json::from_str(json).unwrap();
        assert_eq!(list.models.len(), 1);
        assert_eq!(list.models[0].id, "grok-4");
    }

    #[test]
    fn language_model_list_round_trips() {
        let list = LanguageModelList {
            models: vec![LanguageModel {
                id: "grok-4".into(),
                created: 1_700_000_000,
                owned_by: "xai".into(),
                object: None,
                aliases: vec![],
                input_modalities: vec![],
                output_modalities: vec![],
                prompt_text_token_price: None,
                completion_text_token_price: None,
                cached_prompt_text_token_price: None,
                prompt_image_token_price: None,
                search_price: None,
                image_price: None,
                max_prompt_length: None,
                fingerprint: None,
                version: None,
            }],
        };
        let json = serde_json::to_string(&list).unwrap();
        assert!(json.contains("\"models\":["));
        let back: LanguageModelList = serde_json::from_str(&json).unwrap();
        assert_eq!(list, back);
    }

    #[test]
    fn language_model_max_prompt_length() {
        let json = r#"{"id":"grok-4","created":0,"owned_by":"xai","max_prompt_length":131072}"#;
        let lm: LanguageModel = serde_json::from_str(json).unwrap();
        assert_eq!(lm.max_prompt_length, Some(131_072));
    }

    #[test]
    fn image_model_round_trips() {
        let im = ImageModel {
            id: "grok-2-image".into(),
            created: 1_700_000_000,
            owned_by: "xai".into(),
            input_modalities: vec!["text".into()],
            output_modalities: vec!["image".into()],
            per_image_price: Some(7000),
            fingerprint: None,
            version: None,
        };
        let json = serde_json::to_string(&im).unwrap();
        let back: ImageModel = serde_json::from_str(&json).unwrap();
        assert_eq!(im, back);
    }

    #[test]
    fn image_model_list_round_trips() {
        let list = ImageModelList {
            models: vec![ImageModel {
                id: "grok-2-image".into(),
                created: 1_700_000_000,
                owned_by: "xai".into(),
                input_modalities: vec!["text".into()],
                output_modalities: vec!["image".into()],
                per_image_price: Some(7000),
                fingerprint: None,
                version: None,
            }],
        };
        let json = serde_json::to_string(&list).unwrap();
        assert!(json.contains("\"models\":["));
        let back: ImageModelList = serde_json::from_str(&json).unwrap();
        assert_eq!(list, back);
    }

    #[test]
    fn video_model_round_trips() {
        let vm = VideoModel {
            id: "grok-video".into(),
            created: 1_700_000_000,
            owned_by: "xai".into(),
            input_modalities: vec!["text".into()],
            output_modalities: vec!["video".into()],
            per_second_price: Some(1000),
            fingerprint: None,
            version: None,
        };
        let json = serde_json::to_string(&vm).unwrap();
        let back: VideoModel = serde_json::from_str(&json).unwrap();
        assert_eq!(vm, back);
    }

    #[test]
    fn video_model_list_round_trips() {
        let list = VideoModelList {
            models: vec![VideoModel {
                id: "grok-video".into(),
                created: 1_700_000_000,
                owned_by: "xai".into(),
                input_modalities: vec!["text".into()],
                output_modalities: vec!["video".into()],
                per_second_price: Some(1000),
                fingerprint: None,
                version: None,
            }],
        };
        let json = serde_json::to_string(&list).unwrap();
        assert!(json.contains("\"models\":["));
        let back: VideoModelList = serde_json::from_str(&json).unwrap();
        assert_eq!(list, back);
    }

    #[test]
    fn model_names_are_not_hardcoded() {
        let json = r#"{"id":"totally-custom-model-v99","created":0,"owned_by":"custom-org"}"#;
        let model: Model = serde_json::from_str(json).unwrap();
        assert_eq!(model.id, "totally-custom-model-v99");
        assert_eq!(model.owned_by, "custom-org");
    }
}
