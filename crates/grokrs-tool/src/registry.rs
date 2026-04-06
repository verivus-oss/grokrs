//! Tool registry for managing and querying available tools at runtime.
//!
//! The [`ToolRegistry`] holds a collection of [`ErasedTool`] trait objects and
//! provides methods for:
//! - Registering tools
//! - Filtering available tools by trust rank
//! - Looking up tools by name
//! - Generating Responses API function definitions for the model
//!
//! Use [`default_registry`] to create a registry pre-loaded with the built-in
//! tools at their appropriate trust levels.

use serde_json::json;

use crate::erased::ErasedTool;

/// A registry of dynamically-dispatched tools.
///
/// Tools are stored as `Box<dyn ErasedTool>` and can be filtered by trust rank,
/// looked up by name, and serialized to API-compatible function definitions.
pub struct ToolRegistry {
    tools: Vec<Box<dyn ErasedTool>>,
}

impl ToolRegistry {
    /// Create an empty registry.
    pub fn new() -> Self {
        Self { tools: Vec::new() }
    }

    /// Register a tool.
    pub fn register(&mut self, tool: Box<dyn ErasedTool>) {
        self.tools.push(tool);
    }

    /// Return all tools whose `min_trust_rank` is <= the given `trust_rank`.
    pub fn available_tools(&self, trust_rank: u8) -> Vec<&dyn ErasedTool> {
        self.tools
            .iter()
            .filter(|t| t.min_trust_rank() <= trust_rank)
            .map(|t| t.as_ref())
            .collect()
    }

    /// Look up a tool by its machine-readable name.
    pub fn get(&self, name: &str) -> Option<&dyn ErasedTool> {
        self.tools
            .iter()
            .find(|t| t.name() == name)
            .map(|t| t.as_ref())
    }

    /// Generate Responses API function definitions for all tools available at
    /// the given trust rank.
    ///
    /// Each definition is a flat JSON object:
    /// ```json
    /// {
    ///   "type": "function",
    ///   "name": "read_file",
    ///   "description": "...",
    ///   "parameters": { ... }
    /// }
    /// ```
    pub fn tool_definitions(&self, trust_rank: u8) -> Vec<serde_json::Value> {
        self.available_tools(trust_rank)
            .iter()
            .map(|t| {
                json!({
                    "type": "function",
                    "name": t.name(),
                    "description": t.description(),
                    "parameters": t.input_schema(),
                })
            })
            .collect()
    }

    /// Return the total number of registered tools (regardless of trust rank).
    pub fn len(&self) -> usize {
        self.tools.len()
    }

    /// Return `true` if the registry contains no tools.
    pub fn is_empty(&self) -> bool {
        self.tools.is_empty()
    }
}

impl Default for ToolRegistry {
    fn default() -> Self {
        Self::new()
    }
}

/// Create a registry pre-loaded with the built-in tools at their appropriate
/// trust levels.
///
/// | Tool             | min_trust_rank | Available to              |
/// |------------------|---------------|---------------------------|
/// | `read_file`      | 0             | All (including Untrusted) |
/// | `list_directory` | 0             | All (including Untrusted) |
/// | `git_status`     | 0             | All (including Untrusted) |
/// | `git_diff`       | 0             | All (including Untrusted) |
/// | `write_file`     | 1             | InteractiveTrusted+       |
/// | `git_add`        | 1             | InteractiveTrusted+       |
/// | `run_command`    | 2             | AdminTrusted only         |
/// | `git_commit`     | 2             | AdminTrusted only         |
pub fn default_registry() -> ToolRegistry {
    use crate::tools::{
        ForgetTool, GitAddTool, GitCommitTool, GitDiffTool, GitStatusTool, ListDirectoryTool,
        ReadFileTool, RecallTool, RememberTool, RunCommandTool, WriteFileTool,
    };

    let mut registry = ToolRegistry::new();
    // Rank 0: read-only tools (available to all trust levels).
    registry.register(Box::new(ErasedToolWrapper::new(ReadFileTool, 0)));
    registry.register(Box::new(ErasedToolWrapper::new(ListDirectoryTool, 0)));
    registry.register(Box::new(ErasedToolWrapper::new(GitStatusTool, 0)));
    registry.register(Box::new(ErasedToolWrapper::new(GitDiffTool, 0)));
    registry.register(Box::new(ErasedToolWrapper::new(RecallTool, 0)));
    // Rank 1: write tools (InteractiveTrusted and above).
    registry.register(Box::new(ErasedToolWrapper::new(WriteFileTool, 1)));
    registry.register(Box::new(ErasedToolWrapper::new(GitAddTool, 1)));
    registry.register(Box::new(ErasedToolWrapper::new(RememberTool, 1)));
    registry.register(Box::new(ErasedToolWrapper::new(ForgetTool, 1)));
    // Rank 2: high-privilege tools (AdminTrusted only).
    registry.register(Box::new(ErasedToolWrapper::new(
        RunCommandTool::default(),
        2,
    )));
    registry.register(Box::new(ErasedToolWrapper::new(GitCommitTool, 2)));
    registry
}

