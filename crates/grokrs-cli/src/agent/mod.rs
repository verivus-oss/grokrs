//! Agent execution module: policy-gated tool dispatch for the agent loop.
//!
//! This module provides [`PolicyGatedExecutor`] which implements the
//! [`FunctionExecutor`] trait from `grokrs-api`, bridging the tool calling
//! loop to the policy-gated tool registry. Every tool call is:
//!
//! 1. Looked up in the [`ToolRegistry`] by name.
//! 2. Classified via `classify_json()` to determine effects.
//! 3. Evaluated against the [`PolicyEngine`] (with approval mode resolution).
//! 4. Executed via `execute_json()` only if all effects are allowed.
//!
//! The [`policy_bridge`] submodule provides the approval-mode-aware decision
//! resolver shared with the rest of the CLI.

pub mod executor;
pub mod mcp_adapter;
pub mod policy_bridge;

pub use executor::PolicyGatedExecutor;
pub use mcp_adapter::McpToolAdapter;
