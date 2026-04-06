//! Wire types for the xAI Chat Completions API (legacy).
//!
//! These types map directly to the JSON request/response bodies of the
//! `/v1/chat/completions` endpoint. They are intentionally **deprecated** —
//! new integrations should use the Responses API types and endpoint instead.

// All types in this module are deprecated. Allow them to reference each other
// without triggering deprecation warnings within the module itself.
#![allow(deprecated)]

use serde::{Deserialize, Serialize};

use super::common::Role;
use super::message::Message;
use super::tool::{ToolCall, ToolDefinition};
use super::usage::ChatUsage;

// ---------------------------------------------------------------------------
// Request types
// ---------------------------------------------------------------------------

/// A request body for `POST /v1/chat/completions`.
///
/// # Deprecation
///
/// The Chat Completions API is a legacy interface. Prefer the Responses API
/// for all new integrations.
#[deprecated(note = "Use the Responses API instead")]
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ChatCompletionRequest {
    /// The model ID to use (e.g., `"grok-4"`, `"grok-4-mini"`).
    pub model: String,

    /// The conversation history as a sequence of messages.
    pub messages: Vec<Message>,

    /// Tool definitions available for the model to call.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tools: Option<Vec<ToolDefinition>>,

    /// Controls which tool the model should call. Pass `"auto"`, `"none"`,
    /// `"required"`, or an object `{"type":"function","function":{"name":"..."}}`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool_choice: Option<serde_json::Value>,

    /// Whether to stream the response as SSE events.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub stream: Option<bool>,

    /// Sampling temperature (0.0 to 2.0).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub temperature: Option<f64>,

    /// Nucleus sampling threshold (0.0 to 1.0).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub top_p: Option<f64>,

    /// Maximum number of tokens in the completion.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_completion_tokens: Option<u64>,

    /// Number of completions to generate.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub n: Option<u32>,

    /// Stop sequences that terminate generation.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub stop: Option<Vec<String>>,

    /// Deterministic sampling seed.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub seed: Option<i64>,

    /// Penalise tokens based on their frequency in the text so far (-2.0 to 2.0).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub frequency_penalty: Option<f64>,

    /// Penalise tokens based on whether they have appeared in the text so far (-2.0 to 2.0).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub presence_penalty: Option<f64>,

    /// Constrains the output format (plain text, JSON object, or JSON schema).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub response_format: Option<ResponseFormat>,

    /// Reasoning effort hint for reasoning-capable models (e.g., `"low"`, `"high"`).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reasoning_effort: Option<String>,

    /// Parameters for live-search augmented generation.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub search_parameters: Option<serde_json::Value>,

    /// When `true`, the server returns immediately with a `request_id` and
    /// processes the completion asynchronously. Poll via
    /// `GET /v1/chat/deferred-completion/{request_id}`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub deferred: Option<bool>,

    /// Options for streaming responses (e.g., whether to include usage).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub stream_options: Option<StreamOptions>,

    /// Whether the model should execute multiple tool calls in parallel.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub parallel_tool_calls: Option<bool>,
}

/// Output format constraint for the completion.
#[deprecated(note = "Use the Responses API instead")]
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ResponseFormat {
    /// Plain text output (the default).
    Text,
    /// Structured JSON output conforming to a provided schema.
    JsonSchema {
        /// The JSON schema specification.
        json_schema: JsonSchemaSpec,
    },
    /// Freeform JSON object output (no schema enforced).
    JsonObject,
}

/// A named JSON Schema specification for structured output.
#[deprecated(note = "Use the Responses API instead")]
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct JsonSchemaSpec {
    /// A human-readable name for this schema.
    pub name: String,

    /// Whether the model should strictly adhere to the schema.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub strict: Option<bool>,

    /// The JSON Schema definition.
    pub schema: serde_json::Value,
}

/// Options controlling streaming behaviour.
#[deprecated(note = "Use the Responses API instead")]
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StreamOptions {
    /// When `true`, the final SSE chunk includes a `usage` object.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub include_usage: Option<bool>,
}

// ---------------------------------------------------------------------------
// Response types
// ---------------------------------------------------------------------------

