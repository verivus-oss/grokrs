//! Integration tests for MCP adapter tool discovery and type handling.
//!
//! These tests validate:
//! - `McpToolAdapter` construction and naming conventions
//! - Effect classification (always `NetworkConnect`)
//! - `ErasedTool` trait implementation
//! - MCP protocol type serialization for tool discovery and invocation
//! - `McpClient` handshake flow via `wiremock`
//!
//! The full MCP client flow (connect -> `list_tools` -> `call_tool`) is tested
//! using `wiremock` to mock the MCP server HTTP endpoint.

use std::sync::Arc;

use grokrs_api::mcp::client::McpClient;
use grokrs_api::mcp::types::*;
use grokrs_tool::erased::ErasedTool;

use grokrs_cli::agent::McpToolAdapter;

// ---------------------------------------------------------------------------
// McpToolAdapter construction
// ---------------------------------------------------------------------------

#[test]
fn adapter_with_label_prefixes_tool_name() {
    let definition = McpToolDefinition {
        name: "read_file".to_owned(),
        description: Some("Read a file from the filesystem".to_owned()),
        input_schema: serde_json::json!({
            "type": "object",
            "properties": {
                "path": {"type": "string"}
            },
            "required": ["path"]
        }),
    };

    let client = Arc::new(tokio::sync::RwLock::new(
        McpClient::new("http://localhost:8080/mcp").unwrap(),
    ));

    let adapter = McpToolAdapter::new(
        definition,
        client,
        "localhost".to_owned(),
        Some("code-tools"),
        1,
    );

    // Name should be prefixed.
    assert_eq!(adapter.name(), "mcp_code_tools_read_file");
    assert_eq!(adapter.mcp_name(), "read_file");
}

#[test]
fn adapter_without_label_uses_mcp_prefix() {
    let definition = McpToolDefinition {
        name: "ping".to_owned(),
        description: None,
        input_schema: serde_json::json!({"type": "object", "properties": {}}),
    };

    let client = Arc::new(tokio::sync::RwLock::new(
        McpClient::new("http://localhost:8080").unwrap(),
    ));

    let adapter = McpToolAdapter::new(definition, client, "localhost".to_owned(), None, 0);

    assert_eq!(adapter.name(), "mcp_ping");
}

#[test]
fn adapter_sanitizes_label_special_characters() {
    let definition = McpToolDefinition {
        name: "test".to_owned(),
        description: None,
        input_schema: serde_json::json!({"type": "object"}),
    };

    let client = Arc::new(tokio::sync::RwLock::new(
        McpClient::new("http://localhost:8080").unwrap(),
    ));

    // Label with special characters should be sanitized.
    let adapter = McpToolAdapter::new(
        definition,
        client,
        "localhost".to_owned(),
        Some("My-Server.v2"),
        1,
    );

    assert_eq!(adapter.name(), "mcp_my_server_v2_test");
}

// ---------------------------------------------------------------------------
// ErasedTool trait implementation
// ---------------------------------------------------------------------------

#[test]
fn adapter_description_from_definition() {
    let definition = McpToolDefinition {
        name: "search".to_owned(),
        description: Some("Search for files by pattern".to_owned()),
        input_schema: serde_json::json!({"type": "object"}),
    };

    let client = Arc::new(tokio::sync::RwLock::new(
        McpClient::new("http://localhost:8080").unwrap(),
    ));

    let adapter = McpToolAdapter::new(definition, client, "localhost".to_owned(), None, 0);

    assert_eq!(adapter.description(), "Search for files by pattern");
}

#[test]
fn adapter_description_default_when_none() {
    let definition = McpToolDefinition {
        name: "tool".to_owned(),
        description: None,
        input_schema: serde_json::json!({"type": "object"}),
    };

    let client = Arc::new(tokio::sync::RwLock::new(
        McpClient::new("http://localhost:8080").unwrap(),
    ));

    let adapter = McpToolAdapter::new(definition, client, "localhost".to_owned(), None, 0);

    assert_eq!(adapter.description(), "MCP tool (no description)");
}

