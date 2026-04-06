//! Function calling types for the xAI Grok API.
//!
//! Provides `FunctionToolDefinition` for constructing properly-formatted tool
//! definitions, `ToolChoice` for controlling how the model selects tools, and
//! validation helpers for the 128-tool-per-request limit.

use serde::{Deserialize, Serialize};
use std::fmt;

use super::tool::{FunctionDefinition, ResponsesToolDefinition, ToolDefinition};

// ---------------------------------------------------------------------------
// Constants
// ---------------------------------------------------------------------------

/// Maximum number of tools allowed per request by the xAI API.
pub const MAX_TOOLS_PER_REQUEST: usize = 128;

// ---------------------------------------------------------------------------
// ToolError
// ---------------------------------------------------------------------------

/// Errors related to tool definition validation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ToolError {
    /// The number of tools exceeds the maximum allowed per request.
    TooManyTools {
        /// The number of tools that were provided.
        count: usize,
        /// The maximum number of tools allowed.
        max: usize,
    },
    /// A tool definition has an invalid or missing name.
    InvalidName {
        /// A description of what is wrong with the name.
        reason: String,
    },
}

impl fmt::Display for ToolError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ToolError::TooManyTools { count, max } => {
                write!(f, "too many tools: {count} provided, maximum is {max}")
            }
            ToolError::InvalidName { reason } => {
                write!(f, "invalid tool name: {reason}")
            }
        }
    }
}

impl std::error::Error for ToolError {}

// ---------------------------------------------------------------------------
// FunctionToolDefinition
// ---------------------------------------------------------------------------

/// A high-level wrapper for creating properly-formatted function tool definitions.
///
/// This type can produce wire-format definitions for both the Responses API
/// (flat structure) and the Chat Completions API (nested `function` object).
///
/// # Examples
///
/// ```
/// use grokrs_api::types::function_call::FunctionToolDefinition;
///
/// let tool = FunctionToolDefinition::new(
///     "get_weather",
///     "Get the current weather for a city",
///     serde_json::json!({
///         "type": "object",
///         "properties": {
///             "city": { "type": "string" }
///         },
///         "required": ["city"]
///     }),
/// ).unwrap();
///
/// // For the Responses API (flat format):
/// let responses_def = tool.to_responses_definition();
/// let json = serde_json::to_string(&responses_def).unwrap();
/// assert!(json.contains("\"name\":\"get_weather\""));
///
/// // For the Chat Completions API (nested format):
/// let chat_def = tool.to_chat_definition();
/// let json = serde_json::to_string(&chat_def).unwrap();
/// assert!(json.contains("\"function\""));
/// ```
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FunctionToolDefinition {
    /// The name of the function.
    name: String,
    /// A description of what the function does.
    description: String,
    /// The JSON Schema describing the function's parameters.
    parameters: serde_json::Value,
}

impl FunctionToolDefinition {
    /// Create a new function tool definition.
    ///
    /// # Arguments
    /// * `name` - The function name (must not be empty).
    /// * `description` - A human-readable description of what the function does.
    /// * `parameters` - A JSON Schema object describing the function's parameters.
    ///
    /// # Errors
    /// Returns `ToolError::InvalidName` if `name` is empty.
    pub fn new(
        name: impl Into<String>,
        description: impl Into<String>,
        parameters: serde_json::Value,
    ) -> Result<Self, ToolError> {
        let name = name.into();
        if name.is_empty() {
            return Err(ToolError::InvalidName {
                reason: "function name must not be empty".into(),
            });
        }
        Ok(Self {
            name,
            description: description.into(),
            parameters,
        })
    }

    /// Validate this tool definition, checking that the name is not empty.
    pub fn validate(&self) -> Result<(), ToolError> {
        if self.name.is_empty() {
            return Err(ToolError::InvalidName {
                reason: "function name must not be empty".into(),
            });
        }
        Ok(())
    }

    /// Return the function name.
    pub fn name(&self) -> &str {
        &self.name
    }

    /// Return the function description.
    pub fn description(&self) -> &str {
        &self.description
    }

    /// Return a reference to the parameters schema.
    pub fn parameters(&self) -> &serde_json::Value {
        &self.parameters
    }

