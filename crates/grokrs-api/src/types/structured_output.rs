//! Structured output format helpers for both the Responses and Chat Completions APIs.
//!
//! The xAI API supports JSON Schema-constrained outputs, but the wire format
//! differs between the two APIs:
//!
//! - **Responses API**: `text.format` uses `{"type":"json_schema","name":"...","strict":true,"schema":{...}}`
//! - **Chat Completions API**: `response_format` uses `{"type":"json_schema","json_schema":{"name":"...","strict":true,"schema":{...}}}`
//!
//! `StructuredOutputBuilder` constructs the correct format for each API.

use serde_json::Value;

use super::responses::{TextConfig, TextFormat};

// ---------------------------------------------------------------------------
// StructuredOutputBuilder
// ---------------------------------------------------------------------------

/// Builder for creating structured output configurations in the correct wire
/// format for each xAI API.
///
/// # Examples
///
/// ```
/// use grokrs_api::types::structured_output::StructuredOutputBuilder;
///
/// let schema = serde_json::json!({
///     "type": "object",
///     "properties": {
///         "temperature": { "type": "number" },
///         "unit": { "type": "string" }
///     },
///     "required": ["temperature", "unit"],
///     "additionalProperties": false
/// });
///
/// // Responses API format
/// let text_config = StructuredOutputBuilder::for_responses("weather", schema.clone());
/// let json = serde_json::to_string(&text_config).unwrap();
/// assert!(json.contains("\"json_schema\""));
///
/// // Chat Completions API format
/// let response_format = StructuredOutputBuilder::for_chat("weather", &schema);
/// assert_eq!(response_format["type"], "json_schema");
/// assert_eq!(response_format["json_schema"]["name"], "weather");
/// ```
pub struct StructuredOutputBuilder;

impl StructuredOutputBuilder {
    /// Create a `TextConfig` for the Responses API with JSON Schema output format.
    ///
    /// The resulting configuration sets `text.format` to a `json_schema` variant
    /// with `strict: true` by default.
    ///
    /// Wire format in the request body:
    /// ```json
    /// {
    ///   "text": {
    ///     "format": {
    ///       "type": "json_schema",
    ///       "name": "...",
    ///       "strict": true,
    ///       "schema": { ... }
    ///     }
    ///   }
    /// }
    /// ```
    pub fn for_responses(name: impl Into<String>, schema: Value) -> TextConfig {
        TextConfig {
            format: Some(TextFormat::JsonSchema {
                name: name.into(),
                strict: Some(true),
                schema,
            }),
        }
    }

    /// Create a `TextConfig` for the Responses API with a custom `strict` setting.
    pub fn for_responses_with_strict(
        name: impl Into<String>,
        schema: Value,
        strict: bool,
    ) -> TextConfig {
        TextConfig {
            format: Some(TextFormat::JsonSchema {
                name: name.into(),
                strict: Some(strict),
                schema,
            }),
        }
    }

    /// Create a `serde_json::Value` for the Chat Completions API `response_format` field.
    ///
    /// Wire format:
    /// ```json
    /// {
    ///   "type": "json_schema",
    ///   "json_schema": {
    ///     "name": "...",
    ///     "strict": true,
    ///     "schema": { ... }
    ///   }
    /// }
    /// ```
    pub fn for_chat(name: impl Into<String>, schema: &Value) -> Value {
        serde_json::json!({
            "type": "json_schema",
            "json_schema": {
                "name": name.into(),
                "strict": true,
                "schema": schema,
            }
        })
    }

