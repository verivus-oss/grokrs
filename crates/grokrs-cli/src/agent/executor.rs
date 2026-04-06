//! Policy-gated tool executor implementing [`FunctionExecutor`].
//!
//! [`PolicyGatedExecutor`] bridges the `run_tool_loop` function calling loop
//! to the policy-gated tool registry. Every tool call is:
//!
//! 1. Looked up in the [`ToolRegistry`] by name.
//! 2. Classified via `classify_json()` to determine effects.
//! 3. Evaluated against the [`PolicyEngine`] with approval-mode resolution.
//! 4. Executed via `execute_json()` only if **all** effects are allowed.
//!
//! Denied calls return a structured error message (not a panic or exception)
//! so the model can adapt its strategy.

use grokrs_api::tool_loop::FunctionExecutor;
use grokrs_cap::WorkspaceRoot;
use grokrs_policy::PolicyEngine;
use grokrs_tool::registry::ToolRegistry;

use super::policy_bridge::{ResolvedDecision, resolve_decision};

/// A synchronous [`FunctionExecutor`] that gates every tool call through
/// the policy engine before dispatching to the tool registry.
///
/// Designed to be passed to [`grokrs_api::tool_loop::run_tool_loop`].
pub struct PolicyGatedExecutor {
    /// Tool registry for looking up and executing tools.
    registry: ToolRegistry,
    /// Policy engine for evaluating effects.
    engine: PolicyEngine,
    /// Workspace root for tool execution.
    root: WorkspaceRoot,
    /// Approval mode from session config (e.g., "allow", "deny", "interactive").
    approval_mode: String,
    /// Trust rank of the current session, used to filter available tools.
    trust_rank: u8,
}

impl PolicyGatedExecutor {
    /// Create a new executor with the given dependencies.
    ///
    /// # Arguments
    ///
    /// * `registry` - The tool registry containing available tools.
    /// * `engine` - The policy engine for effect evaluation.
    /// * `root` - The workspace root for tool execution.
    /// * `approval_mode` - How to resolve `Ask` decisions ("allow", "deny", "interactive").
    /// * `trust_rank` - Trust rank of the current session (0=Untrusted, 1=InteractiveTrusted, 2=AdminTrusted).
    #[must_use]
    pub fn new(
        registry: ToolRegistry,
        engine: PolicyEngine,
        root: WorkspaceRoot,
        approval_mode: String,
        trust_rank: u8,
    ) -> Self {
        Self {
            registry,
            engine,
            root,
            approval_mode,
            trust_rank,
        }
    }

    /// Return a reference to the tool registry (for generating tool definitions).
    #[must_use]
    pub fn registry(&self) -> &ToolRegistry {
        &self.registry
    }

    /// Return the trust rank.
    #[must_use]
    pub fn trust_rank(&self) -> u8 {
        self.trust_rank
    }
}