/// A completed chat response from `POST /v1/chat/completions`.
#[deprecated(note = "Use the Responses API instead")]
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ChatCompletion {
    /// Unique identifier for this completion.
    pub id: String,

    /// Object type (typically `"chat.completion"`).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub object: Option<String>,

    /// Unix timestamp (seconds since epoch) when the completion was created.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub created: Option<i64>,

    /// The model that generated the completion.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,

    /// The generated choices.
    pub choices: Vec<ChatChoice>,

    /// Token usage statistics.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub usage: Option<ChatUsage>,

    /// An opaque fingerprint identifying the backend configuration.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub system_fingerprint: Option<String>,

    /// Citations from search-augmented generation, if any.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub citations: Option<Vec<serde_json::Value>>,

    /// Files produced by code execution, if any.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub output_files: Option<Vec<OutputFile>>,
}

/// A file produced by server-side code execution in a Chat Completion.
///
/// This type is scoped to the Chat Completions module for now. If the
/// Responses API adds an equivalent, it can be extracted to a shared module.
#[deprecated(note = "Use the Responses API instead")]
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct OutputFile {
    /// Unique identifier for the file.
    pub file_id: String,

    /// The file name.
    pub name: String,

    /// MIME type of the file, if known.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub mime_type: Option<String>,

    /// Size of the file in bytes, if known.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub size_bytes: Option<u64>,
}

/// A single completion choice within a `ChatCompletion`.
#[deprecated(note = "Use the Responses API instead")]
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ChatChoice {
    /// Zero-based index of this choice.
    pub index: u32,

    /// The assistant message generated by the model.
    pub message: ChatMessage,

    /// Why the model stopped generating tokens.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub finish_reason: Option<FinishReason>,
}

/// An assistant message in a chat completion response.
///
/// This is distinct from `Message` (the request-side type) because the
/// response may include additional fields like `reasoning_content` and
/// `refusal`.
#[deprecated(note = "Use the Responses API instead")]
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ChatMessage {
    /// The role of the message author (typically `"assistant"`).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub role: Option<Role>,

    /// The text content of the message.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub content: Option<String>,

    /// Chain-of-thought reasoning produced by reasoning-capable models.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reasoning_content: Option<String>,

    /// A refusal message if the model declined the request.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub refusal: Option<String>,

    /// Tool calls the model wants to execute.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool_calls: Option<Vec<ToolCall>>,
}

/// The reason the model stopped generating tokens.
#[deprecated(note = "Use the Responses API instead")]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FinishReason {
    /// Natural stop (end of message or hit a stop sequence).
    Stop,
    /// Hit the token limit.
    Length,
    /// Model signalled end of turn.
    EndTurn,
    /// Model wants to call one or more tools.
    ToolCalls,
}

/// A deferred completion response for asynchronous processing.
///
/// When `deferred: true` is set on the request, the server returns a 202
/// with `request_id` and `status`. Polling `GET /v1/chat/deferred-completion/{request_id}`
/// returns this shape: 202 while processing (result is `None`), 200 when done
/// (result contains the full `ChatCompletion`).
#[deprecated(note = "Use the Responses API instead")]
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DeferredCompletion {
    /// The identifier to use when polling for the result.
    pub request_id: String,

    /// Processing status (e.g., `"queued"`, `"processing"`, `"complete"`).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub status: Option<String>,

    /// The completed result, present only when processing is finished.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub result: Option<ChatCompletion>,
}

// ---------------------------------------------------------------------------
// Builder
// ---------------------------------------------------------------------------

/// A builder for constructing `ChatCompletionRequest` instances.
///
/// All optional fields default to `None`. Only `model` and `messages` are
/// required; the builder enforces this at construction time via
/// `ChatCompletionBuilder::new`.
#[deprecated(note = "Use the Responses API instead")]
#[derive(Debug, Clone)]
pub struct ChatCompletionBuilder {
    model: String,
    messages: Vec<Message>,
    tools: Option<Vec<ToolDefinition>>,
    tool_choice: Option<serde_json::Value>,
    stream: Option<bool>,
    temperature: Option<f64>,
    top_p: Option<f64>,
    max_completion_tokens: Option<u64>,
    n: Option<u32>,
    stop: Option<Vec<String>>,
    seed: Option<i64>,
    frequency_penalty: Option<f64>,
    presence_penalty: Option<f64>,
    response_format: Option<ResponseFormat>,
    reasoning_effort: Option<String>,
    search_parameters: Option<serde_json::Value>,
    deferred: Option<bool>,
    stream_options: Option<StreamOptions>,
    parallel_tool_calls: Option<bool>,
}

