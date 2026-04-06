use serde::{Deserialize, Serialize};

/// Token usage for the xAI Chat Completions API.
///
/// Uses `prompt_tokens`/`completion_tokens` field names as per the
/// Chat Completions wire format.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ChatUsage {
    /// Number of tokens in the prompt.
    pub prompt_tokens: u64,
    /// Number of tokens in the completion.
    pub completion_tokens: u64,
    /// Total token count.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub total_tokens: Option<u64>,
    /// Breakdown of prompt token usage by category.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub prompt_tokens_details: Option<PromptTokensDetails>,
    /// Breakdown of completion token usage by category.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub completion_tokens_details: Option<CompletionTokensDetails>,
    /// Cost in integer USD ticks. NEVER floating point.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cost_in_usd_ticks: Option<i64>,
    /// Number of sources used (e.g., web search results) at the top level.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub num_sources_used: Option<u64>,
}

/// Token usage for the xAI Responses API.
///
/// Uses `input_tokens`/`output_tokens` field names as per the Responses wire format.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ResponsesUsage {
    /// Number of tokens in the input.
    pub input_tokens: u64,
    /// Number of tokens in the output.
    pub output_tokens: u64,
    /// Total token count.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub total_tokens: Option<u64>,
    /// Breakdown of output token usage.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub output_tokens_details: Option<OutputTokensDetails>,
    /// Cost in integer USD ticks. NEVER floating point.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cost_in_usd_ticks: Option<i64>,
    /// Cost in integer nano-USD. NEVER floating point.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cost_in_nano_usd: Option<i64>,
    /// Number of sources used (e.g., web search results) at the top level.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub num_sources_used: Option<u64>,
    /// Per-tool token usage details from server-side tool executions.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub server_side_tool_usage_details: Option<Vec<ServerSideToolUsageDetails>>,
}

/// Unified usage type that can deserialize from either Chat Completions
/// or Responses API wire format.
///
/// Uses serde aliases so that `prompt_tokens` and `input_tokens` both
/// deserialize into `input_tokens`, and likewise for output.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Usage {
    /// Number of tokens in the input/prompt.
    /// Deserializes from either `input_tokens` or `prompt_tokens`.
    #[serde(alias = "prompt_tokens")]
    pub input_tokens: u64,
    /// Number of tokens in the output/completion.
    /// Deserializes from either `output_tokens` or `completion_tokens`.
    #[serde(alias = "completion_tokens")]
    pub output_tokens: u64,
    /// Number of reasoning tokens used (if the model supports reasoning).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reasoning_tokens: Option<u64>,
    /// Total token count (input + output + reasoning).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub total_tokens: Option<u64>,
    /// Breakdown of prompt token usage by category (Chat Completions).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub prompt_tokens_details: Option<PromptTokensDetails>,
    /// Breakdown of completion token usage by category (Chat Completions).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub completion_tokens_details: Option<CompletionTokensDetails>,
    /// Breakdown of output token usage (Responses API).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub output_tokens_details: Option<OutputTokensDetails>,
    /// Cost in integer USD ticks. NEVER floating point.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cost_in_usd_ticks: Option<i64>,
    /// Cost in integer nano-USD. NEVER floating point.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cost_in_nano_usd: Option<i64>,
    /// Number of sources used (e.g., web search results) at the top level.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub num_sources_used: Option<u64>,
    /// Per-tool token usage details from server-side tool executions.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub server_side_tool_usage_details: Option<Vec<ServerSideToolUsageDetails>>,
}

/// Breakdown of prompt/input token usage.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PromptTokensDetails {
    /// Tokens served from cache.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cached_tokens: Option<u64>,
    /// Tokens from text content.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub text_tokens: Option<u64>,
    /// Tokens from image content.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub image_tokens: Option<u64>,
    /// Tokens from audio content.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub audio_tokens: Option<u64>,
}

/// Breakdown of completion/output token usage (Chat Completions style).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CompletionTokensDetails {
    /// Tokens used for reasoning in the completion.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reasoning_tokens: Option<u64>,
    /// Tokens from text content in the completion.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub text_tokens: Option<u64>,
    /// Tokens from audio content in the completion.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub audio_tokens: Option<u64>,
    /// Tokens accepted from predicted output.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub accepted_prediction_tokens: Option<u64>,
    /// Tokens rejected from predicted output.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub rejected_prediction_tokens: Option<u64>,
}