impl FunctionExecutor for PolicyGatedExecutor {
    fn execute(
        &self,
        name: &str,
        arguments: &str,
    ) -> Result<String, Box<dyn std::error::Error + Send + Sync>> {
        // Create a tracing span for this tool call (policy + execution).
        #[cfg(feature = "otel")]
        let _tool_call_span = tracing::info_span!(
            "tool_call",
            tool.name = name,
            tool.trust_level = self.trust_rank,
            tool.policy_decision = tracing::field::Empty,
            tool.execution_ms = tracing::field::Empty,
        );
        #[cfg(feature = "otel")]
        let _tool_call_guard = _tool_call_span.enter();

        #[cfg(feature = "otel")]
        let started = std::time::Instant::now();

        // 1. Look up the tool in the registry.
        let tool = match self.registry.get(name) {
            Some(t) => t,
            None => {
                #[cfg(feature = "otel")]
                tracing::Span::current().record("tool.policy_decision", "unknown_tool");

                return Err(format!(
                    "unknown tool '{name}'. Available tools: {}",
                    self.registry
                        .available_tools(self.trust_rank)
                        .iter()
                        .map(|t| t.name())
                        .collect::<Vec<_>>()
                        .join(", ")
                )
                .into());
            }
        };

        // Verify the tool is available at the current trust rank.
        if tool.min_trust_rank() > self.trust_rank {
            #[cfg(feature = "otel")]
            tracing::Span::current().record("tool.policy_decision", "trust_rank_denied");

            return Err(format!(
                "tool '{name}' requires trust rank {} but current session has rank {}",
                tool.min_trust_rank(),
                self.trust_rank
            )
            .into());
        }

        // 2. Classify the input to determine effects.
        let effects = match tool.classify_json(arguments) {
            Ok(effects) => effects,
            Err(e) => {
                #[cfg(feature = "otel")]
                tracing::Span::current().record("tool.policy_decision", "classify_error");

                return Err(format!("failed to classify input for tool '{name}': {e}").into());
            }
        };

        // 3. Evaluate each effect through the policy engine.
        for effect in &effects {
            let decision = self.engine.evaluate(effect);
            let resolved = resolve_decision(decision, &self.approval_mode);
            match resolved {
                ResolvedDecision::Allow => { /* proceed */ }
                ResolvedDecision::Deny { reason } => {
                    #[cfg(feature = "otel")]
                    tracing::Span::current().record("tool.policy_decision", "deny");

                    return Err(format!(
                        "policy denied tool '{name}' for effect {effect:?}: {reason}"
                    )
                    .into());
                }
            }
        }

        #[cfg(feature = "otel")]
        tracing::Span::current().record("tool.policy_decision", "allow");

        // 4. All effects allowed — execute the tool.
        let result = match tool.execute_json(arguments, &self.root) {
            Ok(result) => Ok(result),
            Err(e) => Err(format!("tool '{name}' execution failed: {e}").into()),
        };

        #[cfg(feature = "otel")]
        {
            let elapsed_ms = started.elapsed().as_millis() as u64;
            tracing::Span::current().record("tool.execution_ms", elapsed_ms);
        }

        result
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use grokrs_core::PolicyConfig;
    use grokrs_tool::registry::default_registry;
    use std::fs;
    use tempfile::TempDir;

    fn workspace(dir: &TempDir) -> WorkspaceRoot {
        WorkspaceRoot::new(dir.path()).unwrap()
    }

    fn permissive_engine() -> PolicyEngine {
        PolicyEngine::new(PolicyConfig {
            allow_network: true,
            allow_shell: true,
            allow_workspace_writes: true,
            max_patch_bytes: 1024,
        })
    }

    fn restrictive_engine() -> PolicyEngine {
        PolicyEngine::new(PolicyConfig {
            allow_network: false,
            allow_shell: false,
            allow_workspace_writes: false,
            max_patch_bytes: 1024,
        })
    }

    fn make_executor(
        dir: &TempDir,
        engine: PolicyEngine,
        approval_mode: &str,
        trust_rank: u8,
    ) -> PolicyGatedExecutor {
        PolicyGatedExecutor::new(
            default_registry(),
            engine,
            workspace(dir),
            approval_mode.to_owned(),
            trust_rank,
        )
    }

    // -----------------------------------------------------------------------
    // Unknown tool
    // -----------------------------------------------------------------------

    #[test]
    fn unknown_tool_returns_error() {
        let dir = TempDir::new().unwrap();
        let executor = make_executor(&dir, permissive_engine(), "allow", 2);
        let result = FunctionExecutor::execute(&executor, "nonexistent_tool", "{}");
        assert!(result.is_err());
        let err = result.unwrap_err().to_string();
        assert!(err.contains("unknown tool 'nonexistent_tool'"));
        assert!(err.contains("read_file")); // lists available tools
    }

    // -----------------------------------------------------------------------
    // Trust rank filtering
    // -----------------------------------------------------------------------

    #[test]
    fn trust_rank_too_low_returns_error() {
        let dir = TempDir::new().unwrap();
        // Trust rank 0 cannot use write_file (rank 1) or run_command (rank 2).
        let executor = make_executor(&dir, permissive_engine(), "allow", 0);
        let result = FunctionExecutor::execute(
            &executor,
            "write_file",
            r#"{"path":"x.txt","content":"hi"}"#,
        );
        assert!(result.is_err());
        let err = result.unwrap_err().to_string();
        assert!(err.contains("requires trust rank"));
    }

    // -----------------------------------------------------------------------
    // Successful read_file execution
    // -----------------------------------------------------------------------

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn read_file_success() {
        let dir = TempDir::new().unwrap();
        fs::write(dir.path().join("hello.txt"), "world").unwrap();
        let executor = make_executor(&dir, permissive_engine(), "allow", 0);
        let result = FunctionExecutor::execute(&executor, "read_file", r#"{"path": "hello.txt"}"#);
        assert!(result.is_ok());
        let output = result.unwrap();
        assert!(output.contains("world"));
    }

    // -----------------------------------------------------------------------
    // Successful write_file execution
    // -----------------------------------------------------------------------

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn write_file_success() {
        let dir = TempDir::new().unwrap();
        let executor = make_executor(&dir, permissive_engine(), "allow", 1);
        let result = FunctionExecutor::execute(
            &executor,
            "write_file",
            r#"{"path": "out.txt", "content": "hello agent"}"#,
        );
        assert!(result.is_ok());
        let on_disk = fs::read_to_string(dir.path().join("out.txt")).unwrap();
        assert_eq!(on_disk, "hello agent");
    }

    // -----------------------------------------------------------------------
    // Policy denial
    // -----------------------------------------------------------------------

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn write_file_denied_by_policy() {
        let dir = TempDir::new().unwrap();
        // allow_workspace_writes = false
        let executor = make_executor(&dir, restrictive_engine(), "allow", 1);
        let result = FunctionExecutor::execute(
            &executor,
            "write_file",
            r#"{"path": "out.txt", "content": "no"}"#,
        );
        assert!(result.is_err());
        let err = result.unwrap_err().to_string();
        assert!(err.contains("policy denied"));
        assert!(err.contains("write_file"));
    }

    // -----------------------------------------------------------------------
    // Ask -> Allow with approval_mode="allow"
    // -----------------------------------------------------------------------

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn shell_command_allowed_with_approval_mode_allow() {
        let dir = TempDir::new().unwrap();
        // allow_shell=true produces Ask; approval_mode="allow" upgrades to Allow.
        let engine = PolicyEngine::new(PolicyConfig {
            allow_network: false,
            allow_shell: true,
            allow_workspace_writes: true,
            max_patch_bytes: 1024,
        });
        let executor = make_executor_with_engine(&dir, engine, "allow", 2);
        let result = FunctionExecutor::execute(
            &executor,
            "run_command",
            r#"{"command": "echo", "args": ["hello"]}"#,
        );
        assert!(result.is_ok(), "expected Ok, got: {result:?}");
    }

    // -----------------------------------------------------------------------
    // Ask -> Deny with approval_mode="deny"
    // -----------------------------------------------------------------------

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn shell_command_denied_with_approval_mode_deny() {
        let dir = TempDir::new().unwrap();
        let engine = PolicyEngine::new(PolicyConfig {
            allow_network: false,
            allow_shell: true,
            allow_workspace_writes: true,
            max_patch_bytes: 1024,
        });
        let executor = make_executor_with_engine(&dir, engine, "deny", 2);
        let result = FunctionExecutor::execute(
            &executor,
            "run_command",
            r#"{"command": "echo", "args": ["hello"]}"#,
        );
        assert!(result.is_err());
        let err = result.unwrap_err().to_string();
        assert!(err.contains("policy denied"));
    }

    // -----------------------------------------------------------------------
    // Ask -> Deny with approval_mode="interactive" (no broker yet)
    // -----------------------------------------------------------------------

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn shell_command_denied_with_approval_mode_interactive() {
        let dir = TempDir::new().unwrap();
        let engine = PolicyEngine::new(PolicyConfig {
            allow_network: false,
            allow_shell: true,
            allow_workspace_writes: true,
            max_patch_bytes: 1024,
        });
        let executor = make_executor_with_engine(&dir, engine, "interactive", 2);
        let result = FunctionExecutor::execute(
            &executor,
            "run_command",
            r#"{"command": "echo", "args": ["hello"]}"#,
        );
        assert!(result.is_err());
        let err = result.unwrap_err().to_string();
        assert!(err.contains("approval required"));
    }

    // -----------------------------------------------------------------------
    // Malformed JSON arguments
    // -----------------------------------------------------------------------

    #[test]
    fn malformed_json_returns_error() {
        let dir = TempDir::new().unwrap();
        let executor = make_executor(&dir, permissive_engine(), "allow", 0);
        let result = FunctionExecutor::execute(&executor, "read_file", "not valid json");
        assert!(result.is_err());
        let err = result.unwrap_err().to_string();
        assert!(err.contains("classify") || err.contains("deserialize"));
    }

    // -----------------------------------------------------------------------
    // Invalid path in tool input
    // -----------------------------------------------------------------------

    #[test]
    fn invalid_path_returns_error() {
        let dir = TempDir::new().unwrap();
        let executor = make_executor(&dir, permissive_engine(), "allow", 0);
        let result =
            FunctionExecutor::execute(&executor, "read_file", r#"{"path": "/etc/passwd"}"#);
        assert!(result.is_err());
        let err = result.unwrap_err().to_string();
        assert!(
            err.contains("classify") || err.contains("path") || err.contains("absolute"),
            "unexpected error: {err}"
        );
    }

    // -----------------------------------------------------------------------
    // Registry accessor
    // -----------------------------------------------------------------------

    #[test]
    fn registry_accessor_returns_registry() {
        let dir = TempDir::new().unwrap();
        let executor = make_executor(&dir, permissive_engine(), "allow", 2);
        assert_eq!(executor.registry().len(), 11);
        assert_eq!(executor.trust_rank(), 2);
    }

    // -----------------------------------------------------------------------
    // Helper
    // -----------------------------------------------------------------------

    fn make_executor_with_engine(
        dir: &TempDir,
        engine: PolicyEngine,
        approval_mode: &str,
        trust_rank: u8,
    ) -> PolicyGatedExecutor {
        PolicyGatedExecutor::new(
            default_registry(),
            engine,
            workspace(dir),
            approval_mode.to_owned(),
            trust_rank,
        )
    }
}
