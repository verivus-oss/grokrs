//! Built-in server-side tools for the xAI Grok API.
//!
//! These represent tools that run server-side (web search, code execution, etc.)
//! rather than caller-implemented function tools. Each variant serializes as a
//! typed JSON object, e.g., `{"type": "web_search"}`.

use serde::{Deserialize, Serialize};

use super::mcp_tool::McpToolDefinition;

// ---------------------------------------------------------------------------
// BuiltinTool
// ---------------------------------------------------------------------------

/// A server-side built-in tool that the xAI API can execute.
///
/// These tools are provided as part of the `tools` array in a request, alongside
/// any function tool definitions. The model decides when to invoke them, and the
/// results are returned directly in the response output.
///
/// # Serialization
///
/// Simple variants serialize as `{"type": "<snake_case_name>"}`. The `Mcp`
/// variant flattens `McpToolDefinition` fields alongside the `"type": "mcp"` tag.
///
/// # Examples
///
/// ```
/// use grokrs_api::types::builtin_tools::BuiltinTool;
///
/// let tool = BuiltinTool::WebSearch;
/// let json = serde_json::to_string(&tool).unwrap();
/// assert_eq!(json, r#"{"type":"web_search"}"#);
/// ```
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum BuiltinTool {
    /// Web search — searches the public internet.
    WebSearch,
    /// X (Twitter) search — searches posts on X.
    XSearch,
    /// Code execution — runs code in a sandboxed environment.
    CodeExecution,
    /// Code interpreter — runs and interprets code with output.
    CodeInterpreter,
    /// Collections search — searches user-defined collections.
    CollectionsSearch,
    /// File search — searches uploaded files.
    FileSearch,
    /// Attachment search — searches message attachments.
    AttachmentSearch,
    /// Remote MCP (Model Context Protocol) tool server.
    ///
    /// Allows the model to connect to an external MCP server during the
    /// request. The xAI server establishes the MCP connection; the client
    /// does NOT connect to the MCP server directly.
    #[serde(rename = "mcp")]
    Mcp(McpToolDefinition),
}

impl BuiltinTool {
    /// Return the wire-format type name for this built-in tool.
    #[must_use]
    pub fn type_name(&self) -> &'static str {
        match self {
            BuiltinTool::WebSearch => "web_search",
            BuiltinTool::XSearch => "x_search",
            BuiltinTool::CodeExecution => "code_execution",
            BuiltinTool::CodeInterpreter => "code_interpreter",
            BuiltinTool::CollectionsSearch => "collections_search",
            BuiltinTool::FileSearch => "file_search",
            BuiltinTool::AttachmentSearch => "attachment_search",
            BuiltinTool::Mcp(_) => "mcp",
        }
    }

    /// Convert to a `serde_json::Value` suitable for use in the `tools` array.
    ///
    /// # Panics
    ///
    /// Panics if the value cannot be serialized to JSON. This is infallible
    /// for the known enum variants and indicates a programming error.
    #[must_use]
    pub fn to_value(&self) -> serde_json::Value {
        serde_json::to_value(self).expect("BuiltinTool serialization is infallible")
    }
}

// ---------------------------------------------------------------------------
// SearchParameters
// ---------------------------------------------------------------------------