    /// Convert to a `ResponsesToolDefinition` (flat wire format for the Responses API).
    ///
    /// Wire format: `{"type":"function","name":"...","description":"...","parameters":{...}}`
    pub fn to_responses_definition(&self) -> ResponsesToolDefinition {
        ResponsesToolDefinition {
            r#type: "function".into(),
            name: self.name.clone(),
            description: Some(self.description.clone()),
            parameters: Some(self.parameters.clone()),
        }
    }

    /// Convert to a `ToolDefinition` (nested wire format for the Chat Completions API).
    ///
    /// Wire format: `{"type":"function","function":{"name":"...","description":"...","parameters":{...}}}`
    pub fn to_chat_definition(&self) -> ToolDefinition {
        ToolDefinition {
            r#type: "function".into(),
            function: FunctionDefinition {
                name: self.name.clone(),
                description: Some(self.description.clone()),
                parameters: Some(self.parameters.clone()),
            },
        }
    }

    /// Serialize this definition to a `serde_json::Value` in the Responses API
    /// flat format.
    ///
    /// This is a convenience for building `CreateResponseRequest::tools` which
    /// accepts `Vec<serde_json::Value>`.
    pub fn to_responses_value(&self) -> serde_json::Value {
        serde_json::to_value(self.to_responses_definition())
            .expect("ResponsesToolDefinition serialization is infallible")
    }

    /// Serialize this definition to a `serde_json::Value` in the Chat
    /// Completions API nested format.
    pub fn to_chat_value(&self) -> serde_json::Value {
        serde_json::to_value(self.to_chat_definition())
            .expect("ToolDefinition serialization is infallible")
    }
}

// ---------------------------------------------------------------------------
// ToolChoice
// ---------------------------------------------------------------------------

/// Controls how the model selects which tool (if any) to call.
///
/// # Serialization
///
/// - `Auto` serializes as `"auto"`
/// - `Required` serializes as `"required"`
/// - `None` serializes as `"none"`
/// - `Function { name }` serializes as `{"type":"function","function":{"name":"..."}}`
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ToolChoice {
    /// The model may choose to call a tool or generate text.
    Auto,
    /// The model must call at least one tool.
    Required,
    /// The model must not call any tools.
    None,
    /// The model must call the specific named function.
    Function {
        /// The name of the function to call.
        name: String,
    },
}

impl ToolChoice {
    /// Convert this `ToolChoice` to a `serde_json::Value` suitable for use
    /// in `CreateResponseRequest::tool_choice` or `ChatCompletionRequest::tool_choice`.
    pub fn to_value(&self) -> serde_json::Value {
        match self {
            ToolChoice::Auto => serde_json::Value::String("auto".into()),
            ToolChoice::Required => serde_json::Value::String("required".into()),
            ToolChoice::None => serde_json::Value::String("none".into()),
            ToolChoice::Function { name } => {
                serde_json::json!({
                    "type": "function",
                    "function": {
                        "name": name
                    }
                })
            }
        }
    }
}

impl Serialize for ToolChoice {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        self.to_value().serialize(serializer)
    }
}

impl<'de> Deserialize<'de> for ToolChoice {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let value = serde_json::Value::deserialize(deserializer)?;
        match &value {
            serde_json::Value::String(s) => match s.as_str() {
                "auto" => Ok(ToolChoice::Auto),
                "required" => Ok(ToolChoice::Required),
                "none" => Ok(ToolChoice::None),
                other => Err(serde::de::Error::custom(format!(
                    "unknown tool_choice string: {other}"
                ))),
            },
            serde_json::Value::Object(obj) => {
                let tool_type = obj.get("type").and_then(|v| v.as_str()).unwrap_or("");
                if tool_type != "function" {
                    return Err(serde::de::Error::custom(format!(
                        "expected tool_choice type 'function', got '{tool_type}'"
                    )));
                }
                let name = obj
                    .get("function")
                    .and_then(|f| f.get("name"))
                    .and_then(|n| n.as_str())
                    .ok_or_else(|| {
                        serde::de::Error::custom("missing 'function.name' in tool_choice object")
                    })?;
                Ok(ToolChoice::Function {
                    name: name.to_string(),
                })
            }
            _ => Err(serde::de::Error::custom(
                "tool_choice must be a string or an object",
            )),
        }
    }
}

// ---------------------------------------------------------------------------
// Validation
// ---------------------------------------------------------------------------

