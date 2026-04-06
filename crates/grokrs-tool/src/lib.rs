pub mod erased;
pub mod error;
pub mod registry;
pub mod tools;

use grokrs_cap::WorkspaceRoot;
use grokrs_policy::Effect;

pub use error::ToolError;

/// Classifies a tool input into the set of effects it will produce.
///
/// Classification happens *before* execution so the policy engine can
/// gate dangerous operations. Path validation also happens here --
/// invalid paths are rejected at classification time, not at execution.
pub trait Classify {
    /// # Errors
    ///
    /// Returns [`ToolError::Other`] if effect classification fails (e.g., an
    /// invalid workspace path is provided).
    fn classify(&self) -> Result<Vec<Effect>, ToolError>;
}

/// Specification for a tool that the agent can invoke.
///
/// Each tool declares its name, description, input JSON schema, and
/// an async execute method that performs the actual work.
///
/// The `Input` type must implement `Classify` so the policy engine
/// can evaluate effects before execution is permitted.
pub trait ToolSpec {
    type Input: Classify + Send;
    type Output: Send;

    /// Machine-readable tool name (e.g., `"read_file"`).
    fn name(&self) -> &'static str;

    /// Human-readable description for model consumption.
    fn description(&self) -> &'static str;

    /// JSON Schema describing the expected input structure.
    fn input_schema(&self) -> serde_json::Value;

    /// Execute the tool against a workspace root.
    ///
    /// The caller is responsible for evaluating `input.classify()` against
    /// the policy engine *before* calling this method.
    fn execute(
        &self,
        input: Self::Input,
        root: &WorkspaceRoot,
    ) -> impl std::future::Future<Output = Result<Self::Output, ToolError>> + Send;
}
