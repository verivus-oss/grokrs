use serde::{Deserialize, Serialize};

use super::common::{ContentBlock, Role};
use super::message::InputMessage;
use super::usage::Usage;

// ---------------------------------------------------------------------------
// Request types
// ---------------------------------------------------------------------------

/// The input for a Responses API request.
///
/// Can be either a plain text string or a structured list of input items.
/// Uses serde `untagged` so that a JSON string maps to `Text` and a JSON array
/// maps to `Items`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum ResponseInput {
    /// A plain text prompt string.
    Text(String),
    /// A list of structured input items (messages, reasoning replay, etc.).
    Items(Vec<InputItem>),
}

/// A single input item in a Responses API request.
///
/// Tagged on the `type` field when present. `InputMessage`-style objects
/// (with `role` + `content`) lack a `type` field in the wire format, so
/// they deserialize via the `untagged` fallback.
///
/// New variants should be added here as xAI extends the input schema.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum InputItem {
    /// A structured message (user, assistant, system, tool).
    ///
    /// This is the most common input item. It does NOT carry a `type` tag
    /// in the wire format — serde matches it via the `role` + `content`
    /// shape.
    Message(InputMessage),

    /// Encrypted reasoning content for multi-turn reasoning continuity.
    ///
    /// When a previous response included `encrypted_content` in its
    /// `OutputItem::Reasoning`, the client can replay that blob here so
    /// the model can continue its reasoning chain without exposing the
    /// raw chain-of-thought.
    ///
    /// Wire format:
    /// ```json
    /// { "type": "reasoning", "id": "rs_...", "encrypted_content": "<opaque>" }
    /// ```
    ///
    /// The `encrypted_content` is treated as fully opaque — never parsed,
    /// decrypted, or validated by the client.
    Reasoning {
        /// Discriminator tag — always `"reasoning"`.
        r#type: String,
        /// The reasoning block ID from the original response.
        #[serde(skip_serializing_if = "Option::is_none")]
        id: Option<String>,
        /// The opaque encrypted reasoning blob.
        encrypted_content: String,
    },
}

/// Configuration for reasoning effort.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReasoningConfig {
    /// Effort level: "low", "medium", or "high".
    #[serde(skip_serializing_if = "Option::is_none")]
    pub effort: Option<String>,

    /// Whether to generate a reasoning summary.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub generate_summary: Option<bool>,

    /// The detail level of the reasoning summary.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub summary: Option<ReasoningSummary>,
}

/// Detail level for reasoning summaries.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ReasoningSummary {
    /// Let the model decide the summary detail level.
    Auto,
    /// A brief, concise summary.
    Concise,
    /// A thorough, detailed summary.
    Detailed,
}

/// Configuration for text output format.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TextConfig {
    /// The output text format.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub format: Option<TextFormat>,
}

/// Text output format variants.
///
/// Tagged on the `type` field. `Text` produces `{"type":"text"}` and
/// `JsonSchema` produces `{"type":"json_schema","name":"...","schema":{...}}`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum TextFormat {
    /// Plain text output.
    Text,
    /// Structured JSON output constrained by a JSON Schema.
    JsonSchema {
        /// The name for the schema (used in validation/diagnostics).
        name: String,
        /// Whether the model must strictly follow the schema.
        #[serde(skip_serializing_if = "Option::is_none")]
        strict: Option<bool>,
        /// The JSON Schema definition.
        schema: serde_json::Value,
    },
}

/// Returns `false`, used as a serde default for the `store` field.
fn default_store_false() -> Option<bool> {
    Some(false)
}

/// Request body for the Responses API `POST /v1/responses` endpoint.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CreateResponseRequest {
    /// The model to use (e.g., "grok-4").
    pub model: String,

    /// The input prompt — either a string or structured messages.
    pub input: ResponseInput,

    /// System instructions for the model.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub instructions: Option<String>,

    /// Tool definitions (function defs or built-in tool objects).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tools: Option<Vec<serde_json::Value>>,

    /// How the model should choose which tool to call.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool_choice: Option<serde_json::Value>,

    /// ID of a previous response to continue a multi-turn conversation.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub previous_response_id: Option<String>,

    /// Whether xAI should store the response server-side.
    ///
    /// grokrs overrides xAI's default of `true` to `false`. The serde default
    /// ensures that a deserialized request without this field gets `Some(false)`.
    #[serde(default = "default_store_false")]
    pub store: Option<bool>,

    /// Whether to stream the response via SSE.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub stream: Option<bool>,

    /// Sampling temperature (0.0 to 2.0).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub temperature: Option<f64>,

    /// Nucleus sampling probability (0.0 to 1.0).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub top_p: Option<f64>,

    /// Maximum number of output tokens.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_output_tokens: Option<u64>,

    /// Maximum number of agentic turns the model may take.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_turns: Option<u64>,

    /// Reasoning configuration (effort level).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reasoning: Option<ReasoningConfig>,

    /// Text output format configuration.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub text: Option<TextConfig>,

    /// Parameters for built-in search tools.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub search_parameters: Option<serde_json::Value>,

    /// Arbitrary metadata to attach to the response.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub metadata: Option<serde_json::Value>,

    /// Whether the model should execute multiple tool calls in parallel.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub parallel_tool_calls: Option<bool>,

    /// Which output types to include in the response (e.g., `["reasoning"]`).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub include: Option<Vec<String>>,

    /// Context management directives (schema not yet finalized by xAI).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub context_management: Option<Vec<serde_json::Value>>,

    /// An opaque key used to enable prompt caching on the server.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub prompt_cache_key: Option<String>,
}

// ---------------------------------------------------------------------------
// Response types
// ---------------------------------------------------------------------------

/// Completion status of a response.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ResponseStatus {
    /// The response completed successfully.
    Completed,
    /// The response is still being generated.
    #[serde(rename = "in_progress")]
    InProgress,
    /// The response was truncated or otherwise incomplete.
    Incomplete,
    /// The response generation failed.
    Failed,
}

/// A single reasoning content block in a reasoning output item.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReasoningBlock {
    /// The type of the reasoning block (e.g., "thinking").
    pub r#type: String,
    /// The reasoning text.
    pub text: String,
}