#[test]
fn adapter_input_schema_from_definition() {
    let schema = serde_json::json!({
        "type": "object",
        "properties": {
            "query": {"type": "string"},
            "limit": {"type": "integer"}
        },
        "required": ["query"]
    });

    let definition = McpToolDefinition {
        name: "search".to_owned(),
        description: None,
        input_schema: schema.clone(),
    };

    let client = Arc::new(tokio::sync::RwLock::new(
        McpClient::new("http://localhost:8080").unwrap(),
    ));

    let adapter = McpToolAdapter::new(definition, client, "localhost".to_owned(), None, 0);

    assert_eq!(adapter.input_schema(), schema);
}

#[test]
fn adapter_min_trust_rank() {
    let definition = McpToolDefinition {
        name: "tool".to_owned(),
        description: None,
        input_schema: serde_json::json!({"type": "object"}),
    };

    let client = Arc::new(tokio::sync::RwLock::new(
        McpClient::new("http://localhost:8080").unwrap(),
    ));

    // min_rank = 2 (interactive trust)
    let adapter = McpToolAdapter::new(definition, client, "localhost".to_owned(), None, 2);
    assert_eq!(adapter.min_trust_rank(), 2);
}

// ---------------------------------------------------------------------------
// Effect classification
// ---------------------------------------------------------------------------

