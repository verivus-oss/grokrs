//! MCP protocol types: JSON-RPC 2.0 messages, tool definitions, and tool results.
//!
//! Implements the wire types for the Model Context Protocol (2025-03-26 spec).
//! All types derive `Serialize` and `Deserialize` for JSON round-tripping.

use serde::{Deserialize, Serialize};

// ---------------------------------------------------------------------------
// JSON-RPC 2.0
// ---------------------------------------------------------------------------

/// A JSON-RPC 2.0 request.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct JsonRpcRequest {
    pub jsonrpc: String,
    pub id: JsonRpcId,
    pub method: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub params: Option<serde_json::Value>,
}

impl JsonRpcRequest {
    /// Create a new JSON-RPC 2.0 request with the given method and params.
    pub fn new(id: u64, method: impl Into<String>, params: Option<serde_json::Value>) -> Self {
        Self {
            jsonrpc: "2.0".to_owned(),
            id: JsonRpcId::Number(id),
            method: method.into(),
            params,
        }
    }
}

/// JSON-RPC 2.0 request/response identifier.
///
/// Can be a number or a string per the JSON-RPC spec.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Hash)]
#[serde(untagged)]
pub enum JsonRpcId {
    Number(u64),
    Str(String),
}

/// A JSON-RPC 2.0 response.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct JsonRpcResponse {
    pub jsonrpc: String,
    pub id: JsonRpcId,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub result: Option<serde_json::Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<JsonRpcError>,
}

impl JsonRpcResponse {
    /// Returns `Ok(result)` if the response is successful, or `Err(error)` if it contains an error.
    ///
    /// # Errors
    ///
    /// Returns the [`JsonRpcError`] if the response contains an error field.
    pub fn into_result(self) -> Result<serde_json::Value, JsonRpcError> {
        if let Some(err) = self.error {
            Err(err)
        } else {
            Ok(self.result.unwrap_or(serde_json::Value::Null))
        }
    }
}

/// A JSON-RPC 2.0 error object.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct JsonRpcError {
    pub code: i64,
    pub message: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub data: Option<serde_json::Value>,
}

impl std::fmt::Display for JsonRpcError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "JSON-RPC error {}: {}", self.code, self.message)?;
        if let Some(ref data) = self.data {
            write!(f, " (data: {data})")?;
        }
        Ok(())
    }
}

impl std::error::Error for JsonRpcError {}

// ---------------------------------------------------------------------------
// MCP Initialize
// ---------------------------------------------------------------------------

/// Parameters for the `initialize` JSON-RPC method.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct InitializeParams {
    pub protocol_version: String,
    pub capabilities: ClientCapabilities,
    pub client_info: ClientInfo,
}

/// Client capabilities declared during the `initialize` handshake.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct ClientCapabilities {
    /// Reserved for future capability declarations. Currently empty.
    #[serde(flatten)]
    pub extra: serde_json::Map<String, serde_json::Value>,
}

/// Client identity sent during the `initialize` handshake.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ClientInfo {
    pub name: String,
    pub version: String,
}

/// Result of the `initialize` JSON-RPC method.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct InitializeResult {
    pub protocol_version: String,
    pub capabilities: ServerCapabilities,
    pub server_info: ServerInfo,
}

/// Server capabilities returned from the `initialize` handshake.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct ServerCapabilities {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tools: Option<ToolsCapability>,
    /// Additional capabilities we don't specifically model.
    #[serde(flatten)]
    pub extra: serde_json::Map<String, serde_json::Value>,
}

/// The `tools` capability advertised by the server.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct ToolsCapability {
    /// Whether the server supports `notifications/tools/list_changed`.
    #[serde(default)]
    pub list_changed: bool,
}

/// Server identity returned from the `initialize` handshake.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ServerInfo {
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub version: Option<String>,
}

// ---------------------------------------------------------------------------
// MCP Tools
// ---------------------------------------------------------------------------

/// Result of `tools/list` — a list of available tool definitions.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct ToolListResult {
    pub tools: Vec<McpToolDefinition>,
    /// Pagination cursor for the next page (if any).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub next_cursor: Option<String>,
}

/// A single tool definition as returned by an MCP server.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct McpToolDefinition {
    /// Machine-readable tool name.
    pub name: String,
    /// Human-readable description.
    #[serde(default)]
    pub description: Option<String>,
    /// JSON Schema for the tool's input.
    #[serde(default = "default_input_schema")]
    pub input_schema: serde_json::Value,
}