#[allow(deprecated)]
impl ChatCompletionBuilder {
    /// Create a new builder with the required `model` and `messages`.
    pub fn new(model: impl Into<String>, messages: Vec<Message>) -> Self {
        Self {
            model: model.into(),
            messages,
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
        }
    }

    /// Set the model ID.
    #[must_use]
    pub fn model(mut self, model: impl Into<String>) -> Self {
        self.model = model.into();
        self
    }

    /// Replace the message history.
    #[must_use]
    pub fn messages(mut self, messages: Vec<Message>) -> Self {
        self.messages = messages;
        self
    }

    /// Set the tool definitions available for function calling.
    #[must_use]
    pub fn tools(mut self, tools: Vec<ToolDefinition>) -> Self {
        self.tools = Some(tools);
        self
    }

    /// Set the tool choice strategy.
    #[must_use]
    pub fn tool_choice(mut self, tool_choice: serde_json::Value) -> Self {
        self.tool_choice = Some(tool_choice);
        self
    }

    /// Enable or disable streaming.
    #[must_use]
    pub fn stream(mut self, stream: bool) -> Self {
        self.stream = Some(stream);
        self
    }

    /// Set the sampling temperature.
    #[must_use]
    pub fn temperature(mut self, temperature: f64) -> Self {
        self.temperature = Some(temperature);
        self
    }

    /// Set the nucleus sampling threshold.
    #[must_use]
    pub fn top_p(mut self, top_p: f64) -> Self {
        self.top_p = Some(top_p);
        self
    }

    /// Set the maximum number of completion tokens.
    #[must_use]
    pub fn max_completion_tokens(mut self, max: u64) -> Self {
        self.max_completion_tokens = Some(max);
        self
    }

    /// Set the number of completions to generate.
    #[must_use]
    pub fn n(mut self, n: u32) -> Self {
        self.n = Some(n);
        self
    }

    /// Set the stop sequences.
    #[must_use]
    pub fn stop(mut self, stop: Vec<String>) -> Self {
        self.stop = Some(stop);
        self
    }

    /// Set the deterministic sampling seed.
    #[must_use]
    pub fn seed(mut self, seed: i64) -> Self {
        self.seed = Some(seed);
        self
    }

    /// Set the frequency penalty.
    #[must_use]
    pub fn frequency_penalty(mut self, penalty: f64) -> Self {
        self.frequency_penalty = Some(penalty);
        self
    }

    /// Set the presence penalty.
    #[must_use]
    pub fn presence_penalty(mut self, penalty: f64) -> Self {
        self.presence_penalty = Some(penalty);
        self
    }

    /// Set the response format constraint.
    #[must_use]
    pub fn response_format(mut self, format: ResponseFormat) -> Self {
        self.response_format = Some(format);
        self
    }

    /// Set the reasoning effort hint.
    #[must_use]
    pub fn reasoning_effort(mut self, effort: impl Into<String>) -> Self {
        self.reasoning_effort = Some(effort.into());
        self
    }

    /// Set the search parameters for live-search augmented generation.
    #[must_use]
    pub fn search_parameters(mut self, params: serde_json::Value) -> Self {
        self.search_parameters = Some(params);
        self
    }

    /// Enable or disable deferred (asynchronous) processing.
    #[must_use]
    pub fn deferred(mut self, deferred: bool) -> Self {
        self.deferred = Some(deferred);
        self
    }

    /// Set the stream options.
    #[must_use]
    pub fn stream_options(mut self, options: StreamOptions) -> Self {
        self.stream_options = Some(options);
        self
    }

    /// Set whether the model should execute multiple tool calls in parallel.
    #[must_use]
    pub fn parallel_tool_calls(mut self, parallel: bool) -> Self {
        self.parallel_tool_calls = Some(parallel);
        self
    }