#[test]
fn adapter_classify_always_returns_network_connect() {
    let definition = McpToolDefinition {
        name: "tool".to_owned(),
        description: None,
        input_schema: serde_json::json!({"type": "object"}),
    };

    let client = Arc::new(tokio::sync::RwLock::new(
        McpClient::new("http://tools.example.com:9090").unwrap(),
    ));

    let adapter = McpToolAdapter::new(definition, client, "tools.example.com".to_owned(), None, 0);

    let effects = adapter.classify_json("{}").unwrap();
    assert_eq!(effects.len(), 1);
    match &effects[0] {
        grokrs_policy::Effect::NetworkConnect { host } => {
            assert_eq!(host, "tools.example.com");
        }
        other => panic!("expected NetworkConnect, got: {other:?}"),
    }

    // Different inputs should produce the same effect.
    let effects2 = adapter.classify_json(r#"{"query":"test"}"#).unwrap();
    assert_eq!(effects2.len(), 1);
    match &effects2[0] {
        grokrs_policy::Effect::NetworkConnect { host } => {
            assert_eq!(host, "tools.example.com");
        }
        other => panic!("expected NetworkConnect, got: {other:?}"),
    }
}

// ---------------------------------------------------------------------------
// MCP protocol types: tool discovery and invocation
// ---------------------------------------------------------------------------

#[test]
fn tool_list_result_with_multiple_tools() {
    let result = ToolListResult {
        tools: vec![
            McpToolDefinition {
                name: "read_file".to_owned(),
                description: Some("Read file contents".to_owned()),
                input_schema: serde_json::json!({
                    "type": "object",
                    "properties": {"path": {"type": "string"}},
                    "required": ["path"]
                }),
            },
            McpToolDefinition {
                name: "write_file".to_owned(),
                description: Some("Write file contents".to_owned()),
                input_schema: serde_json::json!({
                    "type": "object",
                    "properties": {
                        "path": {"type": "string"},
                        "content": {"type": "string"}
                    },
                    "required": ["path", "content"]
                }),
            },
            McpToolDefinition {
                name: "search".to_owned(),
                description: None,
                input_schema: serde_json::json!({"type": "object"}),
            },
        ],
        next_cursor: None,
    };

    let json = serde_json::to_value(&result).unwrap();
    let parsed: ToolListResult = serde_json::from_value(json).unwrap();
    assert_eq!(parsed.tools.len(), 3);
    assert_eq!(parsed.tools[0].name, "read_file");
    assert_eq!(
        parsed.tools[0].description,
        Some("Read file contents".to_owned())
    );
    assert!(parsed.tools[2].description.is_none());
}

#[test]
fn tool_call_params_round_trip() {
    let params = ToolCallParams {
        name: "read_file".to_owned(),
        arguments: Some(serde_json::json!({"path": "test.txt"})),
    };
    let json = serde_json::to_string(&params).unwrap();
    let parsed: ToolCallParams = serde_json::from_str(&json).unwrap();
    assert_eq!(parsed.name, "read_file");
    assert_eq!(parsed.arguments.unwrap()["path"], "test.txt");
}

#[test]
fn tool_call_result_text_extraction() {
    let result = ToolCallResult {
        content: vec![
            ToolContent::Text {
                text: "Line 1".to_owned(),
            },
            ToolContent::Image {
                data: "base64data".to_owned(),
                mime_type: "image/png".to_owned(),
            },
            ToolContent::Text {
                text: "Line 2".to_owned(),
            },
        ],
        is_error: false,
    };

    // text() should concatenate only Text blocks.
    assert_eq!(result.text(), "Line 1\nLine 2");
    assert!(!result.is_error);
}

#[test]
fn tool_call_result_error_flag() {
    let result = ToolCallResult {
        content: vec![ToolContent::Text {
            text: "File not found: /nonexistent".to_owned(),
        }],
        is_error: true,
    };
    assert!(result.is_error);
    assert!(result.text().contains("not found"));
}

// ---------------------------------------------------------------------------
// JSON-RPC protocol types
// ---------------------------------------------------------------------------

#[test]
fn json_rpc_initialize_handshake_types() {
    // Client -> Server: initialize request.
    let init_params = InitializeParams {
        protocol_version: PROTOCOL_VERSION.to_owned(),
        capabilities: ClientCapabilities::default(),
        client_info: ClientInfo {
            name: CLIENT_NAME.to_owned(),
            version: CLIENT_VERSION.to_owned(),
        },
    };

    let req = JsonRpcRequest::new(
        1,
        "initialize",
        Some(serde_json::to_value(&init_params).unwrap()),
    );
    let req_json = serde_json::to_value(&req).unwrap();
    assert_eq!(req_json["jsonrpc"], "2.0");
    assert_eq!(req_json["method"], "initialize");
    assert_eq!(req_json["params"]["protocolVersion"], "2025-03-26");

    // Server -> Client: initialize result.
    let result_json = serde_json::json!({
        "protocolVersion": "2025-03-26",
        "capabilities": {
            "tools": {"listChanged": false}
        },
        "serverInfo": {
            "name": "test-mcp-server",
            "version": "2.0.0"
        }
    });
    let result: InitializeResult = serde_json::from_value(result_json).unwrap();
    assert_eq!(result.protocol_version, "2025-03-26");
    assert_eq!(result.server_info.name, "test-mcp-server");
    let tools_cap = result.capabilities.tools.unwrap();
    assert!(!tools_cap.list_changed);
}

#[test]
fn json_rpc_error_response_handling() {
    let response = JsonRpcResponse {
        jsonrpc: "2.0".to_owned(),
        id: JsonRpcId::Number(1),
        result: None,
        error: Some(JsonRpcError {
            code: error_codes::METHOD_NOT_FOUND,
            message: "Method not found: tools/execute".to_owned(),
            data: None,
        }),
    };

    let result = response.into_result();
    assert!(result.is_err());
    let err = result.unwrap_err();
    assert_eq!(err.code, -32601);
    assert!(err.message.contains("Method not found"));
}

// ---------------------------------------------------------------------------
// McpClient construction
// ---------------------------------------------------------------------------

#[test]
fn mcp_client_rejects_invalid_url() {
    let result = McpClient::new("not a url at all");
    assert!(result.is_err());
}

#[test]
fn mcp_client_valid_url() {
    let client = McpClient::new("http://localhost:8080/mcp").unwrap();
    assert_eq!(client.server_url(), "http://localhost:8080/mcp");
    assert!(!client.is_connected());
}

#[test]
fn mcp_client_with_label() {
    let client = McpClient::new("http://localhost:8080")
        .unwrap()
        .with_label("my-tools");
    assert_eq!(client.label(), Some("my-tools"));
}

#[tokio::test]
async fn mcp_client_list_tools_before_connect_fails() {
    let client = McpClient::new("http://localhost:8080").unwrap();
    let result = client.list_tools().await;
    assert!(result.is_err());
}

#[tokio::test]
async fn mcp_client_call_tool_before_connect_fails() {
    let client = McpClient::new("http://localhost:8080").unwrap();
    let result = client.call_tool("test", None).await;
    assert!(result.is_err());
}

// ---------------------------------------------------------------------------
// MCP tool discovery via wiremock
// ---------------------------------------------------------------------------

#[tokio::test]
async fn mcp_tool_discovery_via_wiremock() {
    use wiremock::matchers::{method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    let mock_server = MockServer::start().await;

    // Mount initialize response.
    Mock::given(method("POST"))
        .and(path("/"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "jsonrpc": "2.0",
            "id": 1,
            "result": {
                "protocolVersion": "2025-03-26",
                "capabilities": {"tools": {"listChanged": false}},
                "serverInfo": {"name": "test-server", "version": "1.0.0"}
            }
        })))
        .expect(1..=3)
        .mount(&mock_server)
        .await;

    let mut client = McpClient::new(mock_server.uri()).unwrap();
    let init_result = client.connect().await.unwrap();
    assert_eq!(init_result.server_info.name, "test-server");
    assert!(client.is_connected());

    // Reset mocks and mount tools/list response.
    mock_server.reset().await;
    Mock::given(method("POST"))
        .and(path("/"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "jsonrpc": "2.0",
            "id": 3,
            "result": {
                "tools": [
                    {
                        "name": "read_file",
                        "description": "Read a file",
                        "inputSchema": {
                            "type": "object",
                            "properties": {"path": {"type": "string"}},
                            "required": ["path"]
                        }
                    },
                    {
                        "name": "list_dir",
                        "description": "List directory contents",
                        "inputSchema": {
                            "type": "object",
                            "properties": {"path": {"type": "string"}}
                        }
                    }
                ]
            }
        })))
        .expect(1)
        .mount(&mock_server)
        .await;

    let tools = client.list_tools().await.unwrap();
    assert_eq!(tools.len(), 2);
    assert_eq!(tools[0].name, "read_file");
    assert_eq!(tools[1].name, "list_dir");
    assert_eq!(tools[0].description.as_deref(), Some("Read a file"));
}