fn default_input_schema() -> serde_json::Value {
    serde_json::json!({ "type": "object", "properties": {} })
}

/// Parameters for `tools/call`.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ToolCallParams {
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub arguments: Option<serde_json::Value>,
}

/// Result of `tools/call`.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct ToolCallResult {
    pub content: Vec<ToolContent>,
    /// Indicates whether the tool call failed. Default: false.
    #[serde(default)]
    pub is_error: bool,
}

impl ToolCallResult {
    /// Concatenate all text content blocks into a single string.
    #[must_use]
    pub fn text(&self) -> String {
        let mut result = String::new();
        for item in &self.content {
            match item {
                ToolContent::Text { text } => {
                    if !result.is_empty() {
                        result.push('\n');
                    }
                    result.push_str(text);
                }
                ToolContent::Image { .. } | ToolContent::Resource { .. } => {
                    // Non-text content is not included in the text representation.
                }
            }
        }
        result
    }
}

/// A content block within a tool result.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ToolContent {
    Text {
        text: String,
    },
    Image {
        data: String,
        #[serde(rename = "mimeType")]
        mime_type: String,
    },
    Resource {
        resource: serde_json::Value,
    },
}

// ---------------------------------------------------------------------------
// MCP Error Codes
// ---------------------------------------------------------------------------

/// Well-known JSON-RPC error codes from the MCP spec.
pub mod error_codes {
    /// Parse error: invalid JSON was received.
    pub const PARSE_ERROR: i64 = -32700;
    /// Invalid request: the JSON sent is not a valid request object.
    pub const INVALID_REQUEST: i64 = -32600;
    /// Method not found: the method does not exist.
    pub const METHOD_NOT_FOUND: i64 = -32601;
    /// Invalid params: invalid method parameters.
    pub const INVALID_PARAMS: i64 = -32602;
    /// Internal error: internal JSON-RPC error.
    pub const INTERNAL_ERROR: i64 = -32603;
}

// ---------------------------------------------------------------------------
// MCP Protocol Version
// ---------------------------------------------------------------------------

/// The MCP protocol version this client implements.
pub const PROTOCOL_VERSION: &str = "2025-03-26";

/// Client name sent during initialization.
pub const CLIENT_NAME: &str = "grokrs";