    /// Consume the builder and produce a `ChatCompletionRequest`.
    #[must_use]
    pub fn build(self) -> ChatCompletionRequest {
        ChatCompletionRequest {
            model: self.model,
            messages: self.messages,
            tools: self.tools,
            tool_choice: self.tool_choice,
            stream: self.stream,
            temperature: self.temperature,
            top_p: self.top_p,
            max_completion_tokens: self.max_completion_tokens,
            n: self.n,
            stop: self.stop,
            seed: self.seed,
            frequency_penalty: self.frequency_penalty,
            presence_penalty: self.presence_penalty,
            response_format: self.response_format,
            reasoning_effort: self.reasoning_effort,
            search_parameters: self.search_parameters,
            deferred: self.deferred,
            stream_options: self.stream_options,
            parallel_tool_calls: self.parallel_tool_calls,
        }
    }
}

#[cfg(test)]
#[allow(deprecated)]
mod tests {
    use super::*;
    use crate::types::tool::FunctionCall;

    // -- ChatCompletionRequest round-trips --

    #[test]
    fn request_round_trips_minimal() {
        let req =
            ChatCompletionBuilder::new("grok-4", vec![Message::text(Role::User, "Hello")]).build();

        let json = serde_json::to_string(&req).unwrap();
        let back: ChatCompletionRequest = serde_json::from_str(&json).unwrap();
        assert_eq!(req, back);
    }

    #[test]
    fn request_round_trips_all_fields() {
        let req = ChatCompletionBuilder::new(
            "grok-4-mini",
            vec![
                Message::text(Role::System, "You are a helpful assistant."),
                Message::text(Role::User, "What is 2+2?"),
            ],
        )
        .temperature(0.7)
        .top_p(0.9)
        .max_completion_tokens(1024)
        .n(2)
        .stop(vec!["END".into()])
        .seed(42)
        .frequency_penalty(0.5)
        .presence_penalty(-0.5)
        .reasoning_effort("high")
        .deferred(true)
        .stream(false)
        .stream_options(StreamOptions {
            include_usage: Some(true),
        })
        .response_format(ResponseFormat::Text)
        .search_parameters(serde_json::json!({"mode": "auto"}))
        .tool_choice(serde_json::json!("auto"))
        .build();

        let json = serde_json::to_string(&req).unwrap();
        let back: ChatCompletionRequest = serde_json::from_str(&json).unwrap();
        assert_eq!(req, back);
    }

    #[test]
    fn request_skips_none_optional_fields() {
        let req =
            ChatCompletionBuilder::new("grok-4", vec![Message::text(Role::User, "hi")]).build();

        let json = serde_json::to_string(&req).unwrap();
        assert!(!json.contains("\"tools\""));
        assert!(!json.contains("\"tool_choice\""));
        assert!(!json.contains("\"stream\""));
        assert!(!json.contains("\"temperature\""));
        assert!(!json.contains("\"top_p\""));
        assert!(!json.contains("\"max_completion_tokens\""));
        assert!(!json.contains("\"n\""));
        assert!(!json.contains("\"stop\""));
        assert!(!json.contains("\"seed\""));
        assert!(!json.contains("\"frequency_penalty\""));
        assert!(!json.contains("\"presence_penalty\""));
        assert!(!json.contains("\"response_format\""));
        assert!(!json.contains("\"reasoning_effort\""));
        assert!(!json.contains("\"search_parameters\""));
        assert!(!json.contains("\"deferred\""));
        assert!(!json.contains("\"stream_options\""));
    }

    #[test]
    fn request_with_deferred_true_serializes() {
        let req = ChatCompletionBuilder::new(
            "grok-4",
            vec![Message::text(Role::User, "Compute something complex")],
        )
        .deferred(true)
        .build();

        let json = serde_json::to_string(&req).unwrap();
        assert!(json.contains("\"deferred\":true"));
    }

    // -- Builder tests --

    #[test]
    fn builder_constructs_valid_request() {
        let req = ChatCompletionBuilder::new(
            "grok-4",
            vec![
                Message::text(Role::System, "System prompt"),
                Message::text(Role::User, "User message"),
            ],
        )
        .temperature(0.5)
        .max_completion_tokens(2048)
        .build();

        assert_eq!(req.model, "grok-4");
        assert_eq!(req.messages.len(), 2);
        assert_eq!(req.temperature, Some(0.5));
        assert_eq!(req.max_completion_tokens, Some(2048));
        assert!(req.tools.is_none());
        assert!(req.stream.is_none());
    }

