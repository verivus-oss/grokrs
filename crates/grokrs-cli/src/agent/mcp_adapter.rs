//! MCP tool adapter: wraps MCP-discovered tools as `ErasedTool` implementations.
//!
//! [`McpToolAdapter`] takes a [`McpToolDefinition`] from the MCP server and an
//! `Arc<McpClient>`, implementing the [`ErasedTool`] trait so that MCP tools
//! seamlessly integrate with the `ToolRegistry`, policy engine, and tool loop.
//!
//! Key design decisions:
//! - `classify_json()` always returns `Effect::NetworkConnect` for the MCP server host,
//!   because every MCP tool call is a network operation to the MCP server.
//! - `execute_json()` delegates to `McpClient::call_tool()` using `block_in_place` +
//!   `block_on` to bridge async to sync (same pattern as `ErasedToolWrapper`).
//! - Tool names are prefixed with `mcp_<server_label>_` to avoid collisions with
//!   built-in tools. If no label is set, the prefix is `mcp_`.

use std::sync::Arc;

use grokrs_cap::WorkspaceRoot;
use grokrs_policy::Effect;
use grokrs_tool::erased::ErasedTool;
use grokrs_tool::error::ToolError;

use grokrs_api::mcp::client::McpClient;
use grokrs_api::mcp::types::McpToolDefinition;

/// Wraps a single MCP tool definition as an `ErasedTool` implementation.
///
/// This adapter enables MCP-discovered tools to be registered in the
/// `ToolRegistry` and participate in the standard policy-gated execution flow.
pub struct McpToolAdapter {
    /// The MCP tool definition from the server.
    definition: McpToolDefinition,
    /// Shared MCP client for executing tool calls.
    client: Arc<tokio::sync::RwLock<McpClient>>,
    /// The host of the MCP server (for policy effects).
    server_host: String,
    /// Minimum trust rank required to use this tool.
    min_rank: u8,
    /// Prefixed tool name for the registry (e.g., `mcp_server_tool_name`).
    prefixed_name: String,
}

impl McpToolAdapter {
    /// Create a new adapter for an MCP tool.
    ///
    /// # Arguments
    ///
    /// * `definition` - The tool definition from the MCP server's `tools/list`.
    /// * `client` - The MCP client (shared across all tools from the same server).
    /// * `server_host` - The MCP server hostname (for `NetworkConnect` effects).
    /// * `server_label` - Optional server label for tool name prefixing.
    /// * `min_rank` - Minimum trust rank required (default: 1).
    pub fn new(
        definition: McpToolDefinition,
        client: Arc<tokio::sync::RwLock<McpClient>>,
        server_host: String,
        server_label: Option<&str>,
        min_rank: u8,
    ) -> Self {
        let prefix = match server_label {
            Some(label) => format!("mcp_{}_", sanitize_label(label)),
            None => "mcp_".to_owned(),
        };
        let prefixed_name = format!("{prefix}{}", definition.name);

        Self {
            definition,
            client,
            server_host,
            min_rank,
            prefixed_name,
        }
    }

    /// Return the original (unprefixed) MCP tool name.
    pub fn mcp_name(&self) -> &str {
        &self.definition.name
    }
}

/// Sanitize a server label for use in tool name prefixes.
///
/// Replaces non-alphanumeric characters with underscores and lowercases.
fn sanitize_label(label: &str) -> String {
    label
        .chars()
        .map(|c| {
            if c.is_alphanumeric() {
                c.to_ascii_lowercase()
            } else {
                '_'
            }
        })
        .collect()
}

impl ErasedTool for McpToolAdapter {
    fn name(&self) -> &str {
        &self.prefixed_name
    }

    fn description(&self) -> &str {
        self.definition
            .description
            .as_deref()
            .unwrap_or("MCP tool (no description)")
    }

    fn input_schema(&self) -> serde_json::Value {
        self.definition.input_schema.clone()
    }

    fn min_trust_rank(&self) -> u8 {
        self.min_rank
    }

    fn classify_json(&self, _input_json: &str) -> Result<Vec<Effect>, ToolError> {
        // Every MCP tool call is a network operation to the MCP server.
        Ok(vec![Effect::NetworkConnect {
            host: self.server_host.clone(),
        }])
    }

    fn execute_json(&self, input_json: &str, _root: &WorkspaceRoot) -> Result<String, ToolError> {
        // Parse the input JSON to pass as arguments to the MCP tool.
        let arguments: Option<serde_json::Value> = if input_json.is_empty() || input_json == "{}" {
            None
        } else {
            Some(serde_json::from_str(input_json).map_err(|e| {
                ToolError::Other(format!(
                    "failed to parse input for MCP tool '{}': {e}",
                    self.prefixed_name
                ))
            })?)
        };

        let mcp_name = self.definition.name.clone();
        let client = self.client.clone();

        // Bridge async to sync using the same pattern as ErasedToolWrapper.
        let result = tokio::task::block_in_place(|| {
            tokio::runtime::Handle::current().block_on(async {
                let guard = client.read().await;
                guard.call_tool(&mcp_name, arguments).await
            })
        });

        match result {
            Ok(tool_result) => {
                if tool_result.is_error {
                    // The MCP tool reported an error — return it as a structured
                    // error message so the model can adapt.
                    let error_text = tool_result.text();
                    Ok(serde_json::json!({
                        "error": true,
                        "message": error_text
                    })
                    .to_string())
                } else {
                    // Return the text content as a JSON string for consistency
                    // with built-in tool outputs.
                    let text = tool_result.text();
                    serde_json::to_string(&text).map_err(|e| {
                        ToolError::Other(format!(
                            "failed to serialize MCP tool '{}' output: {e}",
                            self.prefixed_name
                        ))
                    })
                }
            }
            Err(e) => Err(ToolError::Other(format!(
                "MCP tool '{}' call failed: {e}",
                self.prefixed_name
            ))),
        }
    }
}