/// An output item in a Responses API response.
///
/// Tagged on the `type` field. Does NOT use `deny_unknown_fields` so that
/// new fields added by xAI do not break deserialization.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum OutputItem {
    /// A text message from the model.
    Message {
        /// The role of the message author.
        role: Role,
        /// Content blocks within the message.
        content: Vec<ContentBlock>,
    },
    /// A reasoning trace from the model.
    Reasoning {
        /// Optional unique identifier for this reasoning block.
        #[serde(skip_serializing_if = "Option::is_none")]
        id: Option<String>,
        /// The reasoning content blocks.
        content: Vec<ReasoningBlock>,
        /// Encrypted reasoning content for continuity across turns.
        ///
        /// When `include: ["reasoning.encrypted_content"]` is sent in the
        /// request, Grok 4 models return an opaque encrypted blob here.
        /// This blob can be passed back in a subsequent turn's input
        /// (via [`InputItem::Reasoning`]) to give the model reasoning
        /// continuity without exposing raw chain-of-thought.
        ///
        /// Treated as fully opaque — never parsed, decrypted, or validated.
        #[serde(skip_serializing_if = "Option::is_none")]
        encrypted_content: Option<String>,
    },
    /// A function call the model wants to make.
    FunctionCall {
        /// Unique identifier for this output item.
        id: String,
        /// The call ID used to match function call outputs.
        call_id: String,
        /// The name of the function to call.
        name: String,
        /// The arguments as a JSON string.
        arguments: String,
    },
    /// The output/result of a function call, returned to the model.
    FunctionCallOutput {
        /// The call ID this output corresponds to.
        call_id: String,
        /// The function output as a string.
        output: String,
    },

    /// A server-side web search tool call.
    #[serde(rename = "web_search_call")]
    WebSearchCall {
        /// Unique identifier for this tool call.
        id: String,
        /// Status of the tool call (e.g., `"completed"`).
        #[serde(skip_serializing_if = "Option::is_none")]
        status: Option<String>,
        /// Search results returned by the tool, if any.
        #[serde(skip_serializing_if = "Option::is_none")]
        search_results: Option<Vec<serde_json::Value>>,
    },

    /// A server-side X (formerly Twitter) search tool call.
    #[serde(rename = "x_search_call")]
    XSearchCall {
        /// Unique identifier for this tool call.
        id: String,
        /// Status of the tool call (e.g., `"completed"`).
        #[serde(skip_serializing_if = "Option::is_none")]
        status: Option<String>,
        /// Search results returned by the tool, if any.
        #[serde(skip_serializing_if = "Option::is_none")]
        search_results: Option<Vec<serde_json::Value>>,
    },

    /// A server-side code interpreter tool call.
    #[serde(rename = "code_interpreter_call")]
    CodeInterpreterCall {
        /// Unique identifier for this tool call.
        id: String,
        /// Status of the tool call (e.g., `"completed"`).
        #[serde(skip_serializing_if = "Option::is_none")]
        status: Option<String>,
        /// The code that was executed.
        #[serde(skip_serializing_if = "Option::is_none")]
        code: Option<String>,
        /// Outputs produced by the code interpreter.
        #[serde(skip_serializing_if = "Option::is_none")]
        outputs: Option<Vec<serde_json::Value>>,
    },

    /// A server-side file search tool call.
    #[serde(rename = "file_search_call")]
    FileSearchCall {
        /// Unique identifier for this tool call.
        id: String,
        /// Status of the tool call (e.g., `"completed"`).
        #[serde(skip_serializing_if = "Option::is_none")]
        status: Option<String>,
        /// Search results returned by the file search.
        #[serde(skip_serializing_if = "Option::is_none")]
        results: Option<Vec<serde_json::Value>>,
    },

    /// A server-side MCP (Model Context Protocol) tool call.
    #[serde(rename = "mcp_call")]
    McpCall {
        /// Unique identifier for this tool call.
        id: String,
        /// Status of the tool call (e.g., `"completed"`).
        #[serde(skip_serializing_if = "Option::is_none")]
        status: Option<String>,
    },

    /// An unrecognised output item type from the server.
    ///
    /// Provides forward compatibility — new output types added by xAI will
    /// deserialize into this variant instead of causing a parse error.
    #[serde(other)]
    Unknown,
}

/// A complete response object from the Responses API.
///
/// Does NOT use `deny_unknown_fields` so that new fields added by xAI do
/// not break deserialization.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ResponseObject {
    /// Unique identifier for the response.
    pub id: String,

    /// The object type (e.g., "response").
    #[serde(skip_serializing_if = "Option::is_none")]
    pub object: Option<String>,

    /// The completion status.
    pub status: ResponseStatus,

    /// The output items produced by the model.
    pub output: Vec<OutputItem>,

    /// Token usage statistics.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub usage: Option<Usage>,

    /// The model that generated the response.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,

    /// The system instructions that were used.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub instructions: Option<String>,

    /// Metadata attached to the response.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub metadata: Option<serde_json::Value>,

    /// ID of the previous response in a multi-turn conversation.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub previous_response_id: Option<String>,

    /// The sampling temperature used.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub temperature: Option<f64>,

    /// The nucleus sampling probability used.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub top_p: Option<f64>,

    /// The maximum output tokens setting used.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_output_tokens: Option<u64>,
}

// ---------------------------------------------------------------------------
// Builder
// ---------------------------------------------------------------------------

/// Builder for constructing `CreateResponseRequest` with sensible defaults.
///
/// The builder defaults `store` to `false`, overriding xAI's server-side
/// default of `true`. This is a deliberate safety choice — grokrs does not
/// store responses on xAI servers unless explicitly opted in.
#[derive(Debug, Clone)]
pub struct CreateResponseBuilder {
    model: String,
    input: ResponseInput,
    instructions: Option<String>,
    tools: Option<Vec<serde_json::Value>>,
    tool_choice: Option<serde_json::Value>,
    previous_response_id: Option<String>,
    store: Option<bool>,
    stream: Option<bool>,
    temperature: Option<f64>,
    top_p: Option<f64>,
    max_output_tokens: Option<u64>,
    max_turns: Option<u64>,
    reasoning: Option<ReasoningConfig>,
    text: Option<TextConfig>,
    search_parameters: Option<serde_json::Value>,
    metadata: Option<serde_json::Value>,
    parallel_tool_calls: Option<bool>,
    include: Option<Vec<String>>,
    context_management: Option<Vec<serde_json::Value>>,
    prompt_cache_key: Option<String>,
}

impl CreateResponseBuilder {
    /// Create a new builder with the given model and text input.
    ///
    /// `store` defaults to `false`.
    pub fn new(model: impl Into<String>, input: ResponseInput) -> Self {
        Self {
            model: model.into(),
            input,
            instructions: None,
            tools: None,
            tool_choice: None,
            previous_response_id: None,
            store: Some(false),
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
        }
    }

    /// Set the model name.
    pub fn model(mut self, model: impl Into<String>) -> Self {
        self.model = model.into();
        self
    }

    /// Set the input to a plain text string.
    pub fn input_text(mut self, text: impl Into<String>) -> Self {
        self.input = ResponseInput::Text(text.into());
        self
    }

    /// Set the input to structured messages.
    ///
    /// Wraps each `InputMessage` in [`InputItem::Message`].
    pub fn input_messages(mut self, messages: Vec<InputMessage>) -> Self {
        self.input = ResponseInput::Items(messages.into_iter().map(InputItem::Message).collect());
        self
    }

    /// Set the input to a list of structured input items.
    ///
    /// Use this when the input contains a mix of messages and other item
    /// types (e.g. reasoning replay items).
    pub fn input_items(mut self, items: Vec<InputItem>) -> Self {
        self.input = ResponseInput::Items(items);
        self
    }

    /// Set the system instructions.
    pub fn instructions(mut self, instructions: impl Into<String>) -> Self {
        self.instructions = Some(instructions.into());
        self
    }

    /// Set the tool definitions.
    pub fn tools(mut self, tools: Vec<serde_json::Value>) -> Self {
        self.tools = Some(tools);
        self
    }

    /// Set the tool choice strategy.
    pub fn tool_choice(mut self, tool_choice: serde_json::Value) -> Self {
        self.tool_choice = Some(tool_choice);
        self
    }

    /// Chain this request to a previous response by ID.
    ///
    /// # Panics
    ///
    /// Panics if `id` is empty. The xAI API requires a non-empty opaque
    /// string for stateful conversation chaining.
    pub fn previous_response_id(mut self, id: impl Into<String>) -> Self {
        let id = id.into();
        assert!(!id.is_empty(), "previous_response_id must not be empty");
        self.previous_response_id = Some(id);
        self
    }

    /// Set whether xAI should store the response server-side.
    ///
    /// Defaults to `false` in the builder. Pass `true` to opt in.
    pub fn store(mut self, store: bool) -> Self {
        self.store = Some(store);
        self
    }