/// Breakdown of output token usage (Responses API style).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct OutputTokensDetails {
    /// Tokens used for reasoning in the output.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reasoning_tokens: Option<u64>,
    /// Tokens from text content in the output.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub text_tokens: Option<u64>,
    /// Tokens accepted from predicted output.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub accepted_prediction_tokens: Option<u64>,
    /// Tokens rejected from predicted output.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub rejected_prediction_tokens: Option<u64>,
}

/// Server-side tool usage details with per-tool token counts.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ServerSideToolUsageDetails {
    /// Name of the server-side tool.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool_name: Option<String>,
    /// Tokens consumed by the tool input.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub input_tokens: Option<u64>,
    /// Tokens produced by the tool output.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub output_tokens: Option<u64>,
    /// Number of sources used by the tool (e.g., web search results).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub num_sources_used: Option<u64>,
    /// Number of code interpreter invocations.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub code_interpreter_calls: Option<u64>,
    /// Number of document search invocations.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub document_search_calls: Option<u64>,
    /// Number of file search invocations.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub file_search_calls: Option<u64>,
    /// Number of MCP tool invocations.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub mcp_calls: Option<u64>,
    /// Number of web search invocations.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub web_search_calls: Option<u64>,
    /// Number of X (formerly Twitter) search invocations.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub x_search_calls: Option<u64>,
}

#[cfg(test)]
mod tests {
    use super::*;

    // -- Unified Usage tests --

    #[test]
    fn usage_round_trips_minimal() {
        let usage = Usage {
            input_tokens: 100,
            output_tokens: 50,
            reasoning_tokens: None,
            total_tokens: None,
            prompt_tokens_details: None,
            completion_tokens_details: None,
            output_tokens_details: None,
            cost_in_usd_ticks: None,
            cost_in_nano_usd: None,
            num_sources_used: None,
            server_side_tool_usage_details: None,
        };
        let json = serde_json::to_string(&usage).unwrap();
        let back: Usage = serde_json::from_str(&json).unwrap();
        assert_eq!(usage, back);
    }

    #[test]
    fn usage_round_trips_full() {
        let usage = Usage {
            input_tokens: 1000,
            output_tokens: 500,
            reasoning_tokens: Some(200),
            total_tokens: Some(1700),
            prompt_tokens_details: Some(PromptTokensDetails {
                cached_tokens: Some(300),
                text_tokens: Some(600),
                image_tokens: Some(100),
                audio_tokens: None,
            }),
            completion_tokens_details: Some(CompletionTokensDetails {
                reasoning_tokens: Some(200),
                text_tokens: Some(300),
                audio_tokens: None,
                accepted_prediction_tokens: None,
                rejected_prediction_tokens: None,
            }),
            output_tokens_details: None,
            cost_in_usd_ticks: Some(42_000),
            cost_in_nano_usd: None,
            num_sources_used: Some(3),
            server_side_tool_usage_details: None,
        };
        let json = serde_json::to_string(&usage).unwrap();
        let back: Usage = serde_json::from_str(&json).unwrap();
        assert_eq!(usage, back);
    }

