//! MCP client: high-level operations over the MCP transport.
//!
//! Provides `McpClient` which manages the lifecycle of an MCP connection:
//! `connect()` → `list_tools()` → `call_tool()`. The client handles the
//! `initialize` handshake, tool discovery, and tool invocation, returning
//! strongly-typed results.

use super::transport::{McpTransport, McpTransportConfig, McpTransportError};
use super::types::{
    ClientCapabilities, ClientInfo, InitializeParams, InitializeResult, McpToolDefinition,
    ToolCallParams, ToolCallResult, ToolListResult, CLIENT_NAME, CLIENT_VERSION, PROTOCOL_VERSION,
};

/// Errors that can occur during MCP client operations.
#[derive(Debug)]
pub enum McpClientError {
    /// Transport-level error (HTTP, timeout, JSON parse).
    Transport(McpTransportError),
    /// The server returned a JSON-RPC error.
    Rpc(super::types::JsonRpcError),
    /// Failed to deserialize the RPC result into the expected type.
    ResultParse {
        method: String,
        source: serde_json::Error,
    },
    /// The client is not connected (initialize not called or failed).
    NotConnected,
    /// The server does not advertise tool capabilities.
    NoToolCapability,
}

impl std::fmt::Display for McpClientError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            McpClientError::Transport(e) => write!(f, "MCP transport error: {e}"),
            McpClientError::Rpc(e) => write!(f, "MCP RPC error: {e}"),
            McpClientError::ResultParse { method, source } => {
                write!(f, "MCP result parse error for '{method}': {source}")
            }
            McpClientError::NotConnected => {
                write!(f, "MCP client not connected (call connect() first)")
            }
            McpClientError::NoToolCapability => {
                write!(f, "MCP server does not advertise tool capabilities")
            }
        }
    }
}

impl std::error::Error for McpClientError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            McpClientError::Transport(e) => Some(e),
            McpClientError::Rpc(e) => Some(e),
            McpClientError::ResultParse { source, .. } => Some(source),
            _ => None,
        }
    }
}

impl From<McpTransportError> for McpClientError {
    fn from(e: McpTransportError) -> Self {
        McpClientError::Transport(e)
    }
}

/// Connection state of the MCP client.
#[derive(Debug, Clone, PartialEq, Eq)]
enum ConnectionState {
    /// Not yet connected.
    Disconnected,
    /// Successfully initialized with the server.
    Connected,
}

/// High-level MCP client for communicating with a local MCP server.
///
/// Usage:
/// ```no_run
/// # use grokrs_api::mcp::client::McpClient;
/// # async fn example() -> Result<(), Box<dyn std::error::Error>> {
/// let mut client = McpClient::new("http://localhost:8080/mcp")?;
/// client.connect().await?;
/// let tools = client.list_tools().await?;
/// for tool in &tools {
///     println!("{}: {}", tool.name, tool.description.as_deref().unwrap_or(""));
/// }
/// let result = client.call_tool("read_file", Some(serde_json::json!({"path": "test.txt"}))).await?;
/// println!("{}", result.text());
/// # Ok(())
/// # }
/// ```
pub struct McpClient {
    transport: McpTransport,
    state: ConnectionState,
    /// Server info received during initialization.
    server_info: Option<super::types::ServerInfo>,
    /// Label for this MCP server (for logging and display).
    label: Option<String>,
}

impl McpClient {
    /// Create a new MCP client targeting the given server URL.
    ///
    /// The client is not connected until [`connect()`](McpClient::connect) is called.
    pub fn new(server_url: impl Into<String>) -> Result<Self, McpClientError> {
        let config = McpTransportConfig::new(server_url);
        let transport = McpTransport::new(config)?;
        Ok(Self {
            transport,
            state: ConnectionState::Disconnected,
            server_info: None,
            label: None,
        })
    }

    /// Create a new MCP client with a custom transport configuration.
    pub fn with_config(config: McpTransportConfig) -> Result<Self, McpClientError> {
        let transport = McpTransport::new(config)?;
        Ok(Self {
            transport,
            state: ConnectionState::Disconnected,
            server_info: None,
            label: None,
        })
    }

    /// Set a human-readable label for this MCP server.
    pub fn with_label(mut self, label: impl Into<String>) -> Self {
        self.label = Some(label.into());
        self
    }

    /// Return the server URL.
    pub fn server_url(&self) -> &str {
        self.transport.server_url()
    }

    /// Return the server host (for policy effects).
    pub fn server_host(&self) -> String {
        self.transport.server_host()
    }

    /// Return the label (if set).
    pub fn label(&self) -> Option<&str> {
        self.label.as_deref()
    }

    /// Return the server info received during initialization.
    pub fn server_info(&self) -> Option<&super::types::ServerInfo> {
        self.server_info.as_ref()
    }

    /// Whether the client is connected (initialize succeeded).
    pub fn is_connected(&self) -> bool {
        self.state == ConnectionState::Connected
    }