/// Validate that the number of tools does not exceed the per-request limit.
///
/// The xAI API allows a maximum of 128 tools per request. This function
/// checks the count of any slice and returns an error if it exceeds the limit.
///
/// # Examples
///
/// ```
/// use grokrs_api::types::function_call::{validate_tool_count, FunctionToolDefinition};
///
/// let tools: Vec<FunctionToolDefinition> = Vec::new();
/// assert!(validate_tool_count(&tools).is_ok());
/// ```
pub fn validate_tool_count<T>(tools: &[T]) -> Result<(), ToolError> {
    if tools.len() > MAX_TOOLS_PER_REQUEST {
        return Err(ToolError::TooManyTools {
            count: tools.len(),
            max: MAX_TOOLS_PER_REQUEST,
        });
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn function_tool_definition_to_responses_format() {
        let tool = FunctionToolDefinition::new(
            "get_weather",
            "Get the current weather",
            serde_json::json!({
                "type": "object",
                "properties": {
                    "city": { "type": "string" }
                },
                "required": ["city"]
            }),
        )
        .unwrap();

        let def = tool.to_responses_definition();
        assert_eq!(def.r#type, "function");
        assert_eq!(def.name, "get_weather");
        assert_eq!(def.description.as_deref(), Some("Get the current weather"));
        assert!(def.parameters.is_some());

        // Verify flat wire format
        let json = serde_json::to_string(&def).unwrap();
        assert!(json.contains("\"type\":\"function\""));
        assert!(json.contains("\"name\":\"get_weather\""));
        assert!(
            !json.contains("\"function\":{"),
            "should be flat, not nested"
        );
    }

    #[test]
    fn function_tool_definition_to_chat_format() {
        let tool = FunctionToolDefinition::new(
            "search",
            "Search the web",
            serde_json::json!({
                "type": "object",
                "properties": {
                    "query": { "type": "string" }
                }
            }),
        )
        .unwrap();

        let def = tool.to_chat_definition();
        assert_eq!(def.r#type, "function");
        assert_eq!(def.function.name, "search");
        assert_eq!(def.function.description.as_deref(), Some("Search the web"));

        // Verify nested wire format
        let json = serde_json::to_string(&def).unwrap();
        assert!(
            json.contains("\"function\":{"),
            "should be nested under 'function'"
        );
    }

    #[test]
    fn function_tool_definition_to_responses_value() {
        let tool =
            FunctionToolDefinition::new("calc", "Calculate", serde_json::json!({"type": "object"}))
                .unwrap();
        let val = tool.to_responses_value();
        assert_eq!(val["type"], "function");
        assert_eq!(val["name"], "calc");
        assert_eq!(val["description"], "Calculate");
    }

    #[test]
    fn function_tool_definition_to_chat_value() {
        let tool =
            FunctionToolDefinition::new("calc", "Calculate", serde_json::json!({"type": "object"}))
                .unwrap();
        let val = tool.to_chat_value();
        assert_eq!(val["type"], "function");
        assert_eq!(val["function"]["name"], "calc");
    }

    #[test]
    fn function_tool_definition_accessors() {
        let tool =
            FunctionToolDefinition::new("my_func", "Does stuff", serde_json::json!({})).unwrap();
        assert_eq!(tool.name(), "my_func");
        assert_eq!(tool.description(), "Does stuff");
        assert_eq!(tool.parameters(), &serde_json::json!({}));
    }

    #[test]
    fn function_tool_definition_validate_valid() {
        let tool = FunctionToolDefinition::new("f", "desc", serde_json::json!({})).unwrap();
        assert!(tool.validate().is_ok());
    }

    #[test]
    fn function_tool_definition_new_rejects_empty_name() {
        let result = FunctionToolDefinition::new("", "desc", serde_json::json!({}));
        let err = result.unwrap_err();
        match err {
            ToolError::InvalidName { reason } => {
                assert!(reason.contains("empty"));
            }
            other => panic!("expected InvalidName, got: {other}"),
        }
    }

    // -- ToolChoice serialization --

    #[test]
    fn tool_choice_auto_serializes() {
        let json = serde_json::to_string(&ToolChoice::Auto).unwrap();
        assert_eq!(json, "\"auto\"");
    }

    #[test]
    fn tool_choice_required_serializes() {
        let json = serde_json::to_string(&ToolChoice::Required).unwrap();
        assert_eq!(json, "\"required\"");
    }

    #[test]
    fn tool_choice_none_serializes() {
        let json = serde_json::to_string(&ToolChoice::None).unwrap();
        assert_eq!(json, "\"none\"");
    }

    #[test]
    fn tool_choice_function_serializes() {
        let choice = ToolChoice::Function {
            name: "get_weather".into(),
        };
        let json = serde_json::to_string(&choice).unwrap();
        let parsed: serde_json::Value = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed["type"], "function");
        assert_eq!(parsed["function"]["name"], "get_weather");
    }

    #[test]
    fn tool_choice_auto_round_trips() {
        let choice = ToolChoice::Auto;
        let json = serde_json::to_string(&choice).unwrap();
        let back: ToolChoice = serde_json::from_str(&json).unwrap();
        assert_eq!(choice, back);
    }

    #[test]
    fn tool_choice_required_round_trips() {
        let choice = ToolChoice::Required;
        let json = serde_json::to_string(&choice).unwrap();
        let back: ToolChoice = serde_json::from_str(&json).unwrap();
        assert_eq!(choice, back);
    }

    #[test]
    fn tool_choice_none_round_trips() {
        let choice = ToolChoice::None;
        let json = serde_json::to_string(&choice).unwrap();
        let back: ToolChoice = serde_json::from_str(&json).unwrap();
        assert_eq!(choice, back);
    }

    #[test]
    fn tool_choice_function_round_trips() {
        let choice = ToolChoice::Function {
            name: "do_thing".into(),
        };
        let json = serde_json::to_string(&choice).unwrap();
        let back: ToolChoice = serde_json::from_str(&json).unwrap();
        assert_eq!(choice, back);
    }

    #[test]
    fn tool_choice_to_value_auto() {
        let val = ToolChoice::Auto.to_value();
        assert_eq!(val, serde_json::json!("auto"));
    }

    #[test]
    fn tool_choice_to_value_function() {
        let val = ToolChoice::Function { name: "f".into() }.to_value();
        assert_eq!(val["type"], "function");
        assert_eq!(val["function"]["name"], "f");
    }

    #[test]
    fn tool_choice_deserialize_unknown_string_fails() {
        let result: Result<ToolChoice, _> = serde_json::from_str("\"invalid\"");
        assert!(result.is_err());
    }

    #[test]
    fn tool_choice_deserialize_bad_object_type_fails() {
        let json = r#"{"type":"unknown","function":{"name":"f"}}"#;
        let result: Result<ToolChoice, _> = serde_json::from_str(json);
        assert!(result.is_err());
    }

    // -- validate_tool_count --

    #[test]
    fn validate_tool_count_zero_ok() {
        let tools: Vec<u8> = vec![];
        assert!(validate_tool_count(&tools).is_ok());
    }

    #[test]
    fn validate_tool_count_at_limit_ok() {
        let tools = vec![0u8; MAX_TOOLS_PER_REQUEST];
        assert!(validate_tool_count(&tools).is_ok());
    }

    #[test]
    fn validate_tool_count_over_limit_fails() {
        let tools = vec![0u8; MAX_TOOLS_PER_REQUEST + 1];
        let err = validate_tool_count(&tools).unwrap_err();
        match err {
            ToolError::TooManyTools { count, max } => {
                assert_eq!(count, MAX_TOOLS_PER_REQUEST + 1);
                assert_eq!(max, MAX_TOOLS_PER_REQUEST);
            }
            other => panic!("expected TooManyTools, got: {other}"),
        }
    }

    #[test]
    fn validate_tool_count_with_function_tool_definitions() {
        let tools: Vec<FunctionToolDefinition> = (0..128)
            .map(|i| {
                FunctionToolDefinition::new(
                    format!("tool_{i}"),
                    format!("Tool {i}"),
                    serde_json::json!({}),
                )
                .unwrap()
            })
            .collect();
        assert!(validate_tool_count(&tools).is_ok());
    }

    // -- ToolError --

    #[test]
    fn tool_error_display_too_many() {
        let err = ToolError::TooManyTools {
            count: 200,
            max: 128,
        };
        let display = format!("{err}");
        assert!(display.contains("200"));
        assert!(display.contains("128"));
    }

    #[test]
    fn tool_error_display_invalid_name() {
        let err = ToolError::InvalidName {
            reason: "empty".into(),
        };
        let display = format!("{err}");
        assert!(display.contains("invalid tool name"));
        assert!(display.contains("empty"));
    }

    #[test]
    fn tool_error_is_std_error() {
        let err = ToolError::TooManyTools {
            count: 200,
            max: 128,
        };
        let _: &dyn std::error::Error = &err;
    }
}
