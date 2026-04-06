use grokrs_cap::{WorkspacePath, WorkspaceRoot};
use grokrs_policy::Effect;
use serde_json::json;

use crate::error::ToolError;
use crate::{Classify, ToolSpec};

/// Maximum bytes to read from a single file (defense against OOM on huge files).
const MAX_READ_BYTES: u64 = 1_048_576; // 1 MiB

/// Input for the `read_file` tool.
#[derive(Debug, Clone, serde::Deserialize)]
pub struct ReadFileInput {
    /// Workspace-relative path to read.
    pub path: String,
}

impl Classify for ReadFileInput {
    fn classify(&self) -> Result<Vec<Effect>, ToolError> {
        let wp = WorkspacePath::new(&self.path)?;
        Ok(vec![Effect::FsRead(wp)])
    }
}

/// Reads a workspace-relative file and returns its contents as a string.
#[derive(Debug, Clone)]
pub struct ReadFileTool;

impl ToolSpec for ReadFileTool {
    type Input = ReadFileInput;
    type Output = String;

    fn name(&self) -> &'static str {
        "read_file"
    }

    fn description(&self) -> &'static str {
        "Read the contents of a file relative to the workspace root. \
         Returns the file content as a UTF-8 string."
    }

    fn input_schema(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "properties": {
                "path": {
                    "type": "string",
                    "description": "Workspace-relative file path to read"
                }
            },
            "required": ["path"],
            "additionalProperties": false
        })
    }

    async fn execute(
        &self,
        input: Self::Input,
        root: &WorkspaceRoot,
    ) -> Result<Self::Output, ToolError> {
        let wp = WorkspacePath::new(&input.path)?;
        let abs_path = root.join(&wp);

        // Resolve symlinks and verify the canonical path is still under the workspace root.
        let canonical = abs_path.canonicalize().map_err(|e| {
            ToolError::Io(std::io::Error::new(
                e.kind(),
                format!("{}: {}", abs_path.display(), e),
            ))
        })?;
        let canonical_root = root.as_path().canonicalize().map_err(|e| {
            ToolError::Io(std::io::Error::new(
                e.kind(),
                format!("workspace root {}: {}", root.as_path().display(), e),
            ))
        })?;
        if !canonical.starts_with(&canonical_root) {
            return Err(ToolError::PermissionDenied {
                operation: format!("read {}", input.path),
                reason: "resolved path escapes workspace root (symlink traversal)".into(),
            });
        }

        // Check file size before reading.
        let metadata = std::fs::metadata(&canonical).map_err(|e| {
            ToolError::Io(std::io::Error::new(
                e.kind(),
                format!("{}: {}", abs_path.display(), e),
            ))
        })?;
        if metadata.len() > MAX_READ_BYTES {
            return Err(ToolError::Other(format!(
                "file {} is {} bytes, exceeding the {MAX_READ_BYTES}-byte read limit",
                input.path,
                metadata.len()
            )));
        }

        let content = std::fs::read_to_string(&canonical).map_err(|e| {
            ToolError::Io(std::io::Error::new(
                e.kind(),
                format!("{}: {}", abs_path.display(), e),
            ))
        })?;
        Ok(content)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::TempDir;

    fn workspace(dir: &TempDir) -> WorkspaceRoot {
        WorkspaceRoot::new(dir.path()).unwrap()
    }

    #[test]
    fn classify_valid_path() {
        let input = ReadFileInput {
            path: "src/main.rs".into(),
        };
        let effects = input.classify().unwrap();
        assert_eq!(effects.len(), 1);
        assert!(
            matches!(&effects[0], Effect::FsRead(wp) if wp.as_path().to_str() == Some("src/main.rs"))
        );
    }

    #[test]
    fn classify_rejects_absolute_path() {
        let input = ReadFileInput {
            path: "/etc/passwd".into(),
        };
        let err = input.classify().unwrap_err();
        assert!(matches!(err, ToolError::PathValidation(_)));
    }

    #[test]
    fn classify_rejects_dotdot_traversal() {
        let input = ReadFileInput {
            path: "../outside".into(),
        };
        let err = input.classify().unwrap_err();
        assert!(matches!(err, ToolError::PathValidation(_)));
    }

    #[tokio::test]
    async fn reads_file_content() {
        let dir = TempDir::new().unwrap();
        let root = workspace(&dir);
        fs::write(dir.path().join("hello.txt"), "world").unwrap();

        let result = ReadFileTool
            .execute(
                ReadFileInput {
                    path: "hello.txt".into(),
                },
                &root,
            )
            .await
            .unwrap();
        assert_eq!(result, "world");
    }

    #[tokio::test]
    async fn returns_io_error_for_missing_file() {
        let dir = TempDir::new().unwrap();
        let root = workspace(&dir);

        let err = ReadFileTool
            .execute(
                ReadFileInput {
                    path: "missing.txt".into(),
                },
                &root,
            )
            .await
            .unwrap_err();
        assert!(matches!(err, ToolError::Io(_)));
    }

    #[tokio::test]
    async fn rejects_file_exceeding_max_size() {
        let dir = TempDir::new().unwrap();
        let root = workspace(&dir);
        // Write a file slightly over the limit.
        // RATIONALE: MAX_READ_BYTES is 1 MiB (1_048_576), well within usize
        // range on all supported platforms.
        #[allow(clippy::cast_possible_truncation)]
        let big = vec![b'x'; (MAX_READ_BYTES + 1) as usize];
        fs::write(dir.path().join("big.bin"), &big).unwrap();

        let err = ReadFileTool
            .execute(
                ReadFileInput {
                    path: "big.bin".into(),
                },
                &root,
            )
            .await
            .unwrap_err();
        assert!(matches!(err, ToolError::Other(_)));
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn rejects_symlink_escaping_workspace() {
        let dir = TempDir::new().unwrap();
        let root = workspace(&dir);
        // Create a symlink pointing outside the workspace.
        std::os::unix::fs::symlink("/etc/hostname", dir.path().join("escape")).unwrap();

        let err = ReadFileTool
            .execute(
                ReadFileInput {
                    path: "escape".into(),
                },
                &root,
            )
            .await
            .unwrap_err();
        assert!(
            matches!(err, ToolError::PermissionDenied { .. }),
            "expected PermissionDenied, got {err:?}"
        );
    }

    #[test]
    fn has_description_and_schema() {
        let tool = ReadFileTool;
        assert!(!tool.description().is_empty());
        let schema = tool.input_schema();
        assert_eq!(schema["type"], "object");
        assert!(schema["properties"]["path"].is_object());
    }
}