    /// Perform the MCP `initialize` handshake.
    ///
    /// Must be called before `list_tools()` or `call_tool()`. Sends the
    /// `initialize` request followed by `notifications/initialized`.
    pub async fn connect(&mut self) -> Result<InitializeResult, McpClientError> {
        let init_params = InitializeParams {
            protocol_version: PROTOCOL_VERSION.to_owned(),
            capabilities: ClientCapabilities::default(),
            client_info: ClientInfo {
                name: CLIENT_NAME.to_owned(),
                version: CLIENT_VERSION.to_owned(),
            },
        };

        let params =
            serde_json::to_value(&init_params).expect("InitializeParams serialization cannot fail");

        let response = self.transport.send("initialize", Some(params)).await?;
        let result_value = response.into_result().map_err(McpClientError::Rpc)?;

        let init_result: InitializeResult =
            serde_json::from_value(result_value).map_err(|source| McpClientError::ResultParse {
                method: "initialize".into(),
                source,
            })?;

        self.server_info = Some(init_result.server_info.clone());
        self.state = ConnectionState::Connected;

        // Send the `notifications/initialized` notification (fire-and-forget).
        // This is a JSON-RPC notification (no id), but we send it as a request
        // and ignore the response. Some servers may not respond.
        let _ = self.transport.send("notifications/initialized", None).await;

        Ok(init_result)
    }

    /// Discover available tools from the MCP server.
    ///
    /// Returns all tools, handling pagination if the server uses cursors.
    /// Requires [`connect()`](McpClient::connect) to have been called first.
    pub async fn list_tools(&self) -> Result<Vec<McpToolDefinition>, McpClientError> {
        if self.state != ConnectionState::Connected {
            return Err(McpClientError::NotConnected);
        }

        let mut all_tools = Vec::new();
        let mut cursor: Option<String> = None;

        loop {
            let params = cursor.as_ref().map(|c| serde_json::json!({ "cursor": c }));

            let response = self.transport.send("tools/list", params).await?;
            let result_value = response.into_result().map_err(McpClientError::Rpc)?;

            let list_result: ToolListResult =
                serde_json::from_value(result_value).map_err(|source| {
                    McpClientError::ResultParse {
                        method: "tools/list".into(),
                        source,
                    }
                })?;

            all_tools.extend(list_result.tools);

            match list_result.next_cursor {
                Some(next) if !next.is_empty() => cursor = Some(next),
                _ => break,
            }
        }

        Ok(all_tools)
    }

    /// Invoke a tool on the MCP server.
    ///
    /// Requires [`connect()`](McpClient::connect) to have been called first.
    pub async fn call_tool(
        &self,
        name: &str,
        arguments: Option<serde_json::Value>,
    ) -> Result<ToolCallResult, McpClientError> {
        if self.state != ConnectionState::Connected {
            return Err(McpClientError::NotConnected);
        }

        let params = ToolCallParams {
            name: name.to_owned(),
            arguments,
        };

        let params_value =
            serde_json::to_value(&params).expect("ToolCallParams serialization cannot fail");

        let response = self
            .transport
            .send("tools/call", Some(params_value))
            .await?;
        let result_value = response.into_result().map_err(McpClientError::Rpc)?;

        serde_json::from_value(result_value).map_err(|source| McpClientError::ResultParse {
            method: "tools/call".into(),
            source,
        })
    }
}

impl std::fmt::Debug for McpClient {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("McpClient")
            .field("server_url", &self.transport.server_url())
            .field("state", &self.state)
            .field("label", &self.label)
            .field("server_info", &self.server_info)
            .finish()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn client_new_valid_url() {
        let client = McpClient::new("http://localhost:8080/mcp");
        assert!(client.is_ok());
        let client = client.unwrap();
        assert_eq!(client.server_url(), "http://localhost:8080/mcp");
        assert!(!client.is_connected());
    }

    #[test]
    fn client_new_invalid_url() {
        let client = McpClient::new("not a url");
        assert!(client.is_err());
    }

    #[test]
    fn client_with_label() {
        let client = McpClient::new("http://localhost:8080")
            .unwrap()
            .with_label("test-server");
        assert_eq!(client.label(), Some("test-server"));
    }

    #[test]
    fn client_server_host() {
        let client = McpClient::new("http://tools.example.com:9090/v1").unwrap();
        assert_eq!(client.server_host(), "tools.example.com");
    }

    #[tokio::test]
    async fn list_tools_before_connect_fails() {
        let client = McpClient::new("http://localhost:8080").unwrap();
        let result = client.list_tools().await;
        assert!(result.is_err());
        assert!(matches!(result.unwrap_err(), McpClientError::NotConnected));
    }

    #[tokio::test]
    async fn call_tool_before_connect_fails() {
        let client = McpClient::new("http://localhost:8080").unwrap();
        let result = client.call_tool("test", None).await;
        assert!(result.is_err());
        assert!(matches!(result.unwrap_err(), McpClientError::NotConnected));
    }

    #[test]
    fn client_error_display() {
        let err = McpClientError::NotConnected;
        assert!(format!("{err}").contains("not connected"));

        let err = McpClientError::NoToolCapability;
        assert!(format!("{err}").contains("tool capabilities"));

        let err = McpClientError::ResultParse {
            method: "tools/list".into(),
            source: serde_json::from_str::<String>("invalid").unwrap_err(),
        };
        assert!(format!("{err}").contains("tools/list"));
    }

    #[test]
    fn client_debug_format() {
        let client = McpClient::new("http://localhost:8080")
            .unwrap()
            .with_label("debug-test");
        let debug = format!("{client:?}");
        assert!(debug.contains("McpClient"));
        assert!(debug.contains("localhost"));
        assert!(debug.contains("debug-test"));
    }
}