// ---------------------------------------------------------------------------
// ErasedToolWrapper — bridges ToolSpec to ErasedTool
// ---------------------------------------------------------------------------

use grokrs_cap::WorkspaceRoot;
use grokrs_policy::Effect;
use serde::de::DeserializeOwned;

use crate::error::ToolError;
use crate::{Classify, ToolSpec};

/// A wrapper that bridges a generic [`ToolSpec`] implementation to the
/// object-safe [`ErasedTool`] trait.
///
/// The wrapper:
/// 1. Deserializes JSON input into the tool's `Input` type.
/// 2. Delegates `classify` to the `Input`'s `Classify` implementation.
/// 3. Executes the tool using `block_on` (bridging async to sync).
/// 4. Serializes the output back to a JSON string.
///
/// The `Input` must implement `DeserializeOwned` and the `Output` must
/// implement `serde::Serialize` for the JSON round-trip.
pub struct ErasedToolWrapper<T>
where
    T: ToolSpec,
{
    inner: T,
    min_rank: u8,
}

impl<T> ErasedToolWrapper<T>
where
    T: ToolSpec,
{
    /// Create a new wrapper with the given tool and minimum trust rank.
    pub fn new(tool: T, min_rank: u8) -> Self {
        Self {
            inner: tool,
            min_rank,
        }
    }
}

