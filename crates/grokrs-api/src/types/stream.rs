use serde::{Deserialize, Serialize};

use crate::transport::error::TransportError;
use crate::types::usage::ChatUsage;

// ---------------------------------------------------------------------------
// FinishReason
// ---------------------------------------------------------------------------

/// Reason the model stopped generating tokens.
///
/// Defined here because `types/chat.rs` may not exist yet (parallel agent work).
/// Can be reconciled later during merge.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FinishReason {
    /// Natural stop (end-of-turn or stop sequence matched).
    Stop,
    /// Token budget exhausted.
    Length,
    /// Model decided it has finished its turn.
    EndTurn,
    /// Model wants to invoke one or more tools.
    ToolCalls,
}

// ---------------------------------------------------------------------------
// Chat Completions streaming types
// ---------------------------------------------------------------------------

/// A single streamed chunk from the Chat Completions API.
///
/// Each chunk corresponds to one `data:` line in the SSE stream.
/// When `stream_options.include_usage` is set, the final chunk carries
/// a populated `usage` field.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ChatStreamChunk {
    /// Unique identifier for this completion.
    pub id: String,

    /// Object type (e.g. `"chat.completion.chunk"`).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub object: Option<String>,

    /// Unix timestamp (seconds) when the completion was created.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub created: Option<i64>,

    /// Model that generated the completion.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,

    /// Choices in this chunk (typically one).
    pub choices: Vec<ChatStreamChoice>,

    /// Token usage, present only in the final chunk when
    /// `stream_options.include_usage` was requested.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub usage: Option<ChatUsage>,

    /// Server-side fingerprint for reproducibility.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub system_fingerprint: Option<String>,
}

/// A single choice within a streamed chat completion chunk.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ChatStreamChoice {
    /// Zero-based index of this choice.
    pub index: u32,

    /// The incremental content for this choice.
    pub delta: ChatDelta,

    /// Present in the final chunk for this choice; `None` while streaming.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub finish_reason: Option<FinishReason>,
}

/// Incremental content delta within a streamed choice.
///
/// All fields are optional because any given chunk may only carry a subset
/// (e.g. only `content`, or only `tool_calls`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ChatDelta {
    /// Role of the message author (typically only in the first chunk).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub role: Option<crate::types::common::Role>,

    /// Partial text content.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub content: Option<String>,

    /// Partial reasoning/chain-of-thought content.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reasoning_content: Option<String>,

    /// Incremental tool call deltas.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool_calls: Option<Vec<ToolCallDelta>>,
}

/// An incremental tool call within a streamed delta.
///
/// Tool call chunks arrive whole per the xAI API contract: each chunk
/// carries the full current state of the tool call at `index`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ToolCallDelta {
    /// Zero-based index identifying which tool call this delta belongs to.
    pub index: u32,

    /// Tool call identifier (present in the first chunk for this call).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<String>,

    /// Tool type (e.g. `"function"`).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub r#type: Option<String>,

    /// Function name and argument fragments.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub function: Option<FunctionCallDelta>,
}

/// Incremental function call data within a tool call delta.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct FunctionCallDelta {
    /// Function name (present in the first chunk for this call).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,

    /// Partial JSON arguments string.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub arguments: Option<String>,
}

// ---------------------------------------------------------------------------
// Responses API streaming types
// ---------------------------------------------------------------------------

/// A content delta payload within a Responses API stream event.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ContentDeltaPayload {
    /// Content type (e.g. `"text"`).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub r#type: Option<String>,

    /// Partial text content.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub text: Option<String>,
}