impl std::fmt::Debug for McpToolAdapter {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("McpToolAdapter")
            .field("prefixed_name", &self.prefixed_name)
            .field("mcp_name", &self.definition.name)
            .field("server_host", &self.server_host)
            .field("min_rank", &self.min_rank)
            .finish()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use grokrs_api::mcp::types::McpToolDefinition;
    use serde_json::json;

    fn make_definition(name: &str, desc: Option<&str>) -> McpToolDefinition {
        McpToolDefinition {
            name: name.to_owned(),
            description: desc.map(|s| s.to_owned()),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "path": { "type": "string" }
                },
                "required": ["path"]
            }),
        }
    }

    fn make_client() -> Arc<tokio::sync::RwLock<McpClient>> {
        // We can't actually connect for unit tests, but we can create the client
        // struct with a valid URL (tests won't call execute_json).
        Arc::new(tokio::sync::RwLock::new(
            McpClient::new("http://localhost:9999").unwrap(),
        ))
    }

    // -----------------------------------------------------------------------
    // Name prefixing
    // -----------------------------------------------------------------------

    #[test]
    fn prefixed_name_with_label() {
        let adapter = McpToolAdapter::new(
            make_definition("read_file", Some("Read a file")),
            make_client(),
            "localhost".into(),
            Some("my-server"),
            1,
        );
        assert_eq!(adapter.name(), "mcp_my_server_read_file");
    }

    #[test]
    fn prefixed_name_without_label() {
        let adapter = McpToolAdapter::new(
            make_definition("ping", None),
            make_client(),
            "localhost".into(),
            None,
            1,
        );
        assert_eq!(adapter.name(), "mcp_ping");
    }

    #[test]
    fn mcp_name_returns_original() {
        let adapter = McpToolAdapter::new(
            make_definition("test_tool", None),
            make_client(),
            "localhost".into(),
            Some("srv"),
            1,
        );
        assert_eq!(adapter.mcp_name(), "test_tool");
    }

    // -----------------------------------------------------------------------
    // Label sanitization
    // -----------------------------------------------------------------------

    #[test]
    fn sanitize_label_basic() {
        assert_eq!(sanitize_label("my-server"), "my_server");
        assert_eq!(sanitize_label("My Server"), "my_server");
        assert_eq!(sanitize_label("server.local"), "server_local");
        assert_eq!(sanitize_label("abc123"), "abc123");
    }

    // -----------------------------------------------------------------------
    // Description
    // -----------------------------------------------------------------------

    #[test]
    fn description_from_definition() {
        let adapter = McpToolAdapter::new(
            make_definition("tool", Some("A useful tool")),
            make_client(),
            "localhost".into(),
            None,
            1,
        );
        assert_eq!(adapter.description(), "A useful tool");
    }

    #[test]
    fn description_fallback_when_none() {
        let adapter = McpToolAdapter::new(
            make_definition("tool", None),
            make_client(),
            "localhost".into(),
            None,
            1,
        );
        assert_eq!(adapter.description(), "MCP tool (no description)");
    }

    // -----------------------------------------------------------------------
    // Input schema
    // -----------------------------------------------------------------------

    #[test]
    fn input_schema_from_definition() {
        let adapter = McpToolAdapter::new(
            make_definition("tool", None),
            make_client(),
            "localhost".into(),
            None,
            1,
        );
        let schema = adapter.input_schema();
        assert_eq!(schema["type"], "object");
        assert!(schema["properties"]["path"].is_object());
    }

    // -----------------------------------------------------------------------
    // Trust rank
    // -----------------------------------------------------------------------

    #[test]
    fn min_trust_rank_configurable() {
        let adapter_0 = McpToolAdapter::new(
            make_definition("tool", None),
            make_client(),
            "localhost".into(),
            None,
            0,
        );
        assert_eq!(adapter_0.min_trust_rank(), 0);

        let adapter_2 = McpToolAdapter::new(
            make_definition("tool", None),
            make_client(),
            "localhost".into(),
            None,
            2,
        );
        assert_eq!(adapter_2.min_trust_rank(), 2);
    }

    // -----------------------------------------------------------------------
    // Classify
    // -----------------------------------------------------------------------

    #[test]
    fn classify_returns_network_connect() {
        let adapter = McpToolAdapter::new(
            make_definition("tool", None),
            make_client(),
            "tools.example.com".into(),
            None,
            1,
        );
        let effects = adapter.classify_json(r#"{"path":"test.txt"}"#).unwrap();
        assert_eq!(effects.len(), 1);
        assert!(matches!(
            &effects[0],
            Effect::NetworkConnect { host } if host == "tools.example.com"
        ));
    }

    #[test]
    fn classify_empty_input_returns_network_connect() {
        let adapter = McpToolAdapter::new(
            make_definition("ping", None),
            make_client(),
            "localhost".into(),
            None,
            1,
        );
        let effects = adapter.classify_json("{}").unwrap();
        assert_eq!(effects.len(), 1);
    }

    // -----------------------------------------------------------------------
    // Debug format
    // -----------------------------------------------------------------------

    #[test]
    fn debug_format() {
        let adapter = McpToolAdapter::new(
            make_definition("tool", None),
            make_client(),
            "localhost".into(),
            Some("test"),
            1,
        );
        let debug = format!("{adapter:?}");
        assert!(debug.contains("McpToolAdapter"));
        assert!(debug.contains("mcp_test_tool"));
    }
}
