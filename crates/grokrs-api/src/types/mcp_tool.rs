//! MCP (Model Context Protocol) remote tool definitions.
//!
//! An MCP tool allows the model to connect to external MCP servers during a
//! Responses API request. The xAI server handles the actual MCP connection;
//! the client merely declares which MCP servers the model may use via the
//! `tools` array.
//!
//! # Wire format
//!
//! ```json
//! {
//!   "type": "mcp",
//!   "server_url": "https://mcp.example.com/sse",
//!   "server_label": "my-server",
//!   "allowed_tools": ["tool_a", "tool_b"],
//!   "authorization": {
//!     "type": "api_key",
//!     "api_key": "sk-..."
//!   },
//!   "headers": {
//!     "X-Custom": "value"
//!   }
//! }
//! ```

use std::collections::HashMap;

use serde::{Deserialize, Serialize};

// ---------------------------------------------------------------------------
// McpAuthorization
// ---------------------------------------------------------------------------

/// Authorization configuration for an MCP server connection.
///
/// Currently only the `api_key` type is supported. The `type` field is always
/// serialized as `"api_key"` to match the wire format.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum McpAuthorization {
    /// API key authorization — the server sends this key in requests to the
    /// MCP server.
    ApiKey {
        /// The API key value.
        api_key: String,
    },
}

// ---------------------------------------------------------------------------
// McpToolDefinition
// ---------------------------------------------------------------------------

/// Definition of a remote MCP tool server that the model may connect to.
///
/// The xAI server establishes the MCP connection; the client does NOT connect
/// to the MCP server directly. This struct declares the server location,
/// optional access restrictions, and authentication details.
///
/// # Examples
///
/// ```
/// use grokrs_api::types::mcp_tool::McpToolDefinition;
///
/// let tool = McpToolDefinition {
///     server_url: "https://mcp.example.com/sse".into(),
///     server_label: Some("my-mcp-server".into()),
///     server_description: None,
///     allowed_tools: Some(vec!["tool_a".into(), "tool_b".into()]),
///     authorization: None,
///     headers: None,
/// };
/// let json = serde_json::to_value(&tool).unwrap();
/// assert_eq!(json["server_url"], "https://mcp.example.com/sse");
/// ```
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct McpToolDefinition {
    /// The URL of the MCP server to connect to (required).
    pub server_url: String,

    /// A human-readable label for this MCP server (optional).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub server_label: Option<String>,

    /// A human-readable description of what this MCP server provides (optional).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub server_description: Option<String>,

    /// Restrict which tools on the MCP server are exposed to the model (optional).
    ///
    /// When `None`, all tools reported by the server are available.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub allowed_tools: Option<Vec<String>>,

    /// Authorization credentials for connecting to the MCP server (optional).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub authorization: Option<McpAuthorization>,

    /// Custom HTTP headers to send when connecting to the MCP server (optional).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub headers: Option<HashMap<String, String>>,
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    // -- McpAuthorization --

    #[test]
    fn mcp_authorization_api_key_round_trips() {
        let auth = McpAuthorization::ApiKey {
            api_key: "sk-test-key-123".into(),
        };
        let json = serde_json::to_string(&auth).unwrap();
        assert!(json.contains(r#""type":"api_key""#));
        assert!(json.contains(r#""api_key":"sk-test-key-123""#));

        let back: McpAuthorization = serde_json::from_str(&json).unwrap();
        assert_eq!(auth, back);
    }

    #[test]
    fn mcp_authorization_deserializes_from_wire() {
        let json = r#"{"type":"api_key","api_key":"sk-wire-key"}"#;
        let auth: McpAuthorization = serde_json::from_str(json).unwrap();
        match &auth {
            McpAuthorization::ApiKey { api_key } => {
                assert_eq!(api_key, "sk-wire-key");
            }
        }
    }

    // -- McpToolDefinition --

    #[test]
    fn mcp_tool_definition_full_round_trips() {
        let mut headers = HashMap::new();
        headers.insert("X-Custom-Header".into(), "custom-value".into());
        headers.insert("Authorization-Extra".into(), "bearer-extra".into());

        let tool = McpToolDefinition {
            server_url: "https://mcp.example.com/sse".into(),
            server_label: Some("my-mcp-server".into()),
            server_description: Some("A test MCP server".into()),
            allowed_tools: Some(vec!["tool_a".into(), "tool_b".into()]),
            authorization: Some(McpAuthorization::ApiKey {
                api_key: "sk-key-abc".into(),
            }),
            headers: Some(headers),
        };

        let json = serde_json::to_string(&tool).unwrap();
        let back: McpToolDefinition = serde_json::from_str(&json).unwrap();
        assert_eq!(tool, back);
    }

    #[test]
    fn mcp_tool_definition_minimal_round_trips() {
        let tool = McpToolDefinition {
            server_url: "https://mcp.example.com/sse".into(),
            server_label: None,
            server_description: None,
            allowed_tools: None,
            authorization: None,
            headers: None,
        };

        let json = serde_json::to_string(&tool).unwrap();
        let back: McpToolDefinition = serde_json::from_str(&json).unwrap();
        assert_eq!(tool, back);

        // Verify optional fields are not present in JSON
        let val: serde_json::Value = serde_json::from_str(&json).unwrap();
        assert!(val.get("server_label").is_none());
        assert!(val.get("server_description").is_none());
        assert!(val.get("allowed_tools").is_none());
        assert!(val.get("authorization").is_none());
        assert!(val.get("headers").is_none());
    }

    #[test]
    fn mcp_tool_definition_deserializes_from_wire() {
        let json = r#"{
            "server_url": "https://mcp.prod.example.com/sse",
            "server_label": "prod-server",
            "allowed_tools": ["search", "fetch"],
            "authorization": {
                "type": "api_key",
                "api_key": "sk-prod-key"
            }
        }"#;
        let tool: McpToolDefinition = serde_json::from_str(json).unwrap();
        assert_eq!(tool.server_url, "https://mcp.prod.example.com/sse");
        assert_eq!(tool.server_label.as_deref(), Some("prod-server"));
        assert!(tool.server_description.is_none());
        assert_eq!(tool.allowed_tools.as_ref().unwrap(), &["search", "fetch"]);
        match tool.authorization.as_ref().unwrap() {
            McpAuthorization::ApiKey { api_key } => {
                assert_eq!(api_key, "sk-prod-key");
            }
        }
        assert!(tool.headers.is_none());
    }

    #[test]
    fn mcp_tool_definition_with_headers_only() {
        let json = r#"{
            "server_url": "https://mcp.example.com/sse",
            "headers": {
                "X-Api-Version": "2024-01",
                "X-Tenant-Id": "tenant-42"
            }
        }"#;
        let tool: McpToolDefinition = serde_json::from_str(json).unwrap();
        let headers = tool.headers.as_ref().unwrap();
        assert_eq!(headers.len(), 2);
        assert_eq!(headers["X-Api-Version"], "2024-01");
        assert_eq!(headers["X-Tenant-Id"], "tenant-42");
    }

    #[test]
    fn mcp_tool_definition_empty_allowed_tools() {
        let tool = McpToolDefinition {
            server_url: "https://mcp.example.com/sse".into(),
            server_label: None,
            server_description: None,
            allowed_tools: Some(vec![]),
            authorization: None,
            headers: None,
        };

        let json = serde_json::to_string(&tool).unwrap();
        let back: McpToolDefinition = serde_json::from_str(&json).unwrap();
        assert_eq!(tool, back);
        assert!(back.allowed_tools.unwrap().is_empty());
    }
}