/// Parameters for controlling built-in search tool behaviour.
///
/// These parameters apply to `web_search` and `x_search` tools and are passed
/// at the top level of the request as `search_parameters`.
///
/// # Examples
///
/// ```
/// use grokrs_api::types::builtin_tools::{SearchParameters, SearchMode};
///
/// let params = SearchParameters {
///     mode: Some(SearchMode::Auto),
///     sources: None,
///     from_date: Some("2025-01-01".into()),
///     to_date: None,
///     max_search_results: Some(5),
///     return_citations: Some(true),
/// };
/// let json = serde_json::to_string(&params).unwrap();
/// assert!(json.contains("\"mode\":\"auto\""));
/// assert!(json.contains("\"from_date\":\"2025-01-01\""));
/// ```
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SearchParameters {
    /// The search mode determining whether search is performed.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub mode: Option<SearchMode>,

    /// Specific sources to search (provider-specific format).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sources: Option<Vec<serde_json::Value>>,

    /// The earliest date for search results (e.g., "2025-01-01").
    #[serde(skip_serializing_if = "Option::is_none")]
    pub from_date: Option<String>,

    /// The latest date for search results (e.g., "2025-12-31").
    #[serde(skip_serializing_if = "Option::is_none")]
    pub to_date: Option<String>,

    /// Maximum number of search results to return.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_search_results: Option<u32>,

    /// Whether to include citation information in the response.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub return_citations: Option<bool>,
}

impl SearchParameters {
    /// Convert to a `serde_json::Value` suitable for use in
    /// `CreateResponseRequest::search_parameters`.
    ///
    /// # Panics
    ///
    /// Panics if the value cannot be serialized to JSON. This is infallible
    /// for the known struct layout and indicates a programming error.
    #[must_use]
    pub fn to_value(&self) -> serde_json::Value {
        serde_json::to_value(self).expect("SearchParameters serialization is infallible")
    }
}

// ---------------------------------------------------------------------------
// SearchMode
// ---------------------------------------------------------------------------

