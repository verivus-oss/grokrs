//! Object-safe tool trait for dynamic dispatch.
//!
//! The [`ErasedTool`] trait provides an object-safe interface over the generic
//! [`ToolSpec`] trait, enabling tools to be stored in heterogeneous collections
//! (e.g., `Vec<Box<dyn ErasedTool>>`) and dispatched at runtime without
//! requiring the caller to know the concrete tool type.
//!
//! JSON is the lingua franca: inputs arrive as JSON strings, outputs leave as
//! JSON strings, and schemas are `serde_json::Value`. Async tool execution is
//! bridged to synchronous via `tokio::runtime::Handle::current().block_on()`.

use grokrs_cap::WorkspaceRoot;
use grokrs_policy::Effect;

use crate::error::ToolError;

/// Object-safe trait for tools that can be dynamically dispatched.
///
/// Every method uses concrete types (no generics, no `async`) so the trait
/// is safe to use behind `dyn ErasedTool`. Async tool execution is handled
/// internally by blocking on the future with the current tokio runtime.
pub trait ErasedTool: Send + Sync {
    /// Machine-readable tool name (e.g., `"read_file"`).
    fn name(&self) -> &str;

    /// Human-readable description for model consumption.
    fn description(&self) -> &str;

    /// JSON Schema describing the expected input structure.
    fn input_schema(&self) -> serde_json::Value;

    /// Minimum trust rank required to use this tool.
    ///
    /// The tool is available to sessions whose trust rank is >= this value.
    /// See [`grokrs_cap::TrustLevel::trust_rank`] for the rank mapping.
    fn min_trust_rank(&self) -> u8;

    /// Execute the tool with JSON input, returning JSON output.
    ///
    /// The caller is responsible for evaluating [`classify_json`](ErasedTool::classify_json)
    /// against the policy engine *before* calling this method.
    fn execute_json(&self, input_json: &str, root: &WorkspaceRoot) -> Result<String, ToolError>;

    /// Classify a JSON input into the set of effects it will produce.
    ///
    /// This is the object-safe counterpart of [`Classify::classify`](crate::Classify::classify).
    fn classify_json(&self, input_json: &str) -> Result<Vec<Effect>, ToolError>;
}
