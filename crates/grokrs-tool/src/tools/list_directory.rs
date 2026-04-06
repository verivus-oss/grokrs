use grokrs_cap::{WorkspacePath, WorkspaceRoot};
use grokrs_policy::Effect;
use serde_json::json;

use crate::error::ToolError;
use crate::{Classify, ToolSpec};

/// Input for the `list_directory` tool.
#[derive(Debug, Clone, serde::Deserialize)]
pub struct ListDirectoryInput {
    /// Workspace-relative directory path to list.
    pub path: String,
}

impl Classify for ListDirectoryInput {
    fn classify(&self) -> Result<Vec<Effect>, ToolError> {
        let wp = WorkspacePath::new(&self.path)?;
        Ok(vec![Effect::FsRead(wp)])
    }
}

/// Lists entries in a workspace-relative directory, returning sorted names.
#[derive(Debug, Clone)]
pub struct ListDirectoryTool;

impl ToolSpec for ListDirectoryTool {
    type Input = ListDirectoryInput;
    type Output = String;

    fn name(&self) -> &'static str {
        "list_directory"
    }

    fn description(&self) -> &str {
        "List the contents of a directory relative to the workspace root. \
         Returns a sorted, newline-separated list of entry names."
    }

    fn input_schema(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "properties": {
                "path": {
                    "type": "string",
                    "description": "Workspace-relative directory path to list"
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
                operation: format!("list {}", input.path),
                reason: "resolved path escapes workspace root (symlink traversal)".into(),
            });
        }

        let read_dir = std::fs::read_dir(&canonical).map_err(|e| {
            ToolError::Io(std::io::Error::new(
                e.kind(),
                format!("{}: {}", abs_path.display(), e),
            ))
        })?;

        let mut entries: Vec<String> = Vec::new();
        for entry_result in read_dir {
            let entry = entry_result.map_err(|e| {
                ToolError::Io(std::io::Error::new(
                    e.kind(),
                    format!("reading entry in {}: {}", abs_path.display(), e),
                ))
            })?;
            if let Some(name) = entry.file_name().to_str() {
                entries.push(name.to_string());
            } else {
                // Non-UTF-8 filenames: use lossy representation.
                entries.push(entry.file_name().to_string_lossy().into_owned());
            }
        }
        entries.sort();

        Ok(entries.join("\n"))
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
        let input = ListDirectoryInput { path: "src".into() };
        let effects = input.classify().unwrap();
        assert_eq!(effects.len(), 1);
        assert!(matches!(&effects[0], Effect::FsRead(wp) if wp.as_path().to_str() == Some("src")));
    }

    #[test]
    fn classify_rejects_absolute_path() {
        let input = ListDirectoryInput {
            path: "/etc".into(),
        };
        assert!(matches!(
            input.classify().unwrap_err(),
            ToolError::PathValidation(_)
        ));
    }

    #[test]
    fn classify_rejects_dotdot() {
        let input = ListDirectoryInput { path: "..".into() };
        assert!(matches!(
            input.classify().unwrap_err(),
            ToolError::PathValidation(_)
        ));
    }

    #[tokio::test]
    async fn lists_directory_sorted() {
        let dir = TempDir::new().unwrap();
        let root = workspace(&dir);
        let sub = dir.path().join("mydir");
        fs::create_dir(&sub).unwrap();
        fs::write(sub.join("zebra.txt"), "").unwrap();
        fs::write(sub.join("alpha.txt"), "").unwrap();
        fs::write(sub.join("mid.txt"), "").unwrap();

        let result = ListDirectoryTool
            .execute(
                ListDirectoryInput {
                    path: "mydir".into(),
                },
                &root,
            )
            .await
            .unwrap();
        assert_eq!(result, "alpha.txt\nmid.txt\nzebra.txt");
    }

    #[tokio::test]
    async fn returns_io_error_for_missing_directory() {
        let dir = TempDir::new().unwrap();
        let root = workspace(&dir);

        let err = ListDirectoryTool
            .execute(
                ListDirectoryInput {
                    path: "nope".into(),
                },
                &root,
            )
            .await
            .unwrap_err();
        assert!(matches!(err, ToolError::Io(_)));
    }

    #[tokio::test]
    async fn lists_empty_directory() {
        let dir = TempDir::new().unwrap();
        let root = workspace(&dir);
        fs::create_dir(dir.path().join("empty")).unwrap();

        let result = ListDirectoryTool
            .execute(
                ListDirectoryInput {
                    path: "empty".into(),
                },
                &root,
            )
            .await
            .unwrap();
        assert_eq!(result, "");
    }

    #[test]
    fn has_description_and_schema() {
        let tool = ListDirectoryTool;
        assert!(!tool.description().is_empty());
        let schema = tool.input_schema();
        assert_eq!(schema["type"], "object");
        assert!(schema["properties"]["path"].is_object());
    }
}
