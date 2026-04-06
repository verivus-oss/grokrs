use std::time::Duration;

use grokrs_cap::WorkspaceRoot;
use grokrs_policy::Effect;
use serde_json::json;

use crate::error::ToolError;
use crate::{Classify, ToolSpec};

/// Default timeout for command execution.
const DEFAULT_TIMEOUT: Duration = Duration::from_secs(30);

/// Environment variables that are safe to inherit into the child process.
const ALLOWED_ENV_VARS: &[&str] = &["PATH", "HOME", "USER", "LANG", "TERM"];

/// Patterns in environment variable names that indicate secrets.
/// Any env var whose name ends with one of these suffixes is stripped.
const SECRET_SUFFIXES: &[&str] = &["_KEY", "_SECRET", "_TOKEN"];

/// Exact env var names that are always stripped regardless of suffix matching.
const SECRET_EXACT: &[&str] = &["XAI_API_KEY"];

/// Input for the `run_command` tool.
#[derive(Debug, Clone, Default, serde::Deserialize)]
#[serde(default)]
pub struct RunCommandInput {
    /// The program to execute.
    pub command: String,
    /// Arguments to pass to the command.
    pub args: Vec<String>,
}

impl RunCommandInput {
    /// Extract the program name (first whitespace-delimited token of `command`).
    fn program(&self) -> &str {
        self.command
            .split_whitespace()
            .next()
            .unwrap_or(&self.command)
    }
}

impl Classify for RunCommandInput {
    fn classify(&self) -> Result<Vec<Effect>, ToolError> {
        let program = self.program();
        if program.is_empty() {
            return Err(ToolError::Other("command must not be empty".into()));
        }
        Ok(vec![Effect::ProcessSpawn {
            program: program.to_string(),
        }])
    }
}

/// Executes a shell command in the workspace root with environment sanitization,
/// timeout enforcement, and combined stdout/stderr capture.
#[derive(Debug, Clone)]
pub struct RunCommandTool {
    /// Maximum time the command is allowed to run before being killed.
    pub timeout: Duration,
}

impl Default for RunCommandTool {
    fn default() -> Self {
        Self {
            timeout: DEFAULT_TIMEOUT,
        }
    }
}

impl RunCommandTool {
    /// Create a new `RunCommandTool` with a custom timeout.
    pub fn with_timeout(timeout: Duration) -> Self {
        Self { timeout }
    }

    /// Build a sanitized environment map for the child process.
    ///
    /// Starts from the current process environment, keeps only the allowed
    /// variables, and then strips any that match secret patterns.
    fn build_sanitized_env() -> Vec<(String, String)> {
        let mut env: Vec<(String, String)> = Vec::new();

        for key in ALLOWED_ENV_VARS {
            if let Ok(val) = std::env::var(key) {
                env.push(((*key).to_string(), val));
            }
        }

        // Remove any that accidentally match secret patterns
        // (shouldn't happen with the allowlist, but defense in depth).
        env.retain(|(name, _)| !is_secret_var(name));

        env
    }
}

/// Returns `true` if an environment variable name looks like a secret.
fn is_secret_var(name: &str) -> bool {
    let upper = name.to_uppercase();

    // Exact matches.
    if SECRET_EXACT.iter().any(|&exact| upper == exact) {
        return true;
    }

    // Suffix matches.
    SECRET_SUFFIXES
        .iter()
        .any(|&suffix| upper.ends_with(suffix))
}

impl ToolSpec for RunCommandTool {
    type Input = RunCommandInput;
    type Output = String;