/// A typed event from the Responses API SSE stream.
///
/// Each variant corresponds to an event `type` on the wire.
/// We use `serde_json::Value` for response/item/part payloads because the
/// full Responses API object model is large and evolving; typed wrappers
/// can be added incrementally without breaking the stream parser.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum ResponseStreamEvent {
    /// The response object has been created.
    #[serde(rename = "response.created")]
    ResponseCreated {
        /// The full response object snapshot.
        response: serde_json::Value,
    },

    /// The response is being processed.
    #[serde(rename = "response.in_progress")]
    ResponseInProgress {
        /// The full response object snapshot.
        response: serde_json::Value,
    },

    /// The response has completed successfully.
    #[serde(rename = "response.completed")]
    ResponseCompleted {
        /// The full response object snapshot.
        response: serde_json::Value,
    },

    /// The response has failed.
    #[serde(rename = "response.failed")]
    ResponseFailed {
        /// The full response object snapshot.
        response: serde_json::Value,
    },

    /// A new output item has been added.
    #[serde(rename = "response.output_item.added")]
    OutputItemAdded {
        /// Zero-based index of the output item.
        output_index: u32,
        /// The output item object.
        item: serde_json::Value,
    },

    /// An output item is complete.
    #[serde(rename = "response.output_item.done")]
    OutputItemDone {
        /// Zero-based index of the output item.
        output_index: u32,
        /// The completed output item object.
        item: serde_json::Value,
    },

    /// A new content part has been added to an output item.
    #[serde(rename = "response.content_part.added")]
    ContentPartAdded {
        /// Zero-based index of the output item.
        output_index: u32,
        /// Zero-based index of the content part within the output item.
        content_index: u32,
        /// The content part object.
        part: serde_json::Value,
    },

    /// A content part is complete.
    #[serde(rename = "response.content_part.done")]
    ContentPartDone {
        /// Zero-based index of the output item.
        output_index: u32,
        /// Zero-based index of the content part within the output item.
        content_index: u32,
        /// The completed content part object.
        part: serde_json::Value,
    },

    /// An incremental content delta for a content part.
    #[serde(rename = "response.content_part.delta")]
    ContentDelta {
        /// Zero-based index of the output item.
        output_index: u32,
        /// Zero-based index of the content part within the output item.
        content_index: u32,
        /// The incremental content payload.
        delta: ContentDeltaPayload,
    },

    /// An incremental function call arguments delta.
    #[serde(rename = "response.function_call_arguments.delta")]
    FunctionCallArgumentsDelta {
        /// Zero-based index of the output item.
        output_index: u32,
        /// Identifier of the output item.
        #[serde(skip_serializing_if = "Option::is_none")]
        item_id: Option<String>,
        /// Identifier of the function call.
        #[serde(skip_serializing_if = "Option::is_none")]
        call_id: Option<String>,
        /// Partial arguments string.
        delta: String,
    },

    /// Function call arguments are complete.
    #[serde(rename = "response.function_call_arguments.done")]
    FunctionCallArgumentsDone {
        /// Zero-based index of the output item.
        output_index: u32,
        /// Identifier of the output item.
        #[serde(skip_serializing_if = "Option::is_none")]
        item_id: Option<String>,
        /// Identifier of the function call.
        #[serde(skip_serializing_if = "Option::is_none")]
        call_id: Option<String>,
        /// Complete arguments JSON string.
        arguments: String,
    },

    /// An incremental text delta for an output item's text content.
    ///
    /// xAI uses this event alongside (or instead of) `response.content_part.delta`
    /// for streaming text output.
    #[serde(rename = "response.output_text.delta")]
    OutputTextDelta {
        /// Zero-based index of the output item.
        output_index: u32,
        /// Zero-based index of the content part (may be absent for single-part outputs).
        #[serde(skip_serializing_if = "Option::is_none")]
        content_index: Option<u32>,
        /// The incremental text fragment.
        delta: String,
    },

    /// The output text for an output item is complete.
    #[serde(rename = "response.output_text.done")]
    OutputTextDone {
        /// Zero-based index of the output item.
        output_index: u32,
        /// Zero-based index of the content part (may be absent for single-part outputs).
        #[serde(skip_serializing_if = "Option::is_none")]
        content_index: Option<u32>,
        /// The complete text content.
        text: String,
    },

    // -----------------------------------------------------------------
    // MCP (Model Context Protocol) stream events
    // -----------------------------------------------------------------
    /// The server is listing tools available on an MCP server.
    #[serde(rename = "mcp_list_tools.in_progress")]
    McpListToolsInProgress {
        /// Label of the MCP server being queried.
        #[serde(skip_serializing_if = "Option::is_none")]
        server_label: Option<String>,
    },

    /// The server has successfully listed tools on an MCP server.
    #[serde(rename = "mcp_list_tools.completed")]
    McpListToolsCompleted {
        /// Label of the MCP server.
        #[serde(skip_serializing_if = "Option::is_none")]
        server_label: Option<String>,
        /// The tools that were discovered, as raw JSON.
        #[serde(skip_serializing_if = "Option::is_none")]
        tools: Option<Vec<serde_json::Value>>,
    },

    /// The server failed to list tools on an MCP server.
    #[serde(rename = "mcp_list_tools.failed")]
    McpListToolsFailed {
        /// Label of the MCP server.
        #[serde(skip_serializing_if = "Option::is_none")]
        server_label: Option<String>,
        /// Error information.
        #[serde(skip_serializing_if = "Option::is_none")]
        error: Option<serde_json::Value>,
    },

    /// Incremental arguments delta for an MCP call being constructed.
    #[serde(rename = "response.mcp_call_arguments.delta")]
    McpCallArgumentsDelta {
        /// Zero-based index of the output item.
        output_index: u32,
        /// Identifier of the output item.
        #[serde(skip_serializing_if = "Option::is_none")]
        item_id: Option<String>,
        /// Partial arguments string.
        delta: String,
    },

    /// MCP call arguments are complete.
    #[serde(rename = "response.mcp_call_arguments.done")]
    McpCallArgumentsDone {
        /// Zero-based index of the output item.
        output_index: u32,
        /// Identifier of the output item.
        #[serde(skip_serializing_if = "Option::is_none")]
        item_id: Option<String>,
        /// Complete arguments JSON string.
        arguments: String,
    },

    /// An MCP call is being executed server-side.
    #[serde(rename = "response.mcp_call.in_progress")]
    McpCallInProgress {
        /// Zero-based index of the output item.
        output_index: u32,
        /// The output item object snapshot.
        #[serde(skip_serializing_if = "Option::is_none")]
        item: Option<serde_json::Value>,
    },

    /// An MCP call has completed successfully.
    #[serde(rename = "response.mcp_call.completed")]
    McpCallCompleted {
        /// Zero-based index of the output item.
        output_index: u32,
        /// The completed output item object.
        #[serde(skip_serializing_if = "Option::is_none")]
        item: Option<serde_json::Value>,
    },

    /// An MCP call has failed.
    #[serde(rename = "response.mcp_call.failed")]
    McpCallFailed {
        /// Zero-based index of the output item.
        output_index: u32,
        /// The failed output item object (may contain error details).
        #[serde(skip_serializing_if = "Option::is_none")]
        item: Option<serde_json::Value>,
    },

    /// An unrecognised event type from the server.
    ///
    /// Provides forward compatibility — new event types added by xAI will
    /// deserialize into this variant instead of causing a parse error.
    #[serde(other)]
    Unknown,
}