/// Client version sent during initialization.
pub const CLIENT_VERSION: &str = env!("CARGO_PKG_VERSION");

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    // -----------------------------------------------------------------------
    // JSON-RPC Request serialization
    // -----------------------------------------------------------------------

    #[test]
    fn json_rpc_request_serializes_correctly() {
        let req = JsonRpcRequest::new(1, "tools/list", None);
        let json = serde_json::to_value(&req).unwrap();
        assert_eq!(json["jsonrpc"], "2.0");
        assert_eq!(json["id"], 1);
        assert_eq!(json["method"], "tools/list");
        assert!(json.get("params").is_none());
    }

    #[test]
    fn json_rpc_request_with_params_serializes_correctly() {
        let params = json!({ "name": "read_file", "arguments": { "path": "test.txt" } });
        let req = JsonRpcRequest::new(42, "tools/call", Some(params.clone()));
        let json = serde_json::to_value(&req).unwrap();
        assert_eq!(json["params"], params);
    }

    #[test]
    fn json_rpc_request_round_trips() {
        let req = JsonRpcRequest::new(
            99,
            "initialize",
            Some(json!({"protocolVersion": "2025-03-26"})),
        );
        let serialized = serde_json::to_string(&req).unwrap();
        let deserialized: JsonRpcRequest = serde_json::from_str(&serialized).unwrap();
        assert_eq!(req, deserialized);
    }

    // -----------------------------------------------------------------------
    // JSON-RPC Response deserialization
    // -----------------------------------------------------------------------

    #[test]
    fn json_rpc_response_success_deserializes() {
        let raw = r#"{"jsonrpc":"2.0","id":1,"result":{"tools":[]}}"#;
        let resp: JsonRpcResponse = serde_json::from_str(raw).unwrap();
        assert_eq!(resp.id, JsonRpcId::Number(1));
        assert!(resp.error.is_none());
        assert!(resp.result.is_some());
    }

    #[test]
    fn json_rpc_response_error_deserializes() {
        let raw =
            r#"{"jsonrpc":"2.0","id":1,"error":{"code":-32601,"message":"Method not found"}}"#;
        let resp: JsonRpcResponse = serde_json::from_str(raw).unwrap();
        assert!(resp.result.is_none());
        let err = resp.error.unwrap();
        assert_eq!(err.code, error_codes::METHOD_NOT_FOUND);
        assert_eq!(err.message, "Method not found");
    }

    #[test]
    fn json_rpc_response_into_result_success() {
        let resp = JsonRpcResponse {
            jsonrpc: "2.0".into(),
            id: JsonRpcId::Number(1),
            result: Some(json!(42)),
            error: None,
        };
        assert_eq!(resp.into_result().unwrap(), json!(42));
    }

    #[test]
    fn json_rpc_response_into_result_error() {
        let resp = JsonRpcResponse {
            jsonrpc: "2.0".into(),
            id: JsonRpcId::Number(1),
            result: None,
            error: Some(JsonRpcError {
                code: -32600,
                message: "Invalid Request".into(),
                data: None,
            }),
        };
        assert!(resp.into_result().is_err());
    }

    // -----------------------------------------------------------------------
    // JSON-RPC ID variants
    // -----------------------------------------------------------------------

    #[test]
    fn json_rpc_id_number_round_trips() {
        let id = JsonRpcId::Number(123);
        let json = serde_json::to_string(&id).unwrap();
        assert_eq!(json, "123");
        let parsed: JsonRpcId = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed, id);
    }

    #[test]
    fn json_rpc_id_string_round_trips() {
        let id = JsonRpcId::Str("abc-123".into());
        let json = serde_json::to_string(&id).unwrap();
        assert_eq!(json, r#""abc-123""#);
        let parsed: JsonRpcId = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed, id);
    }

    // -----------------------------------------------------------------------
    // Initialize types
    // -----------------------------------------------------------------------

    #[test]
    fn initialize_params_serializes() {
        let params = InitializeParams {
            protocol_version: PROTOCOL_VERSION.into(),
            capabilities: ClientCapabilities::default(),
            client_info: ClientInfo {
                name: CLIENT_NAME.into(),
                version: CLIENT_VERSION.into(),
            },
        };
        let json = serde_json::to_value(&params).unwrap();
        assert_eq!(json["protocolVersion"], PROTOCOL_VERSION);
        assert_eq!(json["clientInfo"]["name"], CLIENT_NAME);
    }

    #[test]
    fn initialize_result_deserializes() {
        let raw = json!({
            "protocolVersion": "2025-03-26",
            "capabilities": {
                "tools": { "listChanged": true }
            },
            "serverInfo": {
                "name": "test-server",
                "version": "1.0.0"
            }
        });
        let result: InitializeResult = serde_json::from_value(raw).unwrap();
        assert_eq!(result.protocol_version, "2025-03-26");
        assert_eq!(result.server_info.name, "test-server");
        assert_eq!(result.server_info.version, Some("1.0.0".into()));
        let tools_cap = result.capabilities.tools.unwrap();
        assert!(tools_cap.list_changed);
    }

    #[test]
    fn initialize_result_minimal_deserializes() {
        let raw = json!({
            "protocolVersion": "2025-03-26",
            "capabilities": {},
            "serverInfo": { "name": "minimal" }
        });
        let result: InitializeResult = serde_json::from_value(raw).unwrap();
        assert!(result.capabilities.tools.is_none());
        assert!(result.server_info.version.is_none());
    }

    // -----------------------------------------------------------------------
    // Tool definitions
    // -----------------------------------------------------------------------

    #[test]
    fn tool_definition_deserializes() {
        let raw = json!({
            "name": "read_file",
            "description": "Read a file",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "path": { "type": "string" }
                },
                "required": ["path"]
            }
        });
        let tool: McpToolDefinition = serde_json::from_value(raw).unwrap();
        assert_eq!(tool.name, "read_file");
        assert_eq!(tool.description, Some("Read a file".into()));
        assert_eq!(tool.input_schema["type"], "object");
    }

    #[test]
    fn tool_definition_minimal_deserializes() {
        let raw = json!({ "name": "ping" });
        let tool: McpToolDefinition = serde_json::from_value(raw).unwrap();
        assert_eq!(tool.name, "ping");
        assert!(tool.description.is_none());
        assert_eq!(tool.input_schema["type"], "object");
    }

    #[test]
    fn tool_list_result_deserializes() {
        let raw = json!({
            "tools": [
                { "name": "tool_a", "description": "A" },
                { "name": "tool_b" }
            ]
        });
        let result: ToolListResult = serde_json::from_value(raw).unwrap();
        assert_eq!(result.tools.len(), 2);
        assert!(result.next_cursor.is_none());
    }

    #[test]
    fn tool_list_result_with_cursor() {
        let raw = json!({
            "tools": [],
            "nextCursor": "abc123"
        });
        let result: ToolListResult = serde_json::from_value(raw).unwrap();
        assert_eq!(result.next_cursor, Some("abc123".into()));
    }

    // -----------------------------------------------------------------------
    // Tool call params
    // -----------------------------------------------------------------------

    #[test]
    fn tool_call_params_serializes() {
        let params = ToolCallParams {
            name: "read_file".into(),
            arguments: Some(json!({ "path": "test.txt" })),
        };
        let json = serde_json::to_value(&params).unwrap();
        assert_eq!(json["name"], "read_file");
        assert_eq!(json["arguments"]["path"], "test.txt");
    }

    #[test]
    fn tool_call_params_no_arguments_serializes() {
        let params = ToolCallParams {
            name: "ping".into(),
            arguments: None,
        };
        let json = serde_json::to_value(&params).unwrap();
        assert!(json.get("arguments").is_none());
    }

    // -----------------------------------------------------------------------
    // Tool call result
    // -----------------------------------------------------------------------

    #[test]
    fn tool_call_result_text_deserializes() {
        let raw = json!({
            "content": [
                { "type": "text", "text": "hello world" }
            ],
            "isError": false
        });
        let result: ToolCallResult = serde_json::from_value(raw).unwrap();
        assert!(!result.is_error);
        assert_eq!(result.text(), "hello world");
    }

    #[test]
    fn tool_call_result_error_deserializes() {
        let raw = json!({
            "content": [
                { "type": "text", "text": "something went wrong" }
            ],
            "isError": true
        });
        let result: ToolCallResult = serde_json::from_value(raw).unwrap();
        assert!(result.is_error);
        assert_eq!(result.text(), "something went wrong");
    }

    #[test]
    fn tool_call_result_multiple_blocks() {
        let raw = json!({
            "content": [
                { "type": "text", "text": "line 1" },
                { "type": "text", "text": "line 2" },
                { "type": "image", "data": "base64...", "mimeType": "image/png" }
            ]
        });
        let result: ToolCallResult = serde_json::from_value(raw).unwrap();
        assert_eq!(result.text(), "line 1\nline 2");
        assert!(!result.is_error);
    }

    #[test]
    fn tool_call_result_empty_content() {
        let raw = json!({ "content": [] });
        let result: ToolCallResult = serde_json::from_value(raw).unwrap();
        assert_eq!(result.text(), "");
    }

    // -----------------------------------------------------------------------
    // Tool content variants
    // -----------------------------------------------------------------------

    #[test]
    fn tool_content_text_round_trips() {
        let content = ToolContent::Text {
            text: "hello".into(),
        };
        let json = serde_json::to_value(&content).unwrap();
        assert_eq!(json["type"], "text");
        assert_eq!(json["text"], "hello");
        let parsed: ToolContent = serde_json::from_value(json).unwrap();
        assert_eq!(content, parsed);
    }

    #[test]
    fn tool_content_image_round_trips() {
        let content = ToolContent::Image {
            data: "aGVsbG8=".into(),
            mime_type: "image/png".into(),
        };
        let json = serde_json::to_value(&content).unwrap();
        assert_eq!(json["type"], "image");
        assert_eq!(json["mimeType"], "image/png");
        let parsed: ToolContent = serde_json::from_value(json).unwrap();
        assert_eq!(content, parsed);
    }

    // -----------------------------------------------------------------------
    // Error display
    // -----------------------------------------------------------------------

    #[test]
    fn json_rpc_error_display() {
        let err = JsonRpcError {
            code: -32601,
            message: "Method not found".into(),
            data: None,
        };
        assert_eq!(format!("{err}"), "JSON-RPC error -32601: Method not found");
    }

    #[test]
    fn json_rpc_error_display_with_data() {
        let err = JsonRpcError {
            code: -32602,
            message: "Invalid params".into(),
            data: Some(json!("missing field")),
        };
        let s = format!("{err}");
        assert!(s.contains("Invalid params"));
        assert!(s.contains("missing field"));
    }
}