// ---------------------------------------------------------------------------
// MCP tool invocation via wiremock
// ---------------------------------------------------------------------------

#[tokio::test]
async fn mcp_tool_invocation_via_wiremock() {
    use wiremock::matchers::{method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    let mock_server = MockServer::start().await;

    // Mount initialize.
    Mock::given(method("POST"))
        .and(path("/"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "jsonrpc": "2.0",
            "id": 1,
            "result": {
                "protocolVersion": "2025-03-26",
                "capabilities": {},
                "serverInfo": {"name": "tool-server"}
            }
        })))
        .expect(1..=2)
        .mount(&mock_server)
        .await;

    let mut client = McpClient::new(mock_server.uri()).unwrap();
    client.connect().await.unwrap();

    // Reset and mount tool call response.
    mock_server.reset().await;
    Mock::given(method("POST"))
        .and(path("/"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "jsonrpc": "2.0",
            "id": 3,
            "result": {
                "content": [
                    {"type": "text", "text": "fn main() { println!(\"Hello!\"); }"}
                ],
                "isError": false
            }
        })))
        .expect(1)
        .mount(&mock_server)
        .await;

    let result = client
        .call_tool(
            "read_file",
            Some(serde_json::json!({"path": "src/main.rs"})),
        )
        .await
        .unwrap();

    assert!(!result.is_error);
    assert!(result.text().contains("Hello!"));
}