    /// Set whether to stream the response via SSE.
    pub fn stream(mut self, stream: bool) -> Self {
        self.stream = Some(stream);
        self
    }

    /// Set the sampling temperature.
    pub fn temperature(mut self, temperature: f64) -> Self {
        self.temperature = Some(temperature);
        self
    }

    /// Set the nucleus sampling probability.
    pub fn top_p(mut self, top_p: f64) -> Self {
        self.top_p = Some(top_p);
        self
    }

    /// Set the maximum number of output tokens.
    pub fn max_output_tokens(mut self, max_output_tokens: u64) -> Self {
        self.max_output_tokens = Some(max_output_tokens);
        self
    }

    /// Set the maximum number of agentic turns.
    pub fn max_turns(mut self, max_turns: u64) -> Self {
        self.max_turns = Some(max_turns);
        self
    }

    /// Set the reasoning configuration.
    pub fn reasoning(mut self, reasoning: ReasoningConfig) -> Self {
        self.reasoning = Some(reasoning);
        self
    }

    /// Set the text output format configuration.
    pub fn text(mut self, text: TextConfig) -> Self {
        self.text = Some(text);
        self
    }

    /// Set the search parameters.
    pub fn search_parameters(mut self, search_parameters: serde_json::Value) -> Self {
        self.search_parameters = Some(search_parameters);
        self
    }

    /// Set arbitrary metadata.
    pub fn metadata(mut self, metadata: serde_json::Value) -> Self {
        self.metadata = Some(metadata);
        self
    }

    /// Set whether the model should execute multiple tool calls in parallel.
    pub fn parallel_tool_calls(mut self, parallel: bool) -> Self {
        self.parallel_tool_calls = Some(parallel);
        self
    }

    /// Set the output types to include in the response.
    pub fn include(mut self, include: Vec<String>) -> Self {
        self.include = Some(include);
        self
    }

    /// Set context management directives.
    pub fn context_management(mut self, context_management: Vec<serde_json::Value>) -> Self {
        self.context_management = Some(context_management);
        self
    }

    /// Set the prompt cache key.
    pub fn prompt_cache_key(mut self, key: impl Into<String>) -> Self {
        self.prompt_cache_key = Some(key.into());
        self
    }

    /// Consume the builder and produce the request.
    pub fn build(self) -> CreateResponseRequest {
        CreateResponseRequest {
            model: self.model,
            input: self.input,
            instructions: self.instructions,
            tools: self.tools,
            tool_choice: self.tool_choice,
            previous_response_id: self.previous_response_id,
            store: self.store,
            stream: self.stream,
            temperature: self.temperature,
            top_p: self.top_p,
            max_output_tokens: self.max_output_tokens,
            max_turns: self.max_turns,
            reasoning: self.reasoning,
            text: self.text,
            search_parameters: self.search_parameters,
            metadata: self.metadata,
            parallel_tool_calls: self.parallel_tool_calls,
            include: self.include,
            context_management: self.context_management,
            prompt_cache_key: self.prompt_cache_key,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::common::Role;
    use crate::types::message::InputMessage;

    // -- CreateResponseRequest round-trip --

    #[test]
    fn create_response_request_round_trips() {
        let req = CreateResponseBuilder::new("grok-4", ResponseInput::Text("Hello, Grok!".into()))
            .instructions("Be concise.")
            .temperature(0.7)
            .build();

        let json = serde_json::to_string(&req).unwrap();
        let back: CreateResponseRequest = serde_json::from_str(&json).unwrap();
        assert_eq!(req.model, back.model);
        assert_eq!(req.input, back.input);
        assert_eq!(req.instructions, back.instructions);
        assert_eq!(req.temperature, back.temperature);
        assert_eq!(req.store, back.store);
    }

    // -- Builder defaults store to false --

    #[test]
    fn builder_defaults_store_to_false() {
        let req = CreateResponseBuilder::new("grok-4", ResponseInput::Text("test".into())).build();
        assert_eq!(req.store, Some(false));
    }

    #[test]
    fn builder_store_can_be_overridden_to_true() {
        let req = CreateResponseBuilder::new("grok-4", ResponseInput::Text("test".into()))
            .store(true)
            .build();
        assert_eq!(req.store, Some(true));
    }

    #[test]
    fn store_serializes_as_false_by_default() {
        let req = CreateResponseBuilder::new("grok-4", ResponseInput::Text("test".into())).build();
        let json = serde_json::to_string(&req).unwrap();
        assert!(json.contains("\"store\":false"));
    }

    #[test]
    fn store_defaults_to_false_on_deserialization_when_missing() {
        let json = r#"{"model":"grok-4","input":"test"}"#;
        let req: CreateResponseRequest = serde_json::from_str(json).unwrap();
        assert_eq!(req.store, Some(false));
    }

    // -- ResponseInput variants --

    #[test]
    fn response_input_text_serializes_as_string() {
        let input = ResponseInput::Text("Hello".into());
        let json = serde_json::to_string(&input).unwrap();
        assert_eq!(json, "\"Hello\"");
    }

    #[test]
    fn response_input_messages_serializes_as_array() {
        let input = ResponseInput::Items(vec![InputItem::Message(InputMessage::text(
            Role::User,
            "Hello",
        ))]);
        let json = serde_json::to_string(&input).unwrap();
        assert!(json.starts_with('['));
        assert!(json.contains("\"role\":\"user\""));
    }

    #[test]
    fn response_input_text_deserializes_from_string() {
        let json = "\"Hello\"";
        let input: ResponseInput = serde_json::from_str(json).unwrap();
        assert_eq!(input, ResponseInput::Text("Hello".into()));
    }

    #[test]
    fn response_input_messages_deserializes_from_array() {
        let json = r#"[{"role":"user","content":"Hello"}]"#;
        let input: ResponseInput = serde_json::from_str(json).unwrap();
        match input {
            ResponseInput::Items(items) => {
                assert_eq!(items.len(), 1);
                match &items[0] {
                    InputItem::Message(msg) => assert_eq!(msg.role, Role::User),
                    other => panic!("expected Message, got: {other:?}"),
                }
            }
            _ => panic!("expected Items variant"),
        }
    }

    // -- OutputItem variants --

    #[test]
    fn output_item_message_deserializes() {
        let json = r#"{
            "type": "message",
            "role": "assistant",
            "content": [{"type":"text","text":"Hi there!"}]
        }"#;
        let item: OutputItem = serde_json::from_str(json).unwrap();
        match item {
            OutputItem::Message { role, content } => {
                assert_eq!(role, Role::Assistant);
                assert_eq!(content.len(), 1);
            }
            other => panic!("expected Message, got: {other:?}"),
        }
    }

    #[test]
    fn output_item_reasoning_deserializes() {
        let json = r#"{
            "type": "reasoning",
            "id": "reasoning_1",
            "content": [{"type":"thinking","text":"Let me think..."}]
        }"#;
        let item: OutputItem = serde_json::from_str(json).unwrap();
        match item {
            OutputItem::Reasoning {
                id,
                content,
                encrypted_content,
            } => {
                assert_eq!(id, Some("reasoning_1".into()));
                assert_eq!(content.len(), 1);
                assert_eq!(content[0].text, "Let me think...");
                assert!(
                    encrypted_content.is_none(),
                    "encrypted_content should be None when not present in JSON"
                );
            }
            other => panic!("expected Reasoning, got: {other:?}"),
        }
    }