    #[test]
    fn usage_deserializes_from_chat_completions_wire() {
        let json = r#"{
            "prompt_tokens": 100,
            "completion_tokens": 50,
            "total_tokens": 150,
            "prompt_tokens_details": {"cached_tokens": 20},
            "completion_tokens_details": {"reasoning_tokens": 10}
        }"#;
        let usage: Usage = serde_json::from_str(json).unwrap();
        assert_eq!(usage.input_tokens, 100);
        assert_eq!(usage.output_tokens, 50);
        assert_eq!(usage.total_tokens, Some(150));
        assert!(usage.prompt_tokens_details.is_some());
        assert!(usage.completion_tokens_details.is_some());
    }

    #[test]
    fn usage_deserializes_from_responses_wire() {
        let json = r#"{
            "input_tokens": 200,
            "output_tokens": 100,
            "total_tokens": 300,
            "output_tokens_details": {"reasoning_tokens": 50}
        }"#;
        let usage: Usage = serde_json::from_str(json).unwrap();
        assert_eq!(usage.input_tokens, 200);
        assert_eq!(usage.output_tokens, 100);
        assert!(usage.output_tokens_details.is_some());
        let details = usage.output_tokens_details.unwrap();
        assert_eq!(details.reasoning_tokens, Some(50));
    }

    #[test]
    fn cost_in_usd_ticks_is_i64() {
        let json = r#"{"input_tokens":10,"output_tokens":5,"cost_in_usd_ticks":-100}"#;
        let usage: Usage = serde_json::from_str(json).unwrap();
        assert_eq!(usage.cost_in_usd_ticks, Some(-100i64));
    }

    #[test]
    fn usage_skips_none_optional_fields() {
        let usage = Usage {
            input_tokens: 10,
            output_tokens: 5,
            reasoning_tokens: None,
            total_tokens: None,
            prompt_tokens_details: None,
            completion_tokens_details: None,
            output_tokens_details: None,
            cost_in_usd_ticks: None,
            cost_in_nano_usd: None,
            num_sources_used: None,
            server_side_tool_usage_details: None,
        };
        let json = serde_json::to_string(&usage).unwrap();
        assert!(!json.contains("\"reasoning_tokens\""));
        assert!(!json.contains("\"total_tokens\""));
        assert!(!json.contains("\"prompt_tokens_details\""));
        assert!(!json.contains("\"completion_tokens_details\""));
        assert!(!json.contains("\"output_tokens_details\""));
        assert!(!json.contains("\"cost_in_usd_ticks\""));
        assert!(!json.contains("\"cost_in_nano_usd\""));
        assert!(!json.contains("\"num_sources_used\""));
    }

    #[test]
    fn usage_deserializes_with_unknown_fields() {
        let json = r#"{
            "input_tokens": 10,
            "output_tokens": 5,
            "some_new_field": "value"
        }"#;
        let usage: Usage = serde_json::from_str(json).unwrap();
        assert_eq!(usage.input_tokens, 10);
    }

    // -- ChatUsage tests --

    #[test]
    fn chat_usage_round_trips() {
        let usage = ChatUsage {
            prompt_tokens: 100,
            completion_tokens: 50,
            total_tokens: Some(150),
            prompt_tokens_details: None,
            completion_tokens_details: None,
            cost_in_usd_ticks: None,
            num_sources_used: None,
        };
        let json = serde_json::to_string(&usage).unwrap();
        assert!(json.contains("\"prompt_tokens\":100"));
        assert!(json.contains("\"completion_tokens\":50"));
        let back: ChatUsage = serde_json::from_str(&json).unwrap();
        assert_eq!(usage, back);
    }

    #[test]
    fn chat_usage_with_prediction_tokens() {
        let usage = ChatUsage {
            prompt_tokens: 100,
            completion_tokens: 50,
            total_tokens: Some(150),
            prompt_tokens_details: None,
            completion_tokens_details: Some(CompletionTokensDetails {
                reasoning_tokens: Some(10),
                text_tokens: Some(30),
                audio_tokens: None,
                accepted_prediction_tokens: Some(5),
                rejected_prediction_tokens: Some(3),
            }),
            cost_in_usd_ticks: None,
            num_sources_used: None,
        };
        let json = serde_json::to_string(&usage).unwrap();
        assert!(json.contains("\"accepted_prediction_tokens\":5"));
        assert!(json.contains("\"rejected_prediction_tokens\":3"));
        let back: ChatUsage = serde_json::from_str(&json).unwrap();
        assert_eq!(usage, back);
    }

    // -- ResponsesUsage tests --

    #[test]
    fn responses_usage_round_trips() {
        let usage = ResponsesUsage {
            input_tokens: 200,
            output_tokens: 100,
            total_tokens: Some(300),
            output_tokens_details: Some(OutputTokensDetails {
                reasoning_tokens: Some(50),
                text_tokens: Some(50),
                accepted_prediction_tokens: None,
                rejected_prediction_tokens: None,
            }),
            cost_in_usd_ticks: None,
            cost_in_nano_usd: None,
            num_sources_used: None,
            server_side_tool_usage_details: None,
        };
        let json = serde_json::to_string(&usage).unwrap();
        assert!(json.contains("\"input_tokens\":200"));
        assert!(json.contains("\"output_tokens\":100"));
        assert!(json.contains("\"output_tokens_details\""));
        let back: ResponsesUsage = serde_json::from_str(&json).unwrap();
        assert_eq!(usage, back);
    }

    #[test]
    fn responses_usage_deserializes_from_wire() {
        let json = r#"{
            "input_tokens": 500,
            "output_tokens": 250,
            "total_tokens": 750,
            "output_tokens_details": {
                "reasoning_tokens": 100,
                "accepted_prediction_tokens": 10,
                "rejected_prediction_tokens": 2
            }
        }"#;
        let usage: ResponsesUsage = serde_json::from_str(json).unwrap();
        assert_eq!(usage.input_tokens, 500);
        assert_eq!(usage.output_tokens, 250);
        let details = usage.output_tokens_details.unwrap();
        assert_eq!(details.accepted_prediction_tokens, Some(10));
        assert_eq!(details.rejected_prediction_tokens, Some(2));
    }

    // -- ServerSideToolUsageDetails tests --

    #[test]
    fn server_side_tool_usage_round_trips() {
        let detail = ServerSideToolUsageDetails {
            tool_name: Some("web_search".into()),
            input_tokens: Some(50),
            output_tokens: Some(200),
            num_sources_used: Some(5),
            code_interpreter_calls: None,
            document_search_calls: None,
            file_search_calls: None,
            mcp_calls: None,
            web_search_calls: None,
            x_search_calls: None,
        };
        let json = serde_json::to_string(&detail).unwrap();
        let back: ServerSideToolUsageDetails = serde_json::from_str(&json).unwrap();
        assert_eq!(detail, back);
    }

    #[test]
    fn server_side_tool_usage_num_sources_used() {
        let json = r#"{"tool_name":"web_search","input_tokens":50,"output_tokens":200,"num_sources_used":8}"#;
        let detail: ServerSideToolUsageDetails = serde_json::from_str(json).unwrap();
        assert_eq!(detail.num_sources_used, Some(8));
    }

    // -- num_sources_used top-level tests --

    #[test]
    fn chat_usage_num_sources_used_round_trips() {
        let usage = ChatUsage {
            prompt_tokens: 100,
            completion_tokens: 50,
            total_tokens: Some(150),
            prompt_tokens_details: None,
            completion_tokens_details: None,
            cost_in_usd_ticks: None,
            num_sources_used: Some(7),
        };
        let json = serde_json::to_string(&usage).unwrap();
        assert!(json.contains("\"num_sources_used\":7"));
        let back: ChatUsage = serde_json::from_str(&json).unwrap();
        assert_eq!(usage, back);
    }

    #[test]
    fn responses_usage_num_sources_used_round_trips() {
        let usage = ResponsesUsage {
            input_tokens: 200,
            output_tokens: 100,
            total_tokens: Some(300),
            output_tokens_details: None,
            cost_in_usd_ticks: None,
            cost_in_nano_usd: None,
            num_sources_used: Some(12),
            server_side_tool_usage_details: None,
        };
        let json = serde_json::to_string(&usage).unwrap();
        assert!(json.contains("\"num_sources_used\":12"));
        let back: ResponsesUsage = serde_json::from_str(&json).unwrap();
        assert_eq!(usage, back);
    }

    #[test]
    fn unified_usage_num_sources_used_round_trips() {
        let usage = Usage {
            input_tokens: 500,
            output_tokens: 250,
            reasoning_tokens: None,
            total_tokens: Some(750),
            prompt_tokens_details: None,
            completion_tokens_details: None,
            output_tokens_details: None,
            cost_in_usd_ticks: None,
            cost_in_nano_usd: None,
            num_sources_used: Some(4),
            server_side_tool_usage_details: None,
        };
        let json = serde_json::to_string(&usage).unwrap();
        assert!(json.contains("\"num_sources_used\":4"));
        let back: Usage = serde_json::from_str(&json).unwrap();
        assert_eq!(usage, back);
    }

    #[test]
    fn unified_usage_deserializes_num_sources_used_from_wire() {
        let json = r#"{
            "input_tokens": 100,
            "output_tokens": 50,
            "num_sources_used": 9
        }"#;
        let usage: Usage = serde_json::from_str(json).unwrap();
        assert_eq!(usage.num_sources_used, Some(9));
    }

    // -- call-count fields on ServerSideToolUsageDetails --

    #[test]
    fn server_side_tool_usage_call_counts_round_trip() {
        let detail = ServerSideToolUsageDetails {
            tool_name: Some("aggregate".into()),
            input_tokens: Some(100),
            output_tokens: Some(200),
            num_sources_used: Some(3),
            code_interpreter_calls: Some(2),
            document_search_calls: Some(1),
            file_search_calls: Some(4),
            mcp_calls: Some(0),
            web_search_calls: Some(5),
            x_search_calls: Some(3),
        };
        let json = serde_json::to_string(&detail).unwrap();
        assert!(json.contains("\"code_interpreter_calls\":2"));
        assert!(json.contains("\"document_search_calls\":1"));
        assert!(json.contains("\"file_search_calls\":4"));
        assert!(json.contains("\"mcp_calls\":0"));
        assert!(json.contains("\"web_search_calls\":5"));
        assert!(json.contains("\"x_search_calls\":3"));
        let back: ServerSideToolUsageDetails = serde_json::from_str(&json).unwrap();
        assert_eq!(detail, back);
    }

    #[test]
    fn server_side_tool_usage_skips_none_call_counts() {
        let detail = ServerSideToolUsageDetails {
            tool_name: Some("web_search".into()),
            input_tokens: None,
            output_tokens: None,
            num_sources_used: None,
            code_interpreter_calls: None,
            document_search_calls: None,
            file_search_calls: None,
            mcp_calls: None,
            web_search_calls: None,
            x_search_calls: None,
        };
        let json = serde_json::to_string(&detail).unwrap();
        assert!(!json.contains("\"code_interpreter_calls\""));
        assert!(!json.contains("\"document_search_calls\""));
        assert!(!json.contains("\"file_search_calls\""));
        assert!(!json.contains("\"mcp_calls\""));
        assert!(!json.contains("\"web_search_calls\""));
        assert!(!json.contains("\"x_search_calls\""));
    }

    #[test]
    fn server_side_tool_usage_deserializes_from_wire_with_call_counts() {
        let json = r#"{
            "tool_name": "combined",
            "input_tokens": 50,
            "output_tokens": 100,
            "web_search_calls": 3,
            "code_interpreter_calls": 1
        }"#;
        let detail: ServerSideToolUsageDetails = serde_json::from_str(json).unwrap();
        assert_eq!(detail.web_search_calls, Some(3));
        assert_eq!(detail.code_interpreter_calls, Some(1));
        assert!(detail.file_search_calls.is_none());
        assert!(detail.mcp_calls.is_none());
    }

    // -- cost_in_nano_usd --

    #[test]
    fn responses_usage_cost_in_nano_usd_round_trips() {
        let usage = ResponsesUsage {
            input_tokens: 100,
            output_tokens: 50,
            total_tokens: Some(150),
            output_tokens_details: None,
            cost_in_usd_ticks: None,
            cost_in_nano_usd: Some(42_000_000),
            num_sources_used: None,
            server_side_tool_usage_details: None,
        };
        let json = serde_json::to_string(&usage).unwrap();
        assert!(json.contains("\"cost_in_nano_usd\":42000000"));
        let back: ResponsesUsage = serde_json::from_str(&json).unwrap();
        assert_eq!(usage, back);
    }

    #[test]
    fn unified_usage_cost_in_nano_usd_round_trips() {
        let usage = Usage {
            input_tokens: 100,
            output_tokens: 50,
            reasoning_tokens: None,
            total_tokens: None,
            prompt_tokens_details: None,
            completion_tokens_details: None,
            output_tokens_details: None,
            cost_in_usd_ticks: None,
            cost_in_nano_usd: Some(-500),
            num_sources_used: None,
            server_side_tool_usage_details: None,
        };
        let json = serde_json::to_string(&usage).unwrap();
        assert!(json.contains("\"cost_in_nano_usd\":-500"));
        let back: Usage = serde_json::from_str(&json).unwrap();
        assert_eq!(usage, back);
    }

    #[test]
    fn cost_in_nano_usd_deserializes_from_wire() {
        let json = r#"{"input_tokens":10,"output_tokens":5,"cost_in_nano_usd":123456789}"#;
        let usage: Usage = serde_json::from_str(json).unwrap();
        assert_eq!(usage.cost_in_nano_usd, Some(123_456_789));
    }

    #[test]
    fn cost_in_nano_usd_skipped_when_none() {
        let usage = ResponsesUsage {
            input_tokens: 10,
            output_tokens: 5,
            total_tokens: None,
            output_tokens_details: None,
            cost_in_usd_ticks: None,
            cost_in_nano_usd: None,
            num_sources_used: None,
            server_side_tool_usage_details: None,
        };
        let json = serde_json::to_string(&usage).unwrap();
        assert!(!json.contains("\"cost_in_nano_usd\""));
    }
}