    /// Create a `serde_json::Value` for the Chat Completions API with a custom
    /// `strict` setting.
    pub fn for_chat_with_strict(name: impl Into<String>, schema: &Value, strict: bool) -> Value {
        serde_json::json!({
            "type": "json_schema",
            "json_schema": {
                "name": name.into(),
                "strict": strict,
                "schema": schema,
            }
        })
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_schema() -> Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "temperature": { "type": "number" },
                "unit": { "type": "string", "enum": ["celsius", "fahrenheit"] }
            },
            "required": ["temperature", "unit"],
            "additionalProperties": false
        })
    }

    // -- Responses API format --

    #[test]
    fn for_responses_produces_correct_format() {
        let config = StructuredOutputBuilder::for_responses("weather_data", sample_schema());
        let json = serde_json::to_string(&config).unwrap();

        // Verify it has the flat format: {"format":{"type":"json_schema","name":"...","strict":true,"schema":{...}}}
        assert!(json.contains("\"type\":\"json_schema\""));
        assert!(json.contains("\"name\":\"weather_data\""));
        assert!(json.contains("\"strict\":true"));
        assert!(json.contains("\"schema\""));

        // Verify it does NOT have the nested json_schema wrapper
        assert!(
            !json.contains("\"json_schema\":{\"name\""),
            "should NOT use nested json_schema object"
        );
    }

    #[test]
    fn for_responses_round_trips() {
        let config = StructuredOutputBuilder::for_responses("test", sample_schema());
        let json = serde_json::to_string(&config).unwrap();
        let back: TextConfig = serde_json::from_str(&json).unwrap();

        match back.format.unwrap() {
            TextFormat::JsonSchema {
                name,
                strict,
                schema,
            } => {
                assert_eq!(name, "test");
                assert_eq!(strict, Some(true));
                assert_eq!(schema["type"], "object");
            }
            other => panic!("expected JsonSchema, got: {other:?}"),
        }
    }

    #[test]
    fn for_responses_with_strict_false() {
        let config =
            StructuredOutputBuilder::for_responses_with_strict("loose", sample_schema(), false);
        match config.format.unwrap() {
            TextFormat::JsonSchema { strict, .. } => {
                assert_eq!(strict, Some(false));
            }
            other => panic!("expected JsonSchema, got: {other:?}"),
        }
    }

    // -- Chat Completions API format --

    #[test]
    fn for_chat_produces_correct_format() {
        let val = StructuredOutputBuilder::for_chat("weather_data", &sample_schema());

        // Verify the nested format
        assert_eq!(val["type"], "json_schema");
        assert_eq!(val["json_schema"]["name"], "weather_data");
        assert_eq!(val["json_schema"]["strict"], true);
        assert!(val["json_schema"]["schema"].is_object());
    }

    #[test]
    fn for_chat_has_nested_json_schema() {
        let val =
            StructuredOutputBuilder::for_chat("output", &serde_json::json!({"type": "object"}));
        let json = serde_json::to_string(&val).unwrap();

        // Must have the nested structure
        assert!(json.contains("\"json_schema\":{"));
        assert!(json.contains("\"name\":\"output\""));
    }

    #[test]
    fn for_chat_with_strict_false() {
        let val = StructuredOutputBuilder::for_chat_with_strict("loose", &sample_schema(), false);
        assert_eq!(val["json_schema"]["strict"], false);
    }

    // -- Format difference between APIs --

    #[test]
    fn responses_and_chat_formats_differ() {
        let schema = sample_schema();
        let responses_config = StructuredOutputBuilder::for_responses("test", schema.clone());
        let chat_val = StructuredOutputBuilder::for_chat("test", &schema);

        let responses_json = serde_json::to_string(&responses_config).unwrap();
        let chat_json = serde_json::to_string(&chat_val).unwrap();

        // Responses format has "format" wrapping; Chat format has "json_schema" nesting
        assert_ne!(responses_json, chat_json);

        // Chat format has the nested json_schema key
        assert!(chat_json.contains("\"json_schema\":{"));

        // Responses format has the flat structure inside format
        assert!(responses_json.contains("\"format\":{\"type\":\"json_schema\""));
    }

    #[test]
    fn for_responses_schema_is_preserved() {
        let schema = serde_json::json!({
            "type": "array",
            "items": { "type": "string" },
            "minItems": 1
        });
        let config = StructuredOutputBuilder::for_responses("list", schema.clone());
        match config.format.unwrap() {
            TextFormat::JsonSchema { schema: s, .. } => {
                assert_eq!(s, schema);
            }
            other => panic!("expected JsonSchema, got: {other:?}"),
        }
    }

    #[test]
    fn for_chat_schema_is_preserved() {
        let schema = serde_json::json!({
            "type": "array",
            "items": { "type": "string" }
        });
        let val = StructuredOutputBuilder::for_chat("list", &schema);
        assert_eq!(val["json_schema"]["schema"], schema);
    }
}