    #[test]
    fn output_item_function_call_deserializes() {
        let json = r#"{
            "type": "function_call",
            "id": "fc_1",
            "call_id": "call_abc",
            "name": "get_weather",
            "arguments": "{\"city\":\"SF\"}"
        }"#;
        let item: OutputItem = serde_json::from_str(json).unwrap();
        match item {
            OutputItem::FunctionCall {
                id,
                call_id,
                name,
                arguments,
            } => {
                assert_eq!(id, "fc_1");
                assert_eq!(call_id, "call_abc");
                assert_eq!(name, "get_weather");
                assert!(arguments.contains("SF"));
            }
            other => panic!("expected FunctionCall, got: {other:?}"),
        }
    }

    #[test]
    fn output_item_function_call_output_deserializes() {
        let json = r#"{
            "type": "function_call_output",
            "call_id": "call_abc",
            "output": "{\"temp\":72}"
        }"#;
        let item: OutputItem = serde_json::from_str(json).unwrap();
        match item {
            OutputItem::FunctionCallOutput { call_id, output } => {
                assert_eq!(call_id, "call_abc");
                assert!(output.contains("72"));
            }
            other => panic!("expected FunctionCallOutput, got: {other:?}"),
        }
    }

    #[test]
    fn output_item_web_search_call_deserializes() {
        let json = r#"{
            "type": "web_search_call",
            "id": "ws_1",
            "status": "completed",
            "search_results": [
                {"title": "Rust programming", "url": "https://rust-lang.org"}
            ]
        }"#;
        let item: OutputItem = serde_json::from_str(json).unwrap();
        match item {
            OutputItem::WebSearchCall {
                id,
                status,
                search_results,
            } => {
                assert_eq!(id, "ws_1");
                assert_eq!(status.as_deref(), Some("completed"));
                let results = search_results.unwrap();
                assert_eq!(results.len(), 1);
                assert_eq!(results[0]["title"], "Rust programming");
            }
            other => panic!("expected WebSearchCall, got: {other:?}"),
        }
    }

    #[test]
    fn output_item_x_search_call_deserializes() {
        let json = r#"{
            "type": "x_search_call",
            "id": "xs_1",
            "status": "completed",
            "search_results": []
        }"#;
        let item: OutputItem = serde_json::from_str(json).unwrap();
        match item {
            OutputItem::XSearchCall {
                id,
                status,
                search_results,
            } => {
                assert_eq!(id, "xs_1");
                assert_eq!(status.as_deref(), Some("completed"));
                assert!(search_results.unwrap().is_empty());
            }
            other => panic!("expected XSearchCall, got: {other:?}"),
        }
    }

    #[test]
    fn output_item_code_interpreter_call_deserializes() {
        let json = r#"{
            "type": "code_interpreter_call",
            "id": "ci_1",
            "status": "completed",
            "code": "print(2 + 2)",
            "outputs": [{"type": "text", "text": "4"}]
        }"#;
        let item: OutputItem = serde_json::from_str(json).unwrap();
        match item {
            OutputItem::CodeInterpreterCall {
                id,
                status,
                code,
                outputs,
            } => {
                assert_eq!(id, "ci_1");
                assert_eq!(status.as_deref(), Some("completed"));
                assert_eq!(code.as_deref(), Some("print(2 + 2)"));
                let outs = outputs.unwrap();
                assert_eq!(outs.len(), 1);
                assert_eq!(outs[0]["text"], "4");
            }
            other => panic!("expected CodeInterpreterCall, got: {other:?}"),
        }
    }

    #[test]
    fn output_item_file_search_call_deserializes() {
        let json = r#"{
            "type": "file_search_call",
            "id": "fs_1",
            "status": "completed",
            "results": [{"file": "data.csv", "score": 0.95}]
        }"#;
        let item: OutputItem = serde_json::from_str(json).unwrap();
        match item {
            OutputItem::FileSearchCall {
                id,
                status,
                results,
            } => {
                assert_eq!(id, "fs_1");
                assert_eq!(status.as_deref(), Some("completed"));
                assert_eq!(results.unwrap().len(), 1);
            }
            other => panic!("expected FileSearchCall, got: {other:?}"),
        }
    }

    #[test]
    fn output_item_mcp_call_deserializes() {
        let json = r#"{
            "type": "mcp_call",
            "id": "mcp_1",
            "status": "completed"
        }"#;
        let item: OutputItem = serde_json::from_str(json).unwrap();
        match item {
            OutputItem::McpCall { id, status } => {
                assert_eq!(id, "mcp_1");
                assert_eq!(status.as_deref(), Some("completed"));
            }
            other => panic!("expected McpCall, got: {other:?}"),
        }
    }

    #[test]
    fn output_item_unknown_type_falls_through() {
        let json = r#"{
            "type": "some_future_tool_call",
            "id": "ft_1",
            "data": "whatever"
        }"#;
        let item: OutputItem = serde_json::from_str(json).unwrap();
        assert!(
            matches!(item, OutputItem::Unknown),
            "unrecognised output item type should deserialize as Unknown"
        );
    }

    #[test]
    fn response_object_with_web_search_and_code_interpreter() {
        let json = r#"{
            "id": "resp_tools",
            "status": "completed",
            "output": [
                {
                    "type": "web_search_call",
                    "id": "ws_1",
                    "status": "completed",
                    "search_results": [{"title": "Test", "url": "https://example.com"}]
                },
                {
                    "type": "code_interpreter_call",
                    "id": "ci_1",
                    "status": "completed",
                    "code": "1+1",
                    "outputs": [{"type": "text", "text": "2"}]
                },
                {
                    "type": "message",
                    "role": "assistant",
                    "content": [{"type": "text", "text": "Done."}]
                }
            ]
        }"#;
        let resp: ResponseObject = serde_json::from_str(json).unwrap();
        assert_eq!(resp.output.len(), 3);
        assert!(matches!(&resp.output[0], OutputItem::WebSearchCall { .. }));
        assert!(matches!(
            &resp.output[1],
            OutputItem::CodeInterpreterCall { .. }
        ));
        assert!(matches!(&resp.output[2], OutputItem::Message { .. }));
    }

    #[test]
    fn output_item_message_round_trips() {
        let item = OutputItem::Message {
            role: Role::Assistant,
            content: vec![ContentBlock::Text {
                text: "Hello!".into(),
            }],
        };
        let json = serde_json::to_string(&item).unwrap();
        let back: OutputItem = serde_json::from_str(&json).unwrap();
        assert_eq!(item, back);
    }

    #[test]
    fn output_item_function_call_round_trips() {
        let item = OutputItem::FunctionCall {
            id: "fc_1".into(),
            call_id: "call_1".into(),
            name: "search".into(),
            arguments: "{}".into(),
        };
        let json = serde_json::to_string(&item).unwrap();
        let back: OutputItem = serde_json::from_str(&json).unwrap();
        assert_eq!(item, back);
    }

    // -- ResponseStatus --

    #[test]
    fn response_status_round_trips() {
        for status in [
            ResponseStatus::Completed,
            ResponseStatus::InProgress,
            ResponseStatus::Incomplete,
            ResponseStatus::Failed,
        ] {
            let json = serde_json::to_string(&status).unwrap();
            let back: ResponseStatus = serde_json::from_str(&json).unwrap();
            assert_eq!(status, back);
        }
    }

    #[test]
    fn response_status_serializes_lowercase() {
        assert_eq!(
            serde_json::to_string(&ResponseStatus::Completed).unwrap(),
            "\"completed\""
        );
        assert_eq!(
            serde_json::to_string(&ResponseStatus::Incomplete).unwrap(),
            "\"incomplete\""
        );
        assert_eq!(
            serde_json::to_string(&ResponseStatus::Failed).unwrap(),
            "\"failed\""
        );
    }

    #[test]
    fn response_status_in_progress_serializes_to_snake_case() {
        assert_eq!(
            serde_json::to_string(&ResponseStatus::InProgress).unwrap(),
            "\"in_progress\""
        );
    }

    #[test]
    fn response_status_in_progress_deserializes_from_snake_case() {
        let status: ResponseStatus = serde_json::from_str("\"in_progress\"").unwrap();
        assert_eq!(status, ResponseStatus::InProgress);
    }

    #[test]
    fn response_status_in_progress_round_trips() {
        let json = serde_json::to_string(&ResponseStatus::InProgress).unwrap();
        let back: ResponseStatus = serde_json::from_str(&json).unwrap();
        assert_eq!(back, ResponseStatus::InProgress);
    }

    #[test]
    fn response_object_with_in_progress_status_deserializes() {
        let json = r#"{
            "id": "resp_in_prog",
            "status": "in_progress",
            "output": []
        }"#;
        let resp: ResponseObject = serde_json::from_str(json).unwrap();
        assert_eq!(resp.id, "resp_in_prog");
        assert_eq!(resp.status, ResponseStatus::InProgress);
        assert!(resp.output.is_empty());
    }

    // -- TextFormat --

    #[test]
    fn text_format_text_serializes() {
        let fmt = TextFormat::Text;
        let json = serde_json::to_string(&fmt).unwrap();
        assert_eq!(json, r#"{"type":"text"}"#);
    }

    #[test]
    fn text_format_json_schema_serializes() {
        let fmt = TextFormat::JsonSchema {
            name: "weather".into(),
            strict: Some(true),
            schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "temp": { "type": "number" }
                },
                "required": ["temp"]
            }),
        };
        let json = serde_json::to_string(&fmt).unwrap();
        assert!(json.contains("\"type\":\"json_schema\""));
        assert!(json.contains("\"name\":\"weather\""));
        assert!(json.contains("\"strict\":true"));
        assert!(json.contains("\"schema\""));
    }

    #[test]
    fn text_format_json_schema_round_trips() {
        let fmt = TextFormat::JsonSchema {
            name: "response".into(),
            strict: None,
            schema: serde_json::json!({"type": "object"}),
        };
        let json = serde_json::to_string(&fmt).unwrap();
        let back: TextFormat = serde_json::from_str(&json).unwrap();
        assert_eq!(fmt, back);
    }

    #[test]
    fn text_format_json_schema_skips_none_strict() {
        let fmt = TextFormat::JsonSchema {
            name: "test".into(),
            strict: None,
            schema: serde_json::json!({}),
        };
        let json = serde_json::to_string(&fmt).unwrap();
        assert!(!json.contains("\"strict\""));
    }

    // -- ResponseObject --

    #[test]
    fn response_object_deserializes_with_unknown_fields() {
        let json = r#"{
            "id": "resp_123",
            "object": "response",
            "status": "completed",
            "output": [],
            "some_future_field": "surprise",
            "another_unknown": 42
        }"#;
        let resp: ResponseObject = serde_json::from_str(json).unwrap();
        assert_eq!(resp.id, "resp_123");
        assert_eq!(resp.status, ResponseStatus::Completed);
        assert!(resp.output.is_empty());
    }

    #[test]
    fn response_object_full_round_trip() {
        let resp = ResponseObject {
            id: "resp_456".into(),
            object: Some("response".into()),
            status: ResponseStatus::Completed,
            output: vec![OutputItem::Message {
                role: Role::Assistant,
                content: vec![ContentBlock::Text {
                    text: "Hello!".into(),
                }],
            }],
            usage: Some(Usage {
                input_tokens: 10,
                output_tokens: 20,
                reasoning_tokens: None,
                total_tokens: Some(30),
                prompt_tokens_details: None,
                completion_tokens_details: None,
                output_tokens_details: None,
                cost_in_usd_ticks: None,
                cost_in_nano_usd: None,
                num_sources_used: None,
                server_side_tool_usage_details: None,
            }),
            model: Some("grok-4".into()),
            instructions: Some("Be helpful.".into()),
            metadata: None,
            previous_response_id: None,
            temperature: Some(0.7),
            top_p: None,
            max_output_tokens: Some(4096),
        };
        let json = serde_json::to_string(&resp).unwrap();
        let back: ResponseObject = serde_json::from_str(&json).unwrap();
        assert_eq!(resp.id, back.id);
        assert_eq!(resp.status, back.status);
        assert_eq!(resp.output.len(), back.output.len());
        assert_eq!(resp.usage, back.usage);
    }

    #[test]
    fn response_object_with_function_call_output() {
        let json = r#"{
            "id": "resp_789",
            "status": "completed",
            "output": [
                {
                    "type": "function_call",
                    "id": "fc_1",
                    "call_id": "call_abc",
                    "name": "get_weather",
                    "arguments": "{\"city\":\"NYC\"}"
                },
                {
                    "type": "function_call_output",
                    "call_id": "call_abc",
                    "output": "{\"temp\":65}"
                },
                {
                    "type": "message",
                    "role": "assistant",
                    "content": [{"type":"text","text":"It's 65F in NYC."}]
                }
            ]
        }"#;
        let resp: ResponseObject = serde_json::from_str(json).unwrap();
        assert_eq!(resp.output.len(), 3);
        match &resp.output[0] {
            OutputItem::FunctionCall { name, .. } => assert_eq!(name, "get_weather"),
            other => panic!("expected FunctionCall, got: {other:?}"),
        }
        match &resp.output[1] {
            OutputItem::FunctionCallOutput { call_id, output } => {
                assert_eq!(call_id, "call_abc");
                assert!(output.contains("65"));
            }
            other => panic!("expected FunctionCallOutput, got: {other:?}"),
        }
    }

    #[test]
    fn response_object_with_reasoning_output() {
        let json = r#"{
            "id": "resp_reason",
            "status": "completed",
            "output": [
                {
                    "type": "reasoning",
                    "content": [{"type":"thinking","text":"Step 1: analyze..."}]
                },
                {
                    "type": "message",
                    "role": "assistant",
                    "content": [{"type":"text","text":"The answer is 42."}]
                }
            ]
        }"#;
        let resp: ResponseObject = serde_json::from_str(json).unwrap();
        assert_eq!(resp.output.len(), 2);
        match &resp.output[0] {
            OutputItem::Reasoning {
                id,
                content,
                encrypted_content,
            } => {
                assert!(id.is_none());
                assert_eq!(content[0].text, "Step 1: analyze...");
                assert!(encrypted_content.is_none());
            }
            other => panic!("expected Reasoning, got: {other:?}"),
        }
    }

    #[test]
    fn response_object_with_usage() {
        let json = r#"{
            "id": "resp_usage",
            "status": "completed",
            "output": [],
            "usage": {
                "input_tokens": 100,
                "output_tokens": 50,
                "total_tokens": 150
            }
        }"#;
        let resp: ResponseObject = serde_json::from_str(json).unwrap();
        let usage = resp.usage.unwrap();
        assert_eq!(usage.input_tokens, 100);
        assert_eq!(usage.output_tokens, 50);
        assert_eq!(usage.total_tokens, Some(150));
    }

    #[test]
    fn previous_response_id_chains_requests() {
        let req = CreateResponseBuilder::new("grok-4", ResponseInput::Text("follow-up".into()))
            .previous_response_id("resp_abc")
            .build();
        let json = serde_json::to_string(&req).unwrap();
        assert!(json.contains("\"previous_response_id\":\"resp_abc\""));
    }

    // -- Structured output --

    #[test]
    fn structured_output_json_schema_request() {
        let req = CreateResponseBuilder::new(
            "grok-4",
            ResponseInput::Text("Give me weather data".into()),
        )
        .text(TextConfig {
            format: Some(TextFormat::JsonSchema {
                name: "weather_data".into(),
                strict: Some(true),
                schema: serde_json::json!({
                    "type": "object",
                    "properties": {
                        "temperature": { "type": "number" },
                        "unit": { "type": "string", "enum": ["celsius", "fahrenheit"] }
                    },
                    "required": ["temperature", "unit"],
                    "additionalProperties": false
                }),
            }),
        })
        .build();

        let json = serde_json::to_string(&req).unwrap();
        assert!(json.contains("\"json_schema\""));
        assert!(json.contains("\"weather_data\""));
        assert!(json.contains("\"strict\":true"));

        // Round-trip
        let back: CreateResponseRequest = serde_json::from_str(&json).unwrap();
        assert!(back.text.is_some());
        let text_config = back.text.unwrap();
        match text_config.format.unwrap() {
            TextFormat::JsonSchema {
                name,
                strict,
                schema: _,
            } => {
                assert_eq!(name, "weather_data");
                assert_eq!(strict, Some(true));
            }
            other => panic!("expected JsonSchema, got: {other:?}"),
        }
    }

    // -- Optional field skipping --

    #[test]
    fn create_response_request_skips_none_fields() {
        let req = CreateResponseBuilder::new("grok-4", ResponseInput::Text("test".into())).build();
        let json = serde_json::to_string(&req).unwrap();
        assert!(!json.contains("\"instructions\""));
        assert!(!json.contains("\"tools\""));
        assert!(!json.contains("\"tool_choice\""));
        assert!(!json.contains("\"previous_response_id\""));
        assert!(!json.contains("\"stream\""));
        assert!(!json.contains("\"temperature\""));
        assert!(!json.contains("\"top_p\""));
        assert!(!json.contains("\"max_output_tokens\""));
        assert!(!json.contains("\"max_turns\""));
        assert!(!json.contains("\"reasoning\""));
        assert!(!json.contains("\"text\""));
        assert!(!json.contains("\"search_parameters\""));
        assert!(!json.contains("\"metadata\""));
        // store IS present because it defaults to Some(false)
        assert!(json.contains("\"store\":false"));
    }

    // -- ReasoningConfig --

    #[test]
    fn reasoning_config_round_trips() {
        let cfg = ReasoningConfig {
            effort: Some("high".into()),
            generate_summary: None,
            summary: None,
        };
        let json = serde_json::to_string(&cfg).unwrap();
        let back: ReasoningConfig = serde_json::from_str(&json).unwrap();
        assert_eq!(cfg, back);
    }

    // -- Builder with all fields --

    #[test]
    fn builder_all_fields() {
        let req = CreateResponseBuilder::new("grok-4-mini", ResponseInput::Text("test".into()))
            .model("grok-4")
            .input_text("updated input")
            .instructions("Be helpful")
            .tools(vec![
                serde_json::json!({"type":"function","name":"f","parameters":{}}),
            ])
            .tool_choice(serde_json::json!("auto"))
            .previous_response_id("resp_prev")
            .store(true)
            .stream(true)
            .temperature(0.5)
            .top_p(0.9)
            .max_output_tokens(1024)
            .max_turns(3)
            .reasoning(ReasoningConfig {
                effort: Some("medium".into()),
                generate_summary: None,
                summary: None,
            })
            .text(TextConfig {
                format: Some(TextFormat::Text),
            })
            .search_parameters(serde_json::json!({"max_results": 5}))
            .metadata(serde_json::json!({"session_id": "s_1"}))
            .build();

        assert_eq!(req.model, "grok-4");
        assert_eq!(req.input, ResponseInput::Text("updated input".into()));
        assert_eq!(req.instructions, Some("Be helpful".into()));
        assert!(req.tools.is_some());
        assert!(req.tool_choice.is_some());
        assert_eq!(req.previous_response_id, Some("resp_prev".into()));
        assert_eq!(req.store, Some(true));
        assert_eq!(req.stream, Some(true));
        assert_eq!(req.temperature, Some(0.5));
        assert_eq!(req.top_p, Some(0.9));
        assert_eq!(req.max_output_tokens, Some(1024));
        assert_eq!(req.max_turns, Some(3));
        assert!(req.reasoning.is_some());
        assert!(req.text.is_some());
        assert!(req.search_parameters.is_some());
        assert!(req.metadata.is_some());
    }

    #[test]
    fn builder_input_messages() {
        let msgs = vec![
            InputMessage::text(Role::System, "You are helpful"),
            InputMessage::text(Role::User, "Hi"),
        ];
        let req = CreateResponseBuilder::new("grok-4", ResponseInput::Text("placeholder".into()))
            .input_messages(msgs.clone())
            .build();

        match &req.input {
            ResponseInput::Items(m) => assert_eq!(m.len(), 2),
            _ => panic!("expected Messages variant"),
        }
    }

    // -- ReasoningSummary --

    #[test]
    fn reasoning_summary_auto_round_trips() {
        let json = serde_json::to_string(&ReasoningSummary::Auto).unwrap();
        assert_eq!(json, "\"auto\"");
        let back: ReasoningSummary = serde_json::from_str(&json).unwrap();
        assert_eq!(back, ReasoningSummary::Auto);
    }

    #[test]
    fn reasoning_summary_concise_round_trips() {
        let json = serde_json::to_string(&ReasoningSummary::Concise).unwrap();
        assert_eq!(json, "\"concise\"");
        let back: ReasoningSummary = serde_json::from_str(&json).unwrap();
        assert_eq!(back, ReasoningSummary::Concise);
    }

    #[test]
    fn reasoning_summary_detailed_round_trips() {
        let json = serde_json::to_string(&ReasoningSummary::Detailed).unwrap();
        assert_eq!(json, "\"detailed\"");
        let back: ReasoningSummary = serde_json::from_str(&json).unwrap();
        assert_eq!(back, ReasoningSummary::Detailed);
    }

    #[test]
    fn reasoning_config_full_round_trips() {
        let cfg = ReasoningConfig {
            effort: Some("high".into()),
            generate_summary: Some(true),
            summary: Some(ReasoningSummary::Detailed),
        };
        let json = serde_json::to_string(&cfg).unwrap();
        assert!(json.contains("\"generate_summary\":true"));
        assert!(json.contains("\"summary\":\"detailed\""));
        let back: ReasoningConfig = serde_json::from_str(&json).unwrap();
        assert_eq!(cfg, back);
    }

    #[test]
    fn reasoning_config_skips_none_new_fields() {
        let cfg = ReasoningConfig {
            effort: Some("low".into()),
            generate_summary: None,
            summary: None,
        };
        let json = serde_json::to_string(&cfg).unwrap();
        assert!(!json.contains("\"generate_summary\""));
        assert!(!json.contains("\"summary\""));
    }

    // -- include, context_management, prompt_cache_key --

    #[test]
    fn create_response_request_with_include_round_trips() {
        let req = CreateResponseBuilder::new("grok-4", ResponseInput::Text("test".into()))
            .include(vec!["reasoning".into(), "usage".into()])
            .build();
        let json = serde_json::to_string(&req).unwrap();
        assert!(json.contains("\"include\""));
        assert!(json.contains("\"reasoning\""));
        let back: CreateResponseRequest = serde_json::from_str(&json).unwrap();
        assert_eq!(back.include, Some(vec!["reasoning".into(), "usage".into()]));
    }

    #[test]
    fn create_response_request_with_context_management_round_trips() {
        let directives = vec![serde_json::json!({"type": "truncation", "max_tokens": 1000})];
        let req = CreateResponseBuilder::new("grok-4", ResponseInput::Text("test".into()))
            .context_management(directives.clone())
            .build();
        let json = serde_json::to_string(&req).unwrap();
        assert!(json.contains("\"context_management\""));
        let back: CreateResponseRequest = serde_json::from_str(&json).unwrap();
        assert_eq!(back.context_management, Some(directives));
    }

    #[test]
    fn create_response_request_with_prompt_cache_key_round_trips() {
        let req = CreateResponseBuilder::new("grok-4", ResponseInput::Text("test".into()))
            .prompt_cache_key("my-cache-key-123")
            .build();
        let json = serde_json::to_string(&req).unwrap();
        assert!(json.contains("\"prompt_cache_key\":\"my-cache-key-123\""));
        let back: CreateResponseRequest = serde_json::from_str(&json).unwrap();
        assert_eq!(back.prompt_cache_key, Some("my-cache-key-123".into()));
    }

    #[test]
    fn create_response_request_skips_none_new_fields() {
        let req = CreateResponseBuilder::new("grok-4", ResponseInput::Text("test".into())).build();
        let json = serde_json::to_string(&req).unwrap();
        assert!(!json.contains("\"include\""));
        assert!(!json.contains("\"context_management\""));
        assert!(!json.contains("\"prompt_cache_key\""));
    }

    #[test]
    fn create_response_request_without_new_fields_deserializes() {
        let json = r#"{"model":"grok-4","input":"hello"}"#;
        let req: CreateResponseRequest = serde_json::from_str(json).unwrap();
        assert!(req.include.is_none());
        assert!(req.context_management.is_none());
        assert!(req.prompt_cache_key.is_none());
    }

    // -- Encrypted reasoning content passthrough --

    #[test]
    fn output_item_reasoning_with_encrypted_content_deserializes() {
        let json = r#"{
            "type": "reasoning",
            "id": "rs_abc123",
            "content": [{"type":"thinking","text":"Analyzing..."}],
            "encrypted_content": "ENC:opaque-blob-data-here"
        }"#;
        let item: OutputItem = serde_json::from_str(json).unwrap();
        match item {
            OutputItem::Reasoning {
                id,
                content,
                encrypted_content,
            } => {
                assert_eq!(id, Some("rs_abc123".into()));
                assert_eq!(content.len(), 1);
                assert_eq!(content[0].text, "Analyzing...");
                assert_eq!(
                    encrypted_content.as_deref(),
                    Some("ENC:opaque-blob-data-here")
                );
            }
            other => panic!("expected Reasoning, got: {other:?}"),
        }
    }

    #[test]
    fn output_item_reasoning_with_encrypted_content_round_trips() {
        let item = OutputItem::Reasoning {
            id: Some("rs_round".into()),
            content: vec![ReasoningBlock {
                r#type: "thinking".into(),
                text: "Step 1".into(),
            }],
            encrypted_content: Some("ENC:round-trip-blob".into()),
        };
        let json = serde_json::to_string(&item).unwrap();
        assert!(json.contains("\"encrypted_content\":\"ENC:round-trip-blob\""));
        let back: OutputItem = serde_json::from_str(&json).unwrap();
        assert_eq!(item, back);
    }

    #[test]
    fn output_item_reasoning_without_encrypted_content_skips_field() {
        let item = OutputItem::Reasoning {
            id: Some("rs_no_enc".into()),
            content: vec![ReasoningBlock {
                r#type: "thinking".into(),
                text: "No encryption".into(),
            }],
            encrypted_content: None,
        };
        let json = serde_json::to_string(&item).unwrap();
        assert!(
            !json.contains("encrypted_content"),
            "encrypted_content: None should be skipped in serialization"
        );
    }

    #[test]
    fn encrypted_content_is_opaque_no_parsing() {
        // Encrypted content can contain arbitrary data — base64, binary-safe
        // strings, JSON fragments, etc. We must preserve it exactly.
        let large_blob = "a".repeat(100_000);
        let blobs = [
            "dGhpcyBpcyBiYXNlNjQ=",
            r#"{"nested":"json","key":123}"#,
            "binary\x00data\x01with\x02control\x03chars",
            "",
            large_blob.as_str(), // large blob
        ];
        for blob in blobs {
            let item = OutputItem::Reasoning {
                id: None,
                content: vec![],
                encrypted_content: Some(blob.to_string()),
            };
            let json = serde_json::to_string(&item).unwrap();
            let back: OutputItem = serde_json::from_str(&json).unwrap();
            match back {
                OutputItem::Reasoning {
                    encrypted_content, ..
                } => {
                    assert_eq!(encrypted_content.as_deref(), Some(blob));
                }
                _ => panic!("expected Reasoning"),
            }
        }
    }

    #[test]
    fn non_reasoning_model_returns_no_encrypted_content() {
        // When the model is not a reasoning model, or `include` did not
        // request encrypted content, the field is simply absent.
        let json = r#"{
            "type": "reasoning",
            "content": [{"type":"thinking","text":"basic thought"}]
        }"#;
        let item: OutputItem = serde_json::from_str(json).unwrap();
        match item {
            OutputItem::Reasoning {
                encrypted_content, ..
            } => {
                assert!(
                    encrypted_content.is_none(),
                    "non-reasoning models should not have encrypted_content"
                );
            }
            _ => panic!("expected Reasoning"),
        }
    }

    // -- InputItem --

    #[test]
    fn input_item_message_round_trips() {
        let item = InputItem::Message(InputMessage::text(Role::User, "Hello"));
        let json = serde_json::to_string(&item).unwrap();
        assert!(json.contains("\"role\":\"user\""));
        assert!(json.contains("\"content\":\"Hello\""));
        let back: InputItem = serde_json::from_str(&json).unwrap();
        assert_eq!(item, back);
    }

    #[test]
    fn input_item_reasoning_serializes_with_type_tag() {
        let item = InputItem::Reasoning {
            r#type: "reasoning".into(),
            id: Some("rs_replay".into()),
            encrypted_content: "ENC:replay-blob".into(),
        };
        let json = serde_json::to_string(&item).unwrap();
        assert!(json.contains("\"type\":\"reasoning\""));
        assert!(json.contains("\"id\":\"rs_replay\""));
        assert!(json.contains("\"encrypted_content\":\"ENC:replay-blob\""));
    }

    #[test]
    fn input_item_reasoning_round_trips() {
        let item = InputItem::Reasoning {
            r#type: "reasoning".into(),
            id: Some("rs_rt".into()),
            encrypted_content: "ENC:data".into(),
        };
        let json = serde_json::to_string(&item).unwrap();
        let back: InputItem = serde_json::from_str(&json).unwrap();
        assert_eq!(item, back);
    }

    #[test]
    fn input_item_reasoning_without_id_round_trips() {
        let item = InputItem::Reasoning {
            r#type: "reasoning".into(),
            id: None,
            encrypted_content: "ENC:no-id-blob".into(),
        };
        let json = serde_json::to_string(&item).unwrap();
        assert!(!json.contains("\"id\""));
        let back: InputItem = serde_json::from_str(&json).unwrap();
        assert_eq!(item, back);
    }

    // -- Full round-trip: request with include -> response with encrypted_content -> replay --

    #[test]
    fn encrypted_reasoning_full_round_trip() {
        // Step 1: Build a request with include: ["reasoning.encrypted_content"]
        let req = CreateResponseBuilder::new(
            "grok-4",
            ResponseInput::Text("Solve this step by step".into()),
        )
        .include(vec!["reasoning.encrypted_content".into()])
        .store(true)
        .build();

        let req_json = serde_json::to_string(&req).unwrap();
        assert!(req_json.contains("\"reasoning.encrypted_content\""));

        // Step 2: Simulate a response with encrypted reasoning
        let response_json = r#"{
            "id": "resp_001",
            "status": "completed",
            "output": [
                {
                    "type": "reasoning",
                    "id": "rs_001",
                    "content": [{"type":"thinking","text":"Let me analyze..."}],
                    "encrypted_content": "ENC:aGVsbG8gd29ybGQ="
                },
                {
                    "type": "message",
                    "role": "assistant",
                    "content": [{"type":"text","text":"The answer is 42."}]
                }
            ]
        }"#;
        let resp: ResponseObject = serde_json::from_str(response_json).unwrap();
        assert_eq!(resp.output.len(), 2);

        // Extract encrypted content from response
        let encrypted = match &resp.output[0] {
            OutputItem::Reasoning {
                id,
                encrypted_content,
                ..
            } => {
                assert_eq!(id.as_deref(), Some("rs_001"));
                encrypted_content
                    .as_ref()
                    .expect("encrypted_content should be present")
                    .clone()
            }
            other => panic!("expected Reasoning, got: {other:?}"),
        };

        // Step 3: Build a follow-up request replaying the encrypted reasoning
        let follow_up = CreateResponseBuilder::new(
            "grok-4",
            ResponseInput::Items(vec![
                InputItem::Reasoning {
                    r#type: "reasoning".into(),
                    id: Some("rs_001".into()),
                    encrypted_content: encrypted.clone(),
                },
                InputItem::Message(InputMessage::text(Role::User, "Now explain why.")),
            ]),
        )
        .previous_response_id("resp_001")
        .include(vec!["reasoning.encrypted_content".into()])
        .store(true)
        .build();

        let follow_up_json = serde_json::to_string(&follow_up).unwrap();
        assert!(follow_up_json.contains("\"type\":\"reasoning\""));
        assert!(follow_up_json.contains("\"encrypted_content\":\"ENC:aGVsbG8gd29ybGQ=\""));
        assert!(follow_up_json.contains("\"previous_response_id\":\"resp_001\""));
        assert!(follow_up_json.contains("\"role\":\"user\""));

        // Verify the follow-up request round-trips through JSON
        let back: CreateResponseRequest = serde_json::from_str(&follow_up_json).unwrap();
        match &back.input {
            ResponseInput::Items(items) => {
                assert_eq!(items.len(), 2);
                match &items[0] {
                    InputItem::Reasoning {
                        encrypted_content, ..
                    } => {
                        assert_eq!(encrypted_content, "ENC:aGVsbG8gd29ybGQ=");
                    }
                    other => panic!("expected Reasoning, got: {other:?}"),
                }
                match &items[1] {
                    InputItem::Message(msg) => {
                        assert_eq!(msg.role, Role::User);
                    }
                    other => panic!("expected Message, got: {other:?}"),
                }
            }
            _ => panic!("expected Items variant"),
        }
    }

    #[test]
    fn response_with_multiple_reasoning_blocks_and_encrypted_content() {
        let json = r#"{
            "id": "resp_multi",
            "status": "completed",
            "output": [
                {
                    "type": "reasoning",
                    "id": "rs_a",
                    "content": [{"type":"thinking","text":"Part A"}],
                    "encrypted_content": "ENC:part_a_blob"
                },
                {
                    "type": "reasoning",
                    "id": "rs_b",
                    "content": [{"type":"thinking","text":"Part B"}],
                    "encrypted_content": "ENC:part_b_blob"
                },
                {
                    "type": "message",
                    "role": "assistant",
                    "content": [{"type":"text","text":"Done."}]
                }
            ]
        }"#;
        let resp: ResponseObject = serde_json::from_str(json).unwrap();
        assert_eq!(resp.output.len(), 3);

        // Both reasoning blocks should have encrypted content
        let mut encrypted: Vec<(Option<String>, String)> = Vec::new();
        for item in &resp.output {
            if let OutputItem::Reasoning {
                id,
                encrypted_content: Some(enc),
                ..
            } = item
            {
                encrypted.push((id.clone(), enc.clone()));
            }
        }
        assert_eq!(encrypted.len(), 2);
        assert_eq!(encrypted[0].0.as_deref(), Some("rs_a"));
        assert_eq!(encrypted[0].1, "ENC:part_a_blob");
        assert_eq!(encrypted[1].0.as_deref(), Some("rs_b"));
        assert_eq!(encrypted[1].1, "ENC:part_b_blob");
    }

    #[test]
    fn builder_input_items_with_mixed_types() {
        let items = vec![
            InputItem::Reasoning {
                r#type: "reasoning".into(),
                id: Some("rs_1".into()),
                encrypted_content: "ENC:blob".into(),
            },
            InputItem::Message(InputMessage::text(Role::User, "Continue")),
        ];
        let req = CreateResponseBuilder::new("grok-4", ResponseInput::Text("placeholder".into()))
            .input_items(items)
            .build();

        match &req.input {
            ResponseInput::Items(items) => {
                assert_eq!(items.len(), 2);
                assert!(matches!(&items[0], InputItem::Reasoning { .. }));
                assert!(matches!(&items[1], InputItem::Message(_)));
            }
            _ => panic!("expected Items variant"),
        }
    }

    #[test]
    fn include_reasoning_encrypted_content_in_request() {
        let req = CreateResponseBuilder::new("grok-4", ResponseInput::Text("test".into()))
            .include(vec!["reasoning.encrypted_content".into()])
            .build();
        let json = serde_json::to_string(&req).unwrap();
        assert!(json.contains("\"include\":[\"reasoning.encrypted_content\"]"));
    }

    #[test]
    fn encrypted_content_survives_transcript_json_round_trip() {
        // Simulate what happens when the full response body is stored as JSON
        // in the transcript table and later deserialized.
        let response = ResponseObject {
            id: "resp_transcript".into(),
            object: Some("response".into()),
            status: ResponseStatus::Completed,
            output: vec![
                OutputItem::Reasoning {
                    id: Some("rs_t1".into()),
                    content: vec![ReasoningBlock {
                        r#type: "thinking".into(),
                        text: "Deep thought".into(),
                    }],
                    encrypted_content: Some("ENC:transcript-blob-12345".into()),
                },
                OutputItem::Message {
                    role: Role::Assistant,
                    content: vec![ContentBlock::Text {
                        text: "Result".into(),
                    }],
                },
            ],
            usage: None,
            model: Some("grok-4".into()),
            instructions: None,
            metadata: None,
            previous_response_id: None,
            temperature: None,
            top_p: None,
            max_output_tokens: None,
        };

        // Store as JSON (what the transcript table does)
        let stored_json = serde_json::to_string(&response).unwrap();

        // Retrieve and deserialize (what transcript reading does)
        let restored: ResponseObject = serde_json::from_str(&stored_json).unwrap();
        match &restored.output[0] {
            OutputItem::Reasoning {
                id,
                encrypted_content,
                ..
            } => {
                assert_eq!(id.as_deref(), Some("rs_t1"));
                assert_eq!(
                    encrypted_content.as_deref(),
                    Some("ENC:transcript-blob-12345")
                );
            }
            other => panic!("expected Reasoning, got: {other:?}"),
        }
    }
}
