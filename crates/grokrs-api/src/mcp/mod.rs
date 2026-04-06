//! MCP (Model Context Protocol) client-side hosting.
//!
//! This module enables grokrs to act as an MCP client, connecting to local MCP
//! servers and discovering tools that can be used during agent sessions. It
//! complements the existing remote MCP support (server-side, via API) with
//! client-side MCP hosting.
//!
//! ## Architecture
//!
//! - [`types`] — MCP protocol types (JSON-RPC 2.0, tool definitions, tool results).
//! - [`transport`] — Streamable HTTP transport for MCP protocol communication.
//! - [`client`] — High-level MCP client (initialize, list_tools, call_tool).
//! - [`adapter`] — `McpToolAdapter` wrapping MCP tools as `ErasedTool` implementations.
//!
//! ## Usage Flow
//!
//! 1. Create an `McpClient` with the server URL.
//! 2. Call `client.connect()` to perform the MCP `initialize` handshake.
//! 3. Call `client.list_tools()` to discover available tools.
//! 4. Wrap each tool with `McpToolAdapter` and register in the `ToolRegistry`.
//! 5. The policy engine evaluates `NetworkConnect` effects before each tool call.
//!
//! ## Security
//!
//! Every MCP tool invocation produces a `NetworkConnect` effect for the MCP
//! server's host. The policy engine must approve this effect before the call
//! proceeds. MCP tools default to trust rank 1 (InteractiveTrusted) — they
//! are not available to Untrusted sessions unless explicitly configured.

pub mod client;
pub mod transport;
pub mod types;

pub use client::{McpClient, McpClientError};
pub use transport::{McpTransport, McpTransportConfig, McpTransportError};
pub use types::{McpToolDefinition, ToolCallResult};