    #[test]
    fn builder_with_tools() {
        let tool = ToolDefinition {
            r#type: "function".into(),
            function: crate::types::tool::FunctionDefinition {
                name: "get_weather".into(),
                description: Some("Get the weather".into()),
                parameters: Some(serde_json::json!({
                    "type": "object",
                    "properties": { "city": { "type": "string" } },
                    "required": ["city"]
                })),
            },
        };

        let req = ChatCompletionBuilder::new(
            "grok-4",
            vec![Message::text(Role::User, "What's the weather?")],
        )
        .tools(vec![tool.clone()])
        .tool_choice(serde_json::json!("auto"))
        .build();

        assert_eq!(req.tools.as_ref().unwrap().len(), 1);
        assert_eq!(req.tools.as_ref().unwrap()[0], tool);
    }

    // -- ChatCompletion deserialization --

    #[test]
    fn completion_deserializes_basic() {
        let json = r#"{
            "id": "chatcmpl-abc123",
            "object": "chat.completion",
            "created": 1700000000,
            "model": "grok-4",
            "choices": [
                {
                    "index": 0,
                    "message": {
                        "role": "assistant",
                        "content": "Hello! How can I help you?"
                    },
                    "finish_reason": "stop"
                }
            ],
            "usage": {
                "prompt_tokens": 10,
                "completion_tokens": 8,
                "total_tokens": 18
            },
            "system_fingerprint": "fp_abc123"
        }"#;