impl<T> ErasedTool for ErasedToolWrapper<T>
where
    T: ToolSpec + Send + Sync + 'static,
    T::Input: DeserializeOwned,
    T::Output: serde::Serialize,
{
    fn name(&self) -> &str {
        self.inner.name()
    }

    fn description(&self) -> &str {
        self.inner.description()
    }

    fn input_schema(&self) -> serde_json::Value {
        self.inner.input_schema()
    }

    fn min_trust_rank(&self) -> u8 {
        self.min_rank
    }

    fn execute_json(&self, input_json: &str, root: &WorkspaceRoot) -> Result<String, ToolError> {
        let input: T::Input = serde_json::from_str(input_json).map_err(|e| {
            ToolError::Other(format!(
                "failed to deserialize input for tool '{}': {e}",
                self.inner.name()
            ))
        })?;

        // Create a tracing span for this tool execution when otel is enabled.
        #[cfg(feature = "otel")]
        let _tool_span = tracing::info_span!(
            "tool_execute",
            tool.name = self.inner.name(),
            tool.min_trust_rank = self.min_rank,
            tool.execution_ms = tracing::field::Empty,
        );
        #[cfg(feature = "otel")]
        let _tool_span_guard = _tool_span.enter();

        #[cfg(feature = "otel")]
        let started = std::time::Instant::now();

        // Bridge async to sync. We use `block_in_place` + `block_on` so this
        // works correctly even when called from within an existing tokio runtime
        // (e.g., during tests or from an async caller).
        let output = tokio::task::block_in_place(|| {
            tokio::runtime::Handle::current().block_on(self.inner.execute(input, root))
        })?;

        #[cfg(feature = "otel")]
        {
            let elapsed_ms = started.elapsed().as_millis() as u64;
            tracing::Span::current().record("tool.execution_ms", elapsed_ms);
        }

        let json = serde_json::to_string(&output).map_err(|e| {
            ToolError::Other(format!(
                "failed to serialize output for tool '{}': {e}",
                self.inner.name()
            ))
        })?;
        Ok(json)
    }

    fn classify_json(&self, input_json: &str) -> Result<Vec<Effect>, ToolError> {
        let input: T::Input = serde_json::from_str(input_json).map_err(|e| {
            ToolError::Other(format!(
                "failed to deserialize input for tool '{}': {e}",
                self.inner.name()
            ))
        })?;
        input.classify()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tools::{ReadFileTool, WriteFileTool};
    use std::fs;
    use tempfile::TempDir;

    fn workspace(dir: &TempDir) -> WorkspaceRoot {
        WorkspaceRoot::new(dir.path()).unwrap()
    }

    #[test]
    fn default_registry_has_eleven_tools() {
        let reg = default_registry();
        assert_eq!(reg.len(), 11);
    }

    #[test]
    fn available_tools_rank_0_returns_five() {
        let reg = default_registry();
        let tools = reg.available_tools(0);
        assert_eq!(tools.len(), 5);
        let names: Vec<&str> = tools.iter().map(|t| t.name()).collect();
        assert!(names.contains(&"read_file"));
        assert!(names.contains(&"list_directory"));
        assert!(names.contains(&"git_status"));
        assert!(names.contains(&"git_diff"));
        assert!(names.contains(&"recall"));
    }

    #[test]
    fn available_tools_rank_1_returns_nine() {
        let reg = default_registry();
        let tools = reg.available_tools(1);
        assert_eq!(tools.len(), 9);
        let names: Vec<&str> = tools.iter().map(|t| t.name()).collect();
        assert!(names.contains(&"read_file"));
        assert!(names.contains(&"list_directory"));
        assert!(names.contains(&"git_status"));
        assert!(names.contains(&"git_diff"));
        assert!(names.contains(&"recall"));
        assert!(names.contains(&"write_file"));
        assert!(names.contains(&"git_add"));
        assert!(names.contains(&"remember"));
        assert!(names.contains(&"forget"));
    }

    #[test]
    fn available_tools_rank_2_returns_eleven() {
        let reg = default_registry();
        let tools = reg.available_tools(2);
        assert_eq!(tools.len(), 11);
        let names: Vec<&str> = tools.iter().map(|t| t.name()).collect();
        assert!(names.contains(&"read_file"));
        assert!(names.contains(&"list_directory"));
        assert!(names.contains(&"git_status"));
        assert!(names.contains(&"git_diff"));
        assert!(names.contains(&"recall"));
        assert!(names.contains(&"write_file"));
        assert!(names.contains(&"git_add"));
        assert!(names.contains(&"remember"));
        assert!(names.contains(&"forget"));
        assert!(names.contains(&"run_command"));
        assert!(names.contains(&"git_commit"));
    }

    #[test]
    fn tool_definitions_returns_valid_json_schemas() {
        let reg = default_registry();
        let defs = reg.tool_definitions(2);
        assert_eq!(defs.len(), 11);
        for def in &defs {
            assert_eq!(def["type"], "function", "def: {def}");
            assert!(def["name"].is_string(), "missing name in: {def}");
            assert!(
                def["description"].is_string(),
                "missing description in: {def}"
            );
            assert!(
                def["parameters"].is_object(),
                "missing parameters in: {def}"
            );
            assert_eq!(
                def["parameters"]["type"], "object",
                "parameters should be an object schema: {def}"
            );
        }
    }

    #[test]
    fn tool_definitions_filtered_by_trust_rank() {
        let reg = default_registry();
        let defs_0 = reg.tool_definitions(0);
        let defs_1 = reg.tool_definitions(1);
        let defs_2 = reg.tool_definitions(2);
        assert_eq!(defs_0.len(), 5);
        assert_eq!(defs_1.len(), 9);
        assert_eq!(defs_2.len(), 11);
    }

    #[test]
    fn get_read_file_returns_correct_tool() {
        let reg = default_registry();
        let tool = reg
            .get("read_file")
            .expect("read_file should be registered");
        assert_eq!(tool.name(), "read_file");
        assert!(!tool.description().is_empty());
        assert_eq!(tool.min_trust_rank(), 0);
    }

    #[test]
    fn get_write_file_returns_correct_tool() {
        let reg = default_registry();
        let tool = reg
            .get("write_file")
            .expect("write_file should be registered");
        assert_eq!(tool.name(), "write_file");
        assert_eq!(tool.min_trust_rank(), 1);
    }

    #[test]
    fn get_run_command_returns_correct_tool() {
        let reg = default_registry();
        let tool = reg
            .get("run_command")
            .expect("run_command should be registered");
        assert_eq!(tool.name(), "run_command");
        assert_eq!(tool.min_trust_rank(), 2);
    }

    #[test]
    fn get_git_status_returns_correct_tool() {
        let reg = default_registry();
        let tool = reg
            .get("git_status")
            .expect("git_status should be registered");
        assert_eq!(tool.name(), "git_status");
        assert_eq!(tool.min_trust_rank(), 0);
    }

    #[test]
    fn get_git_diff_returns_correct_tool() {
        let reg = default_registry();
        let tool = reg.get("git_diff").expect("git_diff should be registered");
        assert_eq!(tool.name(), "git_diff");
        assert_eq!(tool.min_trust_rank(), 0);
    }

    #[test]
    fn get_git_add_returns_correct_tool() {
        let reg = default_registry();
        let tool = reg.get("git_add").expect("git_add should be registered");
        assert_eq!(tool.name(), "git_add");
        assert_eq!(tool.min_trust_rank(), 1);
    }

    #[test]
    fn get_git_commit_returns_correct_tool() {
        let reg = default_registry();
        let tool = reg
            .get("git_commit")
            .expect("git_commit should be registered");
        assert_eq!(tool.name(), "git_commit");
        assert_eq!(tool.min_trust_rank(), 2);
    }

    #[test]
    fn get_nonexistent_returns_none() {
        let reg = default_registry();
        assert!(reg.get("nonexistent_tool").is_none());
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn execute_json_read_file_round_trips() {
        let dir = TempDir::new().unwrap();
        let root = workspace(&dir);
        fs::write(dir.path().join("test.txt"), "hello from erased").unwrap();

        let wrapper = ErasedToolWrapper::new(ReadFileTool, 0);
        let input_json = r#"{"path": "test.txt"}"#;
        let result = wrapper.execute_json(input_json, &root).unwrap();
        // Output is a JSON string (the content serialized)
        let content: String = serde_json::from_str(&result).unwrap();
        assert_eq!(content, "hello from erased");
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn execute_json_write_file_round_trips() {
        let dir = TempDir::new().unwrap();
        let root = workspace(&dir);

        let wrapper = ErasedToolWrapper::new(WriteFileTool, 1);
        let input_json = r#"{"path": "out.txt", "content": "written via erased"}"#;
        let result = wrapper.execute_json(input_json, &root).unwrap();
        // Output is a JSON string (the message serialized)
        let msg: String = serde_json::from_str(&result).unwrap();
        assert!(msg.contains("bytes"), "message: {msg}");

        let on_disk = fs::read_to_string(dir.path().join("out.txt")).unwrap();
        assert_eq!(on_disk, "written via erased");
    }

    #[tokio::test]
    async fn classify_json_read_file() {
        let wrapper = ErasedToolWrapper::new(ReadFileTool, 0);
        let effects = wrapper.classify_json(r#"{"path": "src/main.rs"}"#).unwrap();
        assert_eq!(effects.len(), 1);
        assert!(
            matches!(&effects[0], Effect::FsRead(wp) if wp.as_path().to_str() == Some("src/main.rs"))
        );
    }

    #[tokio::test]
    async fn classify_json_rejects_invalid_path() {
        let wrapper = ErasedToolWrapper::new(ReadFileTool, 0);
        let err = wrapper
            .classify_json(r#"{"path": "/etc/passwd"}"#)
            .unwrap_err();
        assert!(matches!(err, ToolError::PathValidation(_)));
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn execute_json_rejects_bad_json() {
        let dir = TempDir::new().unwrap();
        let root = workspace(&dir);

        let wrapper = ErasedToolWrapper::new(ReadFileTool, 0);
        let err = wrapper.execute_json("not valid json", &root).unwrap_err();
        assert!(matches!(err, ToolError::Other(_)));
    }

    #[test]
    fn empty_registry() {
        let reg = ToolRegistry::new();
        assert!(reg.is_empty());
        assert_eq!(reg.len(), 0);
        assert!(reg.get("anything").is_none());
        assert!(reg.available_tools(255).is_empty());
        assert!(reg.tool_definitions(255).is_empty());
    }

    #[test]
    fn register_custom_tool() {
        let mut reg = ToolRegistry::new();
        reg.register(Box::new(ErasedToolWrapper::new(ReadFileTool, 0)));
        assert_eq!(reg.len(), 1);
        assert!(reg.get("read_file").is_some());
    }
}