// ---------------------------------------------------------------------------
// Top-level stream types
// ---------------------------------------------------------------------------

/// A top-level stream event that can be either a Chat Completions chunk
/// or a Responses API event.
#[derive(Debug, Clone, PartialEq)]
pub enum StreamEvent {
    /// A Chat Completions API streaming chunk (boxed to avoid large enum variant).
    Chat(Box<ChatStreamChunk>),
    /// A Responses API streaming event.
    Response(ResponseStreamEvent),
    /// The stream has terminated (received `[DONE]`).
    Done,
}

/// Errors that can occur while consuming a typed stream.
#[derive(Debug)]
pub enum StreamError {
    /// Failed to parse a stream event from JSON.
    Parse {
        /// Description of the parse failure.
        message: String,
    },
    /// An error from the underlying transport layer.
    Transport(TransportError),
    /// The connection was lost mid-stream.
    ConnectionLost,
}

impl std::fmt::Display for StreamError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            StreamError::Parse { message } => write!(f, "stream parse error: {message}"),
            StreamError::Transport(err) => write!(f, "stream transport error: {err}"),
            StreamError::ConnectionLost => write!(f, "stream connection lost"),
        }
    }
}

impl std::error::Error for StreamError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            StreamError::Transport(err) => Some(err),
            _ => None,
        }
    }
}

impl From<TransportError> for StreamError {
    fn from(err: TransportError) -> Self {
        StreamError::Transport(err)
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    // -- FinishReason --

    #[test]
    fn finish_reason_serializes_snake_case() {
        assert_eq!(
            serde_json::to_string(&FinishReason::Stop).unwrap(),
            "\"stop\""
        );
        assert_eq!(
            serde_json::to_string(&FinishReason::Length).unwrap(),
            "\"length\""
        );
        assert_eq!(
            serde_json::to_string(&FinishReason::EndTurn).unwrap(),
            "\"end_turn\""
        );
        assert_eq!(
            serde_json::to_string(&FinishReason::ToolCalls).unwrap(),
            "\"tool_calls\""
        );
    }

    #[test]
    fn finish_reason_round_trips() {
        for reason in [
            FinishReason::Stop,
            FinishReason::Length,
            FinishReason::EndTurn,
            FinishReason::ToolCalls,
        ] {
            let json = serde_json::to_string(&reason).unwrap();
            let back: FinishReason = serde_json::from_str(&json).unwrap();
            assert_eq!(reason, back);
        }
    }

    // -- ChatStreamChunk deserialization --

    #[test]
    fn chat_stream_chunk_deserializes_from_json() {
        let json = r#"{
            "id": "chatcmpl-abc123",
            "object": "chat.completion.chunk",
            "created": 1700000000,
            "model": "grok-4",
            "choices": [{
                "index": 0,
                "delta": {
                    "content": "Hello"
                },
                "finish_reason": null
            }],
            "system_fingerprint": "fp_abc"
        }"#;
        let chunk: ChatStreamChunk = serde_json::from_str(json).unwrap();
        assert_eq!(chunk.id, "chatcmpl-abc123");
        assert_eq!(chunk.object.as_deref(), Some("chat.completion.chunk"));
        assert_eq!(chunk.created, Some(1_700_000_000));
        assert_eq!(chunk.model.as_deref(), Some("grok-4"));
        assert_eq!(chunk.choices.len(), 1);
        assert_eq!(chunk.choices[0].index, 0);
        assert_eq!(chunk.choices[0].delta.content.as_deref(), Some("Hello"));
        assert!(chunk.choices[0].finish_reason.is_none());
        assert_eq!(chunk.system_fingerprint.as_deref(), Some("fp_abc"));
        assert!(chunk.usage.is_none());
    }