        let completion: ChatCompletion = serde_json::from_str(json).unwrap();
        assert_eq!(completion.id, "chatcmpl-abc123");
        assert_eq!(completion.object.as_deref(), Some("chat.completion"));
        assert_eq!(completion.created, Some(1700000000));
        assert_eq!(completion.model.as_deref(), Some("grok-4"));
        assert_eq!(completion.choices.len(), 1);
        assert_eq!(completion.choices[0].index, 0);
        assert_eq!(
            completion.choices[0].message.content.as_deref(),
            Some("Hello! How can I help you?")
        );
        assert_eq!(
            completion.choices[0].finish_reason,
            Some(FinishReason::Stop)
        );
        assert_eq!(completion.usage.as_ref().unwrap().prompt_tokens, 10);
        assert_eq!(completion.system_fingerprint.as_deref(), Some("fp_abc123"));
    }

    #[test]
    fn completion_with_tool_calls_deserializes() {
        let json = r#"{
            "id": "chatcmpl-tool-1",
            "choices": [
                {
                    "index": 0,
                    "message": {
                        "role": "assistant",
                        "content": null,
                        "tool_calls": [
                            {
                                "id": "call_abc123",
                                "type": "function",
                                "function": {
                                    "name": "get_weather",
                                    "arguments": "{\"city\":\"San Francisco\"}"
                                }
                            }
                        ]
                    },
                    "finish_reason": "tool_calls"
                }
            ]
        }"#;

        let completion: ChatCompletion = serde_json::from_str(json).unwrap();
        assert_eq!(completion.choices.len(), 1);
        let msg = &completion.choices[0].message;
        assert!(msg.content.is_none());
        let tool_calls = msg.tool_calls.as_ref().unwrap();
        assert_eq!(tool_calls.len(), 1);
        assert_eq!(tool_calls[0].id, "call_abc123");
        assert_eq!(tool_calls[0].function.name, "get_weather");
        assert_eq!(
            tool_calls[0].function.arguments,
            r#"{"city":"San Francisco"}"#
        );
        assert_eq!(
            completion.choices[0].finish_reason,
            Some(FinishReason::ToolCalls)
        );
    }

    #[test]
    fn completion_with_reasoning_content_deserializes() {
        let json = r#"{
            "id": "chatcmpl-reasoning-1",
            "choices": [
                {
                    "index": 0,
                    "message": {
                        "role": "assistant",
                        "content": "The answer is 4.",
                        "reasoning_content": "Let me think step by step. 2 + 2 = 4."
                    },
                    "finish_reason": "stop"
                }
            ]
        }"#;

        let completion: ChatCompletion = serde_json::from_str(json).unwrap();
        let msg = &completion.choices[0].message;
        assert_eq!(msg.content.as_deref(), Some("The answer is 4."));
        assert_eq!(
            msg.reasoning_content.as_deref(),
            Some("Let me think step by step. 2 + 2 = 4.")
        );
    }

    // -- FinishReason round-trips --

    #[test]
    fn finish_reason_stop_round_trips() {
        let json = serde_json::to_string(&FinishReason::Stop).unwrap();
        assert_eq!(json, "\"stop\"");
        let back: FinishReason = serde_json::from_str(&json).unwrap();
        assert_eq!(back, FinishReason::Stop);
    }

    #[test]
    fn finish_reason_length_round_trips() {
        let json = serde_json::to_string(&FinishReason::Length).unwrap();
        assert_eq!(json, "\"length\"");
        let back: FinishReason = serde_json::from_str(&json).unwrap();
        assert_eq!(back, FinishReason::Length);
    }

    #[test]
    fn finish_reason_end_turn_round_trips() {
        let json = serde_json::to_string(&FinishReason::EndTurn).unwrap();
        assert_eq!(json, "\"end_turn\"");
        let back: FinishReason = serde_json::from_str(&json).unwrap();
        assert_eq!(back, FinishReason::EndTurn);
    }

    #[test]
    fn finish_reason_tool_calls_round_trips() {
        let json = serde_json::to_string(&FinishReason::ToolCalls).unwrap();
        assert_eq!(json, "\"tool_calls\"");
        let back: FinishReason = serde_json::from_str(&json).unwrap();
        assert_eq!(back, FinishReason::ToolCalls);
    }

    // -- DeferredCompletion --

    #[test]
    fn deferred_completion_202_shape_deserializes() {
        let json = r#"{
            "request_id": "req_abc123",
            "status": "processing"
        }"#;

        let deferred: DeferredCompletion = serde_json::from_str(json).unwrap();
        assert_eq!(deferred.request_id, "req_abc123");
        assert_eq!(deferred.status.as_deref(), Some("processing"));
        assert!(deferred.result.is_none());
    }

    #[test]
    fn deferred_completion_200_shape_deserializes() {
        let json = r#"{
            "request_id": "req_abc123",
            "status": "complete",
            "result": {
                "id": "chatcmpl-done-1",
                "choices": [
                    {
                        "index": 0,
                        "message": {
                            "role": "assistant",
                            "content": "Done processing."
                        },
                        "finish_reason": "stop"
                    }
                ]
            }
        }"#;

        let deferred: DeferredCompletion = serde_json::from_str(json).unwrap();
        assert_eq!(deferred.request_id, "req_abc123");
        assert_eq!(deferred.status.as_deref(), Some("complete"));
        let result = deferred.result.unwrap();
        assert_eq!(result.id, "chatcmpl-done-1");
        assert_eq!(result.choices.len(), 1);
        assert_eq!(
            result.choices[0].message.content.as_deref(),
            Some("Done processing.")
        );
    }

    // -- Unknown fields tolerance --

    #[test]
    fn completion_tolerates_unknown_fields() {
        let json = r#"{
            "id": "chatcmpl-unk",
            "choices": [
                {
                    "index": 0,
                    "message": {
                        "role": "assistant",
                        "content": "ok"
                    },
                    "finish_reason": "stop",
                    "logprobs": null
                }
            ],
            "future_field": "some_value",
            "another_unknown": 42
        }"#;

        let completion: ChatCompletion = serde_json::from_str(json).unwrap();
        assert_eq!(completion.id, "chatcmpl-unk");
        assert_eq!(completion.choices.len(), 1);
    }

    #[test]
    fn chat_message_tolerates_unknown_fields() {
        let json = r#"{
            "role": "assistant",
            "content": "hi",
            "audio": null,
            "function_call": null
        }"#;

        let msg: ChatMessage = serde_json::from_str(json).unwrap();
        assert_eq!(msg.role, Some(Role::Assistant));
        assert_eq!(msg.content.as_deref(), Some("hi"));
    }

    #[test]
    fn deferred_completion_tolerates_unknown_fields() {
        let json = r#"{
            "request_id": "req_1",
            "status": "queued",
            "eta_seconds": 30
        }"#;

        let deferred: DeferredCompletion = serde_json::from_str(json).unwrap();
        assert_eq!(deferred.request_id, "req_1");
        assert_eq!(deferred.status.as_deref(), Some("queued"));
    }

    // -- ResponseFormat --

    #[test]
    fn response_format_text_round_trips() {
        let fmt = ResponseFormat::Text;
        let json = serde_json::to_string(&fmt).unwrap();
        assert!(json.contains("\"type\":\"text\""));
        let back: ResponseFormat = serde_json::from_str(&json).unwrap();
        assert_eq!(fmt, back);
    }

    #[test]
    fn response_format_json_schema_round_trips() {
        let fmt = ResponseFormat::JsonSchema {
            json_schema: JsonSchemaSpec {
                name: "answer".into(),
                strict: Some(true),
                schema: serde_json::json!({
                    "type": "object",
                    "properties": {
                        "value": { "type": "integer" }
                    }
                }),
            },
        };
        let json = serde_json::to_string(&fmt).unwrap();
        assert!(json.contains("\"type\":\"json_schema\""));
        let back: ResponseFormat = serde_json::from_str(&json).unwrap();
        assert_eq!(fmt, back);
    }

    #[test]
    fn response_format_json_object_round_trips() {
        let fmt = ResponseFormat::JsonObject;
        let json = serde_json::to_string(&fmt).unwrap();
        assert!(json.contains("\"type\":\"json_object\""));
        let back: ResponseFormat = serde_json::from_str(&json).unwrap();
        assert_eq!(fmt, back);
    }

    // -- StreamOptions --

    #[test]
    fn stream_options_round_trips() {
        let opts = StreamOptions {
            include_usage: Some(true),
        };
        let json = serde_json::to_string(&opts).unwrap();
        let back: StreamOptions = serde_json::from_str(&json).unwrap();
        assert_eq!(opts, back);
    }

    #[test]
    fn stream_options_skips_none() {
        let opts = StreamOptions {
            include_usage: None,
        };
        let json = serde_json::to_string(&opts).unwrap();
        assert!(!json.contains("\"include_usage\""));
    }

    // -- ChatMessage with tool_calls --

    #[test]
    fn chat_message_with_tool_calls_round_trips() {
        let msg = ChatMessage {
            role: Some(Role::Assistant),
            content: None,
            reasoning_content: None,
            refusal: None,
            tool_calls: Some(vec![
                ToolCall {
                    id: "call_1".into(),
                    r#type: "function".into(),
                    function: FunctionCall {
                        name: "search".into(),
                        arguments: r#"{"q":"rust"}"#.into(),
                    },
                },
                ToolCall {
                    id: "call_2".into(),
                    r#type: "function".into(),
                    function: FunctionCall {
                        name: "calculate".into(),
                        arguments: r#"{"expr":"2+2"}"#.into(),
                    },
                },
            ]),
        };

        let json = serde_json::to_string(&msg).unwrap();
        let back: ChatMessage = serde_json::from_str(&json).unwrap();
        assert_eq!(msg, back);
        assert_eq!(back.tool_calls.as_ref().unwrap().len(), 2);
    }

    // -- ChatCompletion with citations --

    #[test]
    fn completion_with_citations_deserializes() {
        let json = r#"{
            "id": "chatcmpl-cite-1",
            "choices": [
                {
                    "index": 0,
                    "message": {
                        "role": "assistant",
                        "content": "According to sources..."
                    },
                    "finish_reason": "stop"
                }
            ],
            "citations": [
                {"url": "https://example.com", "title": "Example"}
            ]
        }"#;

        let completion: ChatCompletion = serde_json::from_str(json).unwrap();
        let citations = completion.citations.unwrap();
        assert_eq!(citations.len(), 1);
        assert_eq!(citations[0]["url"], "https://example.com");
    }

    // -- ChatCompletion round-trips --

    #[test]
    fn completion_round_trips() {
        let completion = ChatCompletion {
            id: "chatcmpl-rt".into(),
            object: Some("chat.completion".into()),
            created: Some(1700000000),
            model: Some("grok-4".into()),
            choices: vec![ChatChoice {
                index: 0,
                message: ChatMessage {
                    role: Some(Role::Assistant),
                    content: Some("Hello".into()),
                    reasoning_content: None,
                    refusal: None,
                    tool_calls: None,
                },
                finish_reason: Some(FinishReason::Stop),
            }],
            usage: Some(ChatUsage {
                prompt_tokens: 5,
                completion_tokens: 3,
                total_tokens: Some(8),
                prompt_tokens_details: None,
                completion_tokens_details: None,
                cost_in_usd_ticks: None,
                num_sources_used: None,
            }),
            system_fingerprint: Some("fp_test".into()),
            citations: None,
            output_files: None,
        };

        let json = serde_json::to_string(&completion).unwrap();
        let back: ChatCompletion = serde_json::from_str(&json).unwrap();
        assert_eq!(completion, back);
    }

    // -- ChatMessage with refusal --

    #[test]
    fn chat_message_with_refusal_deserializes() {
        let json = r#"{
            "role": "assistant",
            "content": null,
            "refusal": "I cannot help with that request."
        }"#;

        let msg: ChatMessage = serde_json::from_str(json).unwrap();
        assert!(msg.content.is_none());
        assert_eq!(
            msg.refusal.as_deref(),
            Some("I cannot help with that request.")
        );
    }

    // -- Multiple choices --

    #[test]
    fn completion_with_multiple_choices_deserializes() {
        let json = r#"{
            "id": "chatcmpl-multi",
            "choices": [
                {
                    "index": 0,
                    "message": { "role": "assistant", "content": "Answer A" },
                    "finish_reason": "stop"
                },
                {
                    "index": 1,
                    "message": { "role": "assistant", "content": "Answer B" },
                    "finish_reason": "stop"
                }
            ]
        }"#;

        let completion: ChatCompletion = serde_json::from_str(json).unwrap();
        assert_eq!(completion.choices.len(), 2);
        assert_eq!(completion.choices[0].index, 0);
        assert_eq!(completion.choices[1].index, 1);
        assert_eq!(
            completion.choices[0].message.content.as_deref(),
            Some("Answer A")
        );
        assert_eq!(
            completion.choices[1].message.content.as_deref(),
            Some("Answer B")
        );
    }

    // -- OutputFile / output_files --

    #[test]
    fn output_file_round_trips() {
        let file = OutputFile {
            file_id: "file_abc123".into(),
            name: "chart.png".into(),
            mime_type: Some("image/png".into()),
            size_bytes: Some(4096),
        };
        let json = serde_json::to_string(&file).unwrap();
        let back: OutputFile = serde_json::from_str(&json).unwrap();
        assert_eq!(file, back);
    }

    #[test]
    fn output_file_skips_none_optional_fields() {
        let file = OutputFile {
            file_id: "file_1".into(),
            name: "data.csv".into(),
            mime_type: None,
            size_bytes: None,
        };
        let json = serde_json::to_string(&file).unwrap();
        assert!(!json.contains("\"mime_type\""));
        assert!(!json.contains("\"size_bytes\""));
    }

    #[test]
    fn completion_with_output_files_deserializes() {
        let json = r#"{
            "id": "chatcmpl-files-1",
            "choices": [
                {
                    "index": 0,
                    "message": {
                        "role": "assistant",
                        "content": "Here is your chart."
                    },
                    "finish_reason": "stop"
                }
            ],
            "output_files": [
                {
                    "file_id": "file_xyz",
                    "name": "chart.png",
                    "mime_type": "image/png",
                    "size_bytes": 12345
                },
                {
                    "file_id": "file_abc",
                    "name": "data.csv"
                }
            ]
        }"#;

        let completion: ChatCompletion = serde_json::from_str(json).unwrap();
        let files = completion.output_files.unwrap();
        assert_eq!(files.len(), 2);
        assert_eq!(files[0].file_id, "file_xyz");
        assert_eq!(files[0].name, "chart.png");
        assert_eq!(files[0].mime_type.as_deref(), Some("image/png"));
        assert_eq!(files[0].size_bytes, Some(12345));
        assert_eq!(files[1].file_id, "file_abc");
        assert_eq!(files[1].name, "data.csv");
        assert!(files[1].mime_type.is_none());
        assert!(files[1].size_bytes.is_none());
    }

    #[test]
    fn completion_without_output_files_deserializes() {
        let json = r#"{
            "id": "chatcmpl-no-files",
            "choices": [
                {
                    "index": 0,
                    "message": {
                        "role": "assistant",
                        "content": "Hello"
                    },
                    "finish_reason": "stop"
                }
            ]
        }"#;

        let completion: ChatCompletion = serde_json::from_str(json).unwrap();
        assert!(completion.output_files.is_none());
    }

    #[test]
    fn completion_output_files_skipped_when_none() {
        let completion = ChatCompletion {
            id: "chatcmpl-skip".into(),
            object: None,
            created: None,
            model: None,
            choices: vec![],
            usage: None,
            system_fingerprint: None,
            citations: None,
            output_files: None,
        };
        let json = serde_json::to_string(&completion).unwrap();
        assert!(!json.contains("\"output_files\""));
    }
}