    fn name(&self) -> &'static str {
        "run_command"
    }

    fn description(&self) -> &str {
        "Execute a command in the workspace root directory. \
         The environment is sanitized to prevent secret leakage. \
         Returns combined stdout and stderr with the exit code."
    }

    fn input_schema(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "properties": {
                "command": {
                    "type": "string",
                    "description": "The program to execute"
                },
                "args": {
                    "type": "array",
                    "items": { "type": "string" },
                    "description": "Arguments to pass to the command"
                }
            },
            "required": ["command"],
            "additionalProperties": false
        })
    }

    async fn execute(
        &self,
        input: Self::Input,
        root: &WorkspaceRoot,
    ) -> Result<Self::Output, ToolError> {
        let program = input.program().to_string();
        if program.is_empty() {
            return Err(ToolError::Other("command must not be empty".into()));
        }

        let sanitized_env = Self::build_sanitized_env();
        let timeout = self.timeout;

        // Spawn in a blocking context since std::process::Command is blocking.
        // We use tokio::task::spawn_blocking + manual timeout.
        let cwd = root.as_path().to_path_buf();
        let args = input.args.clone();

        let handle = tokio::task::spawn_blocking(move || {
            let mut cmd = std::process::Command::new(&program);
            cmd.args(&args)
                .current_dir(&cwd)
                .stdin(std::process::Stdio::null())
                .stdout(std::process::Stdio::piped())
                .stderr(std::process::Stdio::piped())
                .env_clear();

            for (key, val) in &sanitized_env {
                cmd.env(key, val);
            }

            let mut child = cmd.spawn().map_err(|e| {
                ToolError::Io(std::io::Error::new(
                    e.kind(),
                    format!("spawning '{}': {}", program, e),
                ))
            })?;

            // Take stdout/stderr handles and read them in separate threads
            // to avoid deadlock when pipe buffers fill up.
            let stdout_handle = child.stdout.take();
            let stderr_handle = child.stderr.take();

            let stdout_thread = std::thread::spawn(move || {
                let mut buf = String::new();
                if let Some(mut out) = stdout_handle {
                    let _ = std::io::Read::read_to_string(&mut out, &mut buf);
                }
                buf
            });

            let stderr_thread = std::thread::spawn(move || {
                let mut buf = String::new();
                if let Some(mut err) = stderr_handle {
                    let _ = std::io::Read::read_to_string(&mut err, &mut buf);
                }
                buf
            });

            // Wait for the child with timeout using a polling loop.
            let start = std::time::Instant::now();
            let poll_interval = Duration::from_millis(50);

            let exit_status = loop {
                match child.try_wait() {
                    Ok(Some(status)) => break status,
                    Ok(None) => {
                        if start.elapsed() >= timeout {
                            let _ = child.kill();
                            let _ = child.wait(); // Reap zombie.
                            return Err(ToolError::Timeout {
                                operation: format!("command '{program}'"),
                                duration: timeout,
                            });
                        }
                        std::thread::sleep(poll_interval);
                    }
                    Err(e) => {
                        return Err(ToolError::Io(std::io::Error::new(
                            e.kind(),
                            format!("waiting for '{}': {}", program, e),
                        )));
                    }
                }
            };

            let stdout_buf = stdout_thread.join().unwrap_or_default();
            let stderr_buf = stderr_thread.join().unwrap_or_default();

            let exit_code = exit_status.code().unwrap_or(-1);
            let mut output = String::new();
            if !stdout_buf.is_empty() {
                output.push_str(&stdout_buf);
            }
            if !stderr_buf.is_empty() {
                if !output.is_empty() {
                    output.push('\n');
                }
                output.push_str(&stderr_buf);
            }
            output.push_str(&format!("\n[exit code: {exit_code}]"));
            Ok(output)
        });

        match handle.await {
            Ok(result) => result,
            Err(e) => Err(ToolError::Other(format!("task join error: {e}"))),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn workspace(dir: &TempDir) -> WorkspaceRoot {
        WorkspaceRoot::new(dir.path()).unwrap()
    }

    #[test]
    fn classify_extracts_program_name() {
        let input = RunCommandInput {
            command: "ls".into(),
            args: vec!["-la".into()],
        };
        let effects = input.classify().unwrap();
        assert_eq!(effects.len(), 1);
        assert!(matches!(&effects[0], Effect::ProcessSpawn { program } if program == "ls"));
    }

    #[test]
    fn classify_extracts_first_token_from_compound_command() {
        let input = RunCommandInput {
            command: "cargo".into(),
            args: vec!["build".into(), "--release".into()],
        };
        let effects = input.classify().unwrap();
        assert!(matches!(&effects[0], Effect::ProcessSpawn { program } if program == "cargo"));
    }

    #[test]
    fn classify_rejects_empty_command() {
        let input = RunCommandInput {
            command: "".into(),
            args: vec![],
        };
        let err = input.classify().unwrap_err();
        assert!(matches!(err, ToolError::Other(_)));
    }

    #[test]
    fn is_secret_var_detects_patterns() {
        assert!(is_secret_var("XAI_API_KEY"));
        assert!(is_secret_var("AWS_SECRET"));
        assert!(is_secret_var("GITHUB_TOKEN"));
        assert!(is_secret_var("my_api_key")); // case-insensitive
        assert!(!is_secret_var("PATH"));
        assert!(!is_secret_var("HOME"));
        assert!(!is_secret_var("LANG"));
        assert!(!is_secret_var("KEYBOARD")); // contains KEY but doesn't end with _KEY
    }

    #[tokio::test]
    async fn echo_command_returns_expected_output() {
        let dir = TempDir::new().unwrap();
        let root = workspace(&dir);

        let result = RunCommandTool::default()
            .execute(
                RunCommandInput {
                    command: "echo".into(),
                    args: vec!["hello world".into()],
                },
                &root,
            )
            .await
            .unwrap();
        assert!(result.contains("hello world"), "output: {result}");
        assert!(result.contains("[exit code: 0]"), "output: {result}");
    }

    #[tokio::test]
    async fn nonzero_exit_is_not_an_error() {
        let dir = TempDir::new().unwrap();
        let root = workspace(&dir);

        let result = RunCommandTool::default()
            .execute(
                RunCommandInput {
                    command: "false".into(),
                    args: vec![],
                },
                &root,
            )
            .await
            .unwrap();
        assert!(
            result.contains("[exit code: 1]"),
            "non-zero exit should still return Ok, got: {result}"
        );
    }

    #[tokio::test]
    async fn timeout_fires_for_sleep_command() {
        let dir = TempDir::new().unwrap();
        let root = workspace(&dir);

        let tool = RunCommandTool::with_timeout(Duration::from_millis(200));
        let err = tool
            .execute(
                RunCommandInput {
                    command: "sleep".into(),
                    args: vec!["60".into()],
                },
                &root,
            )
            .await
            .unwrap_err();
        assert!(
            matches!(err, ToolError::Timeout { .. }),
            "expected Timeout, got: {err:?}"
        );
    }

    #[tokio::test]
    async fn env_sanitization_removes_secrets() {
        let dir = TempDir::new().unwrap();
        let root = workspace(&dir);

        // Set a secret env var that should be stripped.
        std::env::set_var("TEST_GROKRS_SECRET", "supersecret");
        std::env::set_var("MY_API_KEY", "should_not_appear");

        let result = RunCommandTool::default()
            .execute(
                RunCommandInput {
                    command: "env".into(),
                    args: vec![],
                },
                &root,
            )
            .await
            .unwrap();

        assert!(
            !result.contains("supersecret"),
            "secret value should not appear in child env: {result}"
        );
        assert!(
            !result.contains("MY_API_KEY"),
            "secret key var should not appear in child env: {result}"
        );
        assert!(
            !result.contains("TEST_GROKRS_SECRET"),
            "non-allowed env var should not appear in child env: {result}"
        );

        // Clean up.
        std::env::remove_var("TEST_GROKRS_SECRET");
        std::env::remove_var("MY_API_KEY");
    }

    #[tokio::test]
    async fn cwd_is_workspace_root() {
        let dir = TempDir::new().unwrap();
        let root = workspace(&dir);

        let result = RunCommandTool::default()
            .execute(
                RunCommandInput {
                    command: "pwd".into(),
                    args: vec![],
                },
                &root,
            )
            .await
            .unwrap();

        // Canonicalize both for comparison (tempdir may use symlinks).
        let expected = dir.path().canonicalize().unwrap();
        let actual_line = result.lines().next().unwrap_or("");
        let actual = std::path::Path::new(actual_line)
            .canonicalize()
            .unwrap_or_default();
        assert_eq!(
            actual, expected,
            "cwd should be workspace root, got: {result}"
        );
    }

    #[tokio::test]
    async fn captures_stderr() {
        let dir = TempDir::new().unwrap();
        let root = workspace(&dir);

        let result = RunCommandTool::default()
            .execute(
                RunCommandInput {
                    command: "sh".into(),
                    args: vec!["-c".into(), "echo stdout_msg; echo stderr_msg >&2".into()],
                },
                &root,
            )
            .await
            .unwrap();
        assert!(result.contains("stdout_msg"), "output: {result}");
        assert!(result.contains("stderr_msg"), "output: {result}");
    }

    #[test]
    fn has_description_and_schema() {
        let tool = RunCommandTool::default();
        assert!(!tool.description().is_empty());
        let schema = tool.input_schema();
        assert_eq!(schema["type"], "object");
        assert!(schema["properties"]["command"].is_object());
        assert!(schema["properties"]["args"].is_object());
    }
}