    #[test]
    fn chat_stream_chunk_with_role_delta() {
        let json = r#"{
            "id": "chatcmpl-abc",
            "choices": [{
                "index": 0,
                "delta": {
                    "role": "assistant"
                }
            }]
        }"#;
        let chunk: ChatStreamChunk = serde_json::from_str(json).unwrap();
        assert_eq!(
            chunk.choices[0].delta.role,
            Some(crate::types::common::Role::Assistant)
        );
        assert!(chunk.choices[0].delta.content.is_none());
    }

    #[test]
    fn chat_stream_chunk_with_finish_reason() {
        let json = r#"{
            "id": "chatcmpl-abc",
            "choices": [{
                "index": 0,
                "delta": {},
                "finish_reason": "stop"
            }]
        }"#;
        let chunk: ChatStreamChunk = serde_json::from_str(json).unwrap();
        assert_eq!(chunk.choices[0].finish_reason, Some(FinishReason::Stop));
    }

    #[test]
    fn chat_stream_chunk_with_usage_in_final_chunk() {
        let json = r#"{
            "id": "chatcmpl-abc",
            "choices": [],
            "usage": {
                "prompt_tokens": 10,
                "completion_tokens": 5,
                "total_tokens": 15
            }
        }"#;
        let chunk: ChatStreamChunk = serde_json::from_str(json).unwrap();
        let usage = chunk.usage.unwrap();
        assert_eq!(usage.prompt_tokens, 10);
        assert_eq!(usage.completion_tokens, 5);
        assert_eq!(usage.total_tokens, Some(15));
    }

    #[test]
    fn chat_stream_chunk_with_tool_call_delta() {
        let json = r#"{
            "id": "chatcmpl-abc",
            "choices": [{
                "index": 0,
                "delta": {
                    "tool_calls": [{
                        "index": 0,
                        "id": "call_abc",
                        "type": "function",
                        "function": {
                            "name": "get_weather",
                            "arguments": "{\"location\":"
                        }
                    }]
                }
            }]
        }"#;
        let chunk: ChatStreamChunk = serde_json::from_str(json).unwrap();
        let tool_calls = chunk.choices[0].delta.tool_calls.as_ref().unwrap();
        assert_eq!(tool_calls.len(), 1);
        assert_eq!(tool_calls[0].index, 0);
        assert_eq!(tool_calls[0].id.as_deref(), Some("call_abc"));
        assert_eq!(tool_calls[0].r#type.as_deref(), Some("function"));
        let func = tool_calls[0].function.as_ref().unwrap();
        assert_eq!(func.name.as_deref(), Some("get_weather"));
        assert_eq!(func.arguments.as_deref(), Some("{\"location\":"));
    }

    #[test]
    fn chat_stream_chunk_with_reasoning_content() {
        let json = r#"{
            "id": "chatcmpl-abc",
            "choices": [{
                "index": 0,
                "delta": {
                    "reasoning_content": "Let me think about this..."
                }
            }]
        }"#;
        let chunk: ChatStreamChunk = serde_json::from_str(json).unwrap();
        assert_eq!(
            chunk.choices[0].delta.reasoning_content.as_deref(),
            Some("Let me think about this...")
        );
    }

    #[test]
    fn chat_stream_chunk_skips_none_fields() {
        let chunk = ChatStreamChunk {
            id: "test".into(),
            object: None,
            created: None,
            model: None,
            choices: vec![],
            usage: None,
            system_fingerprint: None,
        };
        let json = serde_json::to_string(&chunk).unwrap();
        assert!(!json.contains("\"object\""));
        assert!(!json.contains("\"created\""));
        assert!(!json.contains("\"model\""));
        assert!(!json.contains("\"usage\""));
        assert!(!json.contains("\"system_fingerprint\""));
    }

    // -- ResponseStreamEvent deserialization --

    #[test]
    fn response_stream_event_response_created() {
        let json = r#"{
            "type": "response.created",
            "response": {"id": "resp_abc", "status": "in_progress"}
        }"#;
        let event: ResponseStreamEvent = serde_json::from_str(json).unwrap();
        match &event {
            ResponseStreamEvent::ResponseCreated { response } => {
                assert_eq!(response["id"], "resp_abc");
            }
            other => panic!("expected ResponseCreated, got: {other:?}"),
        }
    }

    #[test]
    fn response_stream_event_response_in_progress() {
        let json = r#"{
            "type": "response.in_progress",
            "response": {"id": "resp_abc", "status": "in_progress"}
        }"#;
        let event: ResponseStreamEvent = serde_json::from_str(json).unwrap();
        assert!(matches!(
            event,
            ResponseStreamEvent::ResponseInProgress { .. }
        ));
    }

    #[test]
    fn response_stream_event_response_completed() {
        let json = r#"{
            "type": "response.completed",
            "response": {"id": "resp_abc", "status": "completed"}
        }"#;
        let event: ResponseStreamEvent = serde_json::from_str(json).unwrap();
        assert!(matches!(
            event,
            ResponseStreamEvent::ResponseCompleted { .. }
        ));
    }

    #[test]
    fn response_stream_event_response_failed() {
        let json = r#"{
            "type": "response.failed",
            "response": {"id": "resp_abc", "status": "failed", "error": {"message": "boom"}}
        }"#;
        let event: ResponseStreamEvent = serde_json::from_str(json).unwrap();
        assert!(matches!(event, ResponseStreamEvent::ResponseFailed { .. }));
    }

    #[test]
    fn response_stream_event_output_item_added() {
        let json = r#"{
            "type": "response.output_item.added",
            "output_index": 0,
            "item": {"type": "message", "role": "assistant"}
        }"#;
        let event: ResponseStreamEvent = serde_json::from_str(json).unwrap();
        match &event {
            ResponseStreamEvent::OutputItemAdded { output_index, item } => {
                assert_eq!(*output_index, 0);
                assert_eq!(item["type"], "message");
            }
            other => panic!("expected OutputItemAdded, got: {other:?}"),
        }
    }

    #[test]
    fn response_stream_event_output_item_done() {
        let json = r#"{
            "type": "response.output_item.done",
            "output_index": 0,
            "item": {"type": "message", "role": "assistant", "content": []}
        }"#;
        let event: ResponseStreamEvent = serde_json::from_str(json).unwrap();
        assert!(matches!(event, ResponseStreamEvent::OutputItemDone { .. }));
    }

    #[test]
    fn response_stream_event_content_part_added() {
        let json = r#"{
            "type": "response.content_part.added",
            "output_index": 0,
            "content_index": 0,
            "part": {"type": "text", "text": ""}
        }"#;
        let event: ResponseStreamEvent = serde_json::from_str(json).unwrap();
        match &event {
            ResponseStreamEvent::ContentPartAdded {
                output_index,
                content_index,
                part,
            } => {
                assert_eq!(*output_index, 0);
                assert_eq!(*content_index, 0);
                assert_eq!(part["type"], "text");
            }
            other => panic!("expected ContentPartAdded, got: {other:?}"),
        }
    }

    #[test]
    fn response_stream_event_content_part_done() {
        let json = r#"{
            "type": "response.content_part.done",
            "output_index": 0,
            "content_index": 0,
            "part": {"type": "text", "text": "Hello, world!"}
        }"#;
        let event: ResponseStreamEvent = serde_json::from_str(json).unwrap();
        assert!(matches!(event, ResponseStreamEvent::ContentPartDone { .. }));
    }

    #[test]
    fn response_stream_event_content_delta_with_text() {
        let json = r#"{
            "type": "response.content_part.delta",
            "output_index": 0,
            "content_index": 0,
            "delta": {"type": "text", "text": "Hello"}
        }"#;
        let event: ResponseStreamEvent = serde_json::from_str(json).unwrap();
        match &event {
            ResponseStreamEvent::ContentDelta {
                output_index,
                content_index,
                delta,
            } => {
                assert_eq!(*output_index, 0);
                assert_eq!(*content_index, 0);
                assert_eq!(delta.r#type.as_deref(), Some("text"));
                assert_eq!(delta.text.as_deref(), Some("Hello"));
            }
            other => panic!("expected ContentDelta, got: {other:?}"),
        }
    }

    #[test]
    fn response_stream_event_function_call_arguments_delta() {
        let json = r#"{
            "type": "response.function_call_arguments.delta",
            "output_index": 1,
            "item_id": "item_abc",
            "call_id": "call_xyz",
            "delta": "{\"loc"
        }"#;
        let event: ResponseStreamEvent = serde_json::from_str(json).unwrap();
        match &event {
            ResponseStreamEvent::FunctionCallArgumentsDelta {
                output_index,
                item_id,
                call_id,
                delta,
            } => {
                assert_eq!(*output_index, 1);
                assert_eq!(item_id.as_deref(), Some("item_abc"));
                assert_eq!(call_id.as_deref(), Some("call_xyz"));
                assert_eq!(delta, "{\"loc");
            }
            other => panic!("expected FunctionCallArgumentsDelta, got: {other:?}"),
        }
    }

    #[test]
    fn response_stream_event_function_call_arguments_done() {
        let json = r#"{
            "type": "response.function_call_arguments.done",
            "output_index": 1,
            "item_id": "item_abc",
            "call_id": "call_xyz",
            "arguments": "{\"location\":\"Tokyo\"}"
        }"#;
        let event: ResponseStreamEvent = serde_json::from_str(json).unwrap();
        match &event {
            ResponseStreamEvent::FunctionCallArgumentsDone {
                output_index,
                item_id,
                call_id,
                arguments,
            } => {
                assert_eq!(*output_index, 1);
                assert_eq!(item_id.as_deref(), Some("item_abc"));
                assert_eq!(call_id.as_deref(), Some("call_xyz"));
                assert_eq!(arguments, "{\"location\":\"Tokyo\"}");
            }
            other => panic!("expected FunctionCallArgumentsDone, got: {other:?}"),
        }
    }

    #[test]
    fn response_stream_event_output_text_delta() {
        let json = r#"{
            "type": "response.output_text.delta",
            "output_index": 0,
            "content_index": 0,
            "delta": "Hello, "
        }"#;
        let event: ResponseStreamEvent = serde_json::from_str(json).unwrap();
        match &event {
            ResponseStreamEvent::OutputTextDelta {
                output_index,
                content_index,
                delta,
            } => {
                assert_eq!(*output_index, 0);
                assert_eq!(*content_index, Some(0));
                assert_eq!(delta, "Hello, ");
            }
            other => panic!("expected OutputTextDelta, got: {other:?}"),
        }
    }

    #[test]
    fn response_stream_event_output_text_delta_no_content_index() {
        let json = r#"{
            "type": "response.output_text.delta",
            "output_index": 1,
            "delta": "world"
        }"#;
        let event: ResponseStreamEvent = serde_json::from_str(json).unwrap();
        match &event {
            ResponseStreamEvent::OutputTextDelta {
                output_index,
                content_index,
                delta,
            } => {
                assert_eq!(*output_index, 1);
                assert!(content_index.is_none());
                assert_eq!(delta, "world");
            }
            other => panic!("expected OutputTextDelta, got: {other:?}"),
        }
    }

    #[test]
    fn response_stream_event_output_text_done() {
        let json = r#"{
            "type": "response.output_text.done",
            "output_index": 0,
            "content_index": 0,
            "text": "Hello, world!"
        }"#;
        let event: ResponseStreamEvent = serde_json::from_str(json).unwrap();
        match &event {
            ResponseStreamEvent::OutputTextDone {
                output_index,
                content_index,
                text,
            } => {
                assert_eq!(*output_index, 0);
                assert_eq!(*content_index, Some(0));
                assert_eq!(text, "Hello, world!");
            }
            other => panic!("expected OutputTextDone, got: {other:?}"),
        }
    }

    #[test]
    fn response_stream_event_output_text_done_no_content_index() {
        let json = r#"{
            "type": "response.output_text.done",
            "output_index": 2,
            "text": "Complete text."
        }"#;
        let event: ResponseStreamEvent = serde_json::from_str(json).unwrap();
        match &event {
            ResponseStreamEvent::OutputTextDone {
                output_index,
                content_index,
                text,
            } => {
                assert_eq!(*output_index, 2);
                assert!(content_index.is_none());
                assert_eq!(text, "Complete text.");
            }
            other => panic!("expected OutputTextDone, got: {other:?}"),
        }
    }

    // -- MCP stream events --

    #[test]
    fn response_stream_event_mcp_list_tools_in_progress() {
        let json = r#"{
            "type": "mcp_list_tools.in_progress",
            "server_label": "my-mcp-server"
        }"#;
        let event: ResponseStreamEvent = serde_json::from_str(json).unwrap();
        match &event {
            ResponseStreamEvent::McpListToolsInProgress { server_label } => {
                assert_eq!(server_label.as_deref(), Some("my-mcp-server"));
            }
            other => panic!("expected McpListToolsInProgress, got: {other:?}"),
        }
    }

    #[test]
    fn response_stream_event_mcp_list_tools_in_progress_no_label() {
        let json = r#"{"type": "mcp_list_tools.in_progress"}"#;
        let event: ResponseStreamEvent = serde_json::from_str(json).unwrap();
        assert!(matches!(
            event,
            ResponseStreamEvent::McpListToolsInProgress { server_label: None }
        ));
    }

    #[test]
    fn response_stream_event_mcp_list_tools_completed() {
        let json = r#"{
            "type": "mcp_list_tools.completed",
            "server_label": "prod-server",
            "tools": [
                {"name": "search", "description": "Search the web"},
                {"name": "fetch", "description": "Fetch a URL"}
            ]
        }"#;
        let event: ResponseStreamEvent = serde_json::from_str(json).unwrap();
        match &event {
            ResponseStreamEvent::McpListToolsCompleted {
                server_label,
                tools,
            } => {
                assert_eq!(server_label.as_deref(), Some("prod-server"));
                let tools = tools.as_ref().unwrap();
                assert_eq!(tools.len(), 2);
                assert_eq!(tools[0]["name"], "search");
                assert_eq!(tools[1]["name"], "fetch");
            }
            other => panic!("expected McpListToolsCompleted, got: {other:?}"),
        }
    }

    #[test]
    fn response_stream_event_mcp_list_tools_failed() {
        let json = r#"{
            "type": "mcp_list_tools.failed",
            "server_label": "bad-server",
            "error": {"message": "connection refused", "code": "mcp_connect_error"}
        }"#;
        let event: ResponseStreamEvent = serde_json::from_str(json).unwrap();
        match &event {
            ResponseStreamEvent::McpListToolsFailed {
                server_label,
                error,
            } => {
                assert_eq!(server_label.as_deref(), Some("bad-server"));
                let err = error.as_ref().unwrap();
                assert_eq!(err["message"], "connection refused");
            }
            other => panic!("expected McpListToolsFailed, got: {other:?}"),
        }
    }

    #[test]
    fn response_stream_event_mcp_call_arguments_delta() {
        let json = r#"{
            "type": "response.mcp_call_arguments.delta",
            "output_index": 2,
            "item_id": "mcp_item_1",
            "delta": "{\"query\":"
        }"#;
        let event: ResponseStreamEvent = serde_json::from_str(json).unwrap();
        match &event {
            ResponseStreamEvent::McpCallArgumentsDelta {
                output_index,
                item_id,
                delta,
            } => {
                assert_eq!(*output_index, 2);
                assert_eq!(item_id.as_deref(), Some("mcp_item_1"));
                assert_eq!(delta, "{\"query\":");
            }
            other => panic!("expected McpCallArgumentsDelta, got: {other:?}"),
        }
    }

    #[test]
    fn response_stream_event_mcp_call_arguments_done() {
        let json = r#"{
            "type": "response.mcp_call_arguments.done",
            "output_index": 2,
            "item_id": "mcp_item_1",
            "arguments": "{\"query\":\"hello world\"}"
        }"#;
        let event: ResponseStreamEvent = serde_json::from_str(json).unwrap();
        match &event {
            ResponseStreamEvent::McpCallArgumentsDone {
                output_index,
                item_id,
                arguments,
            } => {
                assert_eq!(*output_index, 2);
                assert_eq!(item_id.as_deref(), Some("mcp_item_1"));
                assert_eq!(arguments, "{\"query\":\"hello world\"}");
            }
            other => panic!("expected McpCallArgumentsDone, got: {other:?}"),
        }
    }

    #[test]
    fn response_stream_event_mcp_call_in_progress() {
        let json = r#"{
            "type": "response.mcp_call.in_progress",
            "output_index": 1,
            "item": {"type": "mcp_call", "id": "mcp_1", "status": "in_progress"}
        }"#;
        let event: ResponseStreamEvent = serde_json::from_str(json).unwrap();
        match &event {
            ResponseStreamEvent::McpCallInProgress { output_index, item } => {
                assert_eq!(*output_index, 1);
                let item = item.as_ref().unwrap();
                assert_eq!(item["status"], "in_progress");
            }
            other => panic!("expected McpCallInProgress, got: {other:?}"),
        }
    }

    #[test]
    fn response_stream_event_mcp_call_completed() {
        let json = r#"{
            "type": "response.mcp_call.completed",
            "output_index": 1,
            "item": {
                "type": "mcp_call",
                "id": "mcp_1",
                "status": "completed",
                "output": "Search result: ..."
            }
        }"#;
        let event: ResponseStreamEvent = serde_json::from_str(json).unwrap();
        match &event {
            ResponseStreamEvent::McpCallCompleted { output_index, item } => {
                assert_eq!(*output_index, 1);
                let item = item.as_ref().unwrap();
                assert_eq!(item["status"], "completed");
                assert_eq!(item["output"], "Search result: ...");
            }
            other => panic!("expected McpCallCompleted, got: {other:?}"),
        }
    }

    #[test]
    fn response_stream_event_mcp_call_failed() {
        let json = r#"{
            "type": "response.mcp_call.failed",
            "output_index": 1,
            "item": {
                "type": "mcp_call",
                "id": "mcp_1",
                "status": "failed",
                "error": {"message": "tool execution failed"}
            }
        }"#;
        let event: ResponseStreamEvent = serde_json::from_str(json).unwrap();
        match &event {
            ResponseStreamEvent::McpCallFailed { output_index, item } => {
                assert_eq!(*output_index, 1);
                let item = item.as_ref().unwrap();
                assert_eq!(item["status"], "failed");
            }
            other => panic!("expected McpCallFailed, got: {other:?}"),
        }
    }

    #[test]
    fn response_stream_event_mcp_events_round_trip() {
        // Verify MCP events survive serialization round-trip
        let events = vec![
            ResponseStreamEvent::McpListToolsInProgress {
                server_label: Some("test".into()),
            },
            ResponseStreamEvent::McpListToolsCompleted {
                server_label: Some("test".into()),
                tools: Some(vec![serde_json::json!({"name": "tool_a"})]),
            },
            ResponseStreamEvent::McpListToolsFailed {
                server_label: None,
                error: Some(serde_json::json!({"message": "fail"})),
            },
            ResponseStreamEvent::McpCallArgumentsDelta {
                output_index: 0,
                item_id: Some("item_1".into()),
                delta: "{".into(),
            },
            ResponseStreamEvent::McpCallArgumentsDone {
                output_index: 0,
                item_id: None,
                arguments: "{}".into(),
            },
            ResponseStreamEvent::McpCallInProgress {
                output_index: 1,
                item: None,
            },
            ResponseStreamEvent::McpCallCompleted {
                output_index: 1,
                item: Some(serde_json::json!({"id": "mcp_1"})),
            },
            ResponseStreamEvent::McpCallFailed {
                output_index: 2,
                item: None,
            },
        ];

        for event in &events {
            let json = serde_json::to_string(event).unwrap();
            let back: ResponseStreamEvent = serde_json::from_str(&json).unwrap();
            assert_eq!(event, &back, "round-trip failed for: {json}");
        }
    }

    #[test]
    fn response_stream_event_future_mcp_event_falls_through_to_unknown() {
        // Future MCP events we haven't modeled should still deserialize as Unknown
        let json = r#"{
            "type": "mcp_list_tools.some_future_event",
            "data": "whatever"
        }"#;
        let event: ResponseStreamEvent = serde_json::from_str(json).unwrap();
        assert!(matches!(event, ResponseStreamEvent::Unknown));
    }

    #[test]
    fn response_stream_event_unknown_type_falls_through() {
        let json = r#"{
            "type": "response.some_future_event",
            "data": "whatever"
        }"#;
        let event: ResponseStreamEvent = serde_json::from_str(json).unwrap();
        assert!(
            matches!(event, ResponseStreamEvent::Unknown),
            "unrecognised event type should deserialize as Unknown"
        );
    }

    #[test]
    fn response_stream_event_round_trips() {
        let event = ResponseStreamEvent::ContentDelta {
            output_index: 0,
            content_index: 1,
            delta: ContentDeltaPayload {
                r#type: Some("text".into()),
                text: Some("Hi".into()),
            },
        };
        let json = serde_json::to_string(&event).unwrap();
        let back: ResponseStreamEvent = serde_json::from_str(&json).unwrap();
        assert_eq!(event, back);
    }

    // -- ContentDeltaPayload --

    #[test]
    fn content_delta_payload_deserializes() {
        let json = r#"{"type": "text", "text": "world"}"#;
        let payload: ContentDeltaPayload = serde_json::from_str(json).unwrap();
        assert_eq!(payload.r#type.as_deref(), Some("text"));
        assert_eq!(payload.text.as_deref(), Some("world"));
    }

    #[test]
    fn content_delta_payload_skips_none() {
        let payload = ContentDeltaPayload {
            r#type: None,
            text: None,
        };
        let json = serde_json::to_string(&payload).unwrap();
        assert_eq!(json, "{}");
    }

    // -- StreamError --

    #[test]
    fn stream_error_parse_displays() {
        let err = StreamError::Parse {
            message: "bad json".into(),
        };
        let display = format!("{err}");
        assert!(display.contains("parse error"));
        assert!(display.contains("bad json"));
    }

    #[test]
    fn stream_error_connection_lost_displays() {
        let err = StreamError::ConnectionLost;
        assert!(format!("{err}").contains("connection lost"));
    }

    #[test]
    fn stream_error_is_std_error() {
        let err = StreamError::ConnectionLost;
        let _: &dyn std::error::Error = &err;
    }

    #[test]
    fn stream_error_from_transport_error() {
        let transport = TransportError::Timeout;
        let stream_err: StreamError = transport.into();
        assert!(matches!(stream_err, StreamError::Transport(_)));
    }
}