/// Controls whether the model performs a search.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum SearchMode {
    /// Let the model decide whether to search.
    Auto,
    /// Always perform a search.
    On,
    /// Never perform a search.
    Off,
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::mcp_tool::{McpAuthorization, McpToolDefinition};
    use std::collections::HashMap;

    // -- BuiltinTool serialization --

    #[test]
    fn builtin_tool_web_search_serializes() {
        let json = serde_json::to_string(&BuiltinTool::WebSearch).unwrap();
        assert_eq!(json, r#"{"type":"web_search"}"#);
    }

    #[test]
    fn builtin_tool_x_search_serializes() {
        let json = serde_json::to_string(&BuiltinTool::XSearch).unwrap();
        assert_eq!(json, r#"{"type":"x_search"}"#);
    }

    #[test]
    fn builtin_tool_code_execution_serializes() {
        let json = serde_json::to_string(&BuiltinTool::CodeExecution).unwrap();
        assert_eq!(json, r#"{"type":"code_execution"}"#);
    }

    #[test]
    fn builtin_tool_code_interpreter_serializes() {
        let json = serde_json::to_string(&BuiltinTool::CodeInterpreter).unwrap();
        assert_eq!(json, r#"{"type":"code_interpreter"}"#);
    }

    #[test]
    fn builtin_tool_collections_search_serializes() {
        let json = serde_json::to_string(&BuiltinTool::CollectionsSearch).unwrap();
        assert_eq!(json, r#"{"type":"collections_search"}"#);
    }

    #[test]
    fn builtin_tool_file_search_serializes() {
        let json = serde_json::to_string(&BuiltinTool::FileSearch).unwrap();
        assert_eq!(json, r#"{"type":"file_search"}"#);
    }

    #[test]
    fn builtin_tool_attachment_search_serializes() {
        let json = serde_json::to_string(&BuiltinTool::AttachmentSearch).unwrap();
        assert_eq!(json, r#"{"type":"attachment_search"}"#);
    }

    #[test]
    fn builtin_tool_all_variants_round_trip() {
        let simple_variants = [
            BuiltinTool::WebSearch,
            BuiltinTool::XSearch,
            BuiltinTool::CodeExecution,
            BuiltinTool::CodeInterpreter,
            BuiltinTool::CollectionsSearch,
            BuiltinTool::FileSearch,
            BuiltinTool::AttachmentSearch,
        ];
        for variant in &simple_variants {
            let json = serde_json::to_string(variant).unwrap();
            let back: BuiltinTool = serde_json::from_str(&json).unwrap();
            assert_eq!(variant, &back);
        }

        // MCP variant with data
        let mcp = BuiltinTool::Mcp(McpToolDefinition {
            server_url: "https://mcp.example.com/sse".into(),
            server_label: Some("test".into()),
            server_description: None,
            allowed_tools: None,
            authorization: None,
            headers: None,
        });
        let json = serde_json::to_string(&mcp).unwrap();
        let back: BuiltinTool = serde_json::from_str(&json).unwrap();
        assert_eq!(mcp, back);
    }

    #[test]
    fn builtin_tool_type_name() {
        assert_eq!(BuiltinTool::WebSearch.type_name(), "web_search");
        assert_eq!(BuiltinTool::XSearch.type_name(), "x_search");
        assert_eq!(BuiltinTool::CodeExecution.type_name(), "code_execution");
        assert_eq!(BuiltinTool::CodeInterpreter.type_name(), "code_interpreter");
        assert_eq!(
            BuiltinTool::CollectionsSearch.type_name(),
            "collections_search"
        );
        assert_eq!(BuiltinTool::FileSearch.type_name(), "file_search");
        assert_eq!(
            BuiltinTool::AttachmentSearch.type_name(),
            "attachment_search"
        );
        let mcp = BuiltinTool::Mcp(McpToolDefinition {
            server_url: "https://mcp.example.com".into(),
            server_label: None,
            server_description: None,
            allowed_tools: None,
            authorization: None,
            headers: None,
        });
        assert_eq!(mcp.type_name(), "mcp");
    }

    #[test]
    fn builtin_tool_to_value() {
        let val = BuiltinTool::WebSearch.to_value();
        assert_eq!(val["type"], "web_search");
    }

    #[test]
    fn builtin_tool_deserializes_from_wire() {
        let json = r#"{"type":"code_interpreter"}"#;
        let tool: BuiltinTool = serde_json::from_str(json).unwrap();
        assert_eq!(tool, BuiltinTool::CodeInterpreter);
    }

    // -- BuiltinTool::Mcp serialization --

    #[test]
    fn builtin_tool_mcp_serializes_with_type_tag() {
        let tool = BuiltinTool::Mcp(McpToolDefinition {
            server_url: "https://mcp.example.com/sse".into(),
            server_label: None,
            server_description: None,
            allowed_tools: None,
            authorization: None,
            headers: None,
        });
        let val = serde_json::to_value(&tool).unwrap();
        assert_eq!(val["type"], "mcp");
        assert_eq!(val["server_url"], "https://mcp.example.com/sse");
    }

    #[test]
    fn builtin_tool_mcp_full_round_trips() {
        let mut headers = HashMap::new();
        headers.insert("X-Custom".into(), "value".into());

        let tool = BuiltinTool::Mcp(McpToolDefinition {
            server_url: "https://mcp.example.com/sse".into(),
            server_label: Some("my-server".into()),
            server_description: Some("A great MCP server".into()),
            allowed_tools: Some(vec!["tool_a".into(), "tool_b".into()]),
            authorization: Some(McpAuthorization::ApiKey {
                api_key: "sk-key".into(),
            }),
            headers: Some(headers),
        });

        let json = serde_json::to_string(&tool).unwrap();
        let back: BuiltinTool = serde_json::from_str(&json).unwrap();
        assert_eq!(tool, back);
    }

    #[test]
    fn builtin_tool_mcp_deserializes_from_wire() {
        let json = r#"{
            "type": "mcp",
            "server_url": "https://mcp.prod.example.com/sse",
            "server_label": "prod",
            "allowed_tools": ["search"],
            "authorization": {"type": "api_key", "api_key": "sk-123"}
        }"#;
        let tool: BuiltinTool = serde_json::from_str(json).unwrap();
        match &tool {
            BuiltinTool::Mcp(def) => {
                assert_eq!(def.server_url, "https://mcp.prod.example.com/sse");
                assert_eq!(def.server_label.as_deref(), Some("prod"));
                assert_eq!(def.allowed_tools.as_ref().unwrap(), &["search"]);
            }
            other => panic!("expected Mcp, got: {other:?}"),
        }
    }

    #[test]
    fn builtin_tool_mcp_to_value() {
        let tool = BuiltinTool::Mcp(McpToolDefinition {
            server_url: "https://mcp.example.com/sse".into(),
            server_label: None,
            server_description: None,
            allowed_tools: None,
            authorization: None,
            headers: None,
        });
        let val = tool.to_value();
        assert_eq!(val["type"], "mcp");
        assert_eq!(val["server_url"], "https://mcp.example.com/sse");
    }

    #[test]
    fn builtin_tool_mcp_coexists_with_simple_variants_in_tools_array() {
        let tools = vec![
            BuiltinTool::WebSearch,
            BuiltinTool::Mcp(McpToolDefinition {
                server_url: "https://mcp.example.com/sse".into(),
                server_label: Some("test".into()),
                server_description: None,
                allowed_tools: None,
                authorization: None,
                headers: None,
            }),
            BuiltinTool::CodeInterpreter,
        ];

        let json = serde_json::to_string(&tools).unwrap();
        let back: Vec<BuiltinTool> = serde_json::from_str(&json).unwrap();
        assert_eq!(tools, back);
        assert_eq!(back.len(), 3);
    }

    // -- SearchParameters --

    #[test]
    fn search_parameters_full_round_trips() {
        let params = SearchParameters {
            mode: Some(SearchMode::Auto),
            sources: Some(vec![serde_json::json!({"type": "web"})]),
            from_date: Some("2025-01-01".into()),
            to_date: Some("2025-12-31".into()),
            max_search_results: Some(10),
            return_citations: Some(true),
        };
        let json = serde_json::to_string(&params).unwrap();
        let back: SearchParameters = serde_json::from_str(&json).unwrap();
        assert_eq!(params, back);
    }

    #[test]
    fn search_parameters_skips_none_fields() {
        let params = SearchParameters {
            mode: None,
            sources: None,
            from_date: None,
            to_date: None,
            max_search_results: None,
            return_citations: None,
        };
        let json = serde_json::to_string(&params).unwrap();
        assert_eq!(json, "{}");
    }

    #[test]
    fn search_parameters_partial() {
        let params = SearchParameters {
            mode: Some(SearchMode::On),
            sources: None,
            from_date: Some("2025-06-01".into()),
            to_date: None,
            max_search_results: Some(5),
            return_citations: None,
        };
        let json = serde_json::to_string(&params).unwrap();
        assert!(json.contains("\"mode\":\"on\""));
        assert!(json.contains("\"from_date\":\"2025-06-01\""));
        assert!(json.contains("\"max_search_results\":5"));
        assert!(!json.contains("\"sources\""));
        assert!(!json.contains("\"to_date\""));
        assert!(!json.contains("\"return_citations\""));
    }

    #[test]
    fn search_parameters_to_value() {
        let params = SearchParameters {
            mode: Some(SearchMode::Off),
            sources: None,
            from_date: None,
            to_date: None,
            max_search_results: None,
            return_citations: None,
        };
        let val = params.to_value();
        assert_eq!(val["mode"], "off");
    }

    // -- SearchMode --

    #[test]
    fn search_mode_serializes_lowercase() {
        assert_eq!(
            serde_json::to_string(&SearchMode::Auto).unwrap(),
            "\"auto\""
        );
        assert_eq!(serde_json::to_string(&SearchMode::On).unwrap(), "\"on\"");
        assert_eq!(serde_json::to_string(&SearchMode::Off).unwrap(), "\"off\"");
    }

    #[test]
    fn search_mode_round_trips() {
        for mode in [SearchMode::Auto, SearchMode::On, SearchMode::Off] {
            let json = serde_json::to_string(&mode).unwrap();
            let back: SearchMode = serde_json::from_str(&json).unwrap();
            assert_eq!(mode, back);
        }
    }
}
