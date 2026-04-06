use grokrs_cap::{WorkspacePath, WorkspaceRoot};
use grokrs_policy::Effect;
use serde_json::json;

use crate::error::ToolError;
use crate::{Classify, ToolSpec};

/// Input for the `write_file` tool.
#[derive(Debug, Clone, serde::Deserialize)]
pub struct WriteFileInput {
    /// Workspace-relative path to write.
    pub path: String,
    /// Content to write to the file.
    pub content: String,
}

impl Classify for WriteFileInput {
    fn classify(&self) -> Result<Vec<Effect>, ToolError> {
        let wp = WorkspacePath::new(&self.path)?;
        Ok(vec![Effect::FsWrite(wp)])
    }
}

/// Writes content to a workspace-relative file, creating parent directories
/// if they don't exist.
#[derive(Debug, Clone)]
pub struct WriteFileTool;

impl ToolSpec for WriteFileTool {
    type Input = WriteFileInput;
    type Output = String;

    fn name(&self) -> &'static str {
        "write_file"
    }

    fn description(&self) -> &'static str {
        "Write content to a file relative to the workspace root. \
         Creates parent directories if they do not exist."
    }

    fn input_schema(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "properties": {
                "path": {
                    "type": "string",
                    "description": "Workspace-relative file path to write"
                },
                "content": {
                    "type": "string",
                    "description": "Content to write to the file"
                }
            },
            "required": ["path", "content"],
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

        // Resolve symlinks in existing path components BEFORE creating any
        // directories. Walk up from the target path to find the deepest
        // existing ancestor, canonicalize it, and verify it's within the
        // workspace root. This prevents create_dir_all from creating
        // directories through symlinked intermediate paths.
        {
            let canonical_root = root.as_path().canonicalize().map_err(|e| {
                ToolError::Io(std::io::Error::new(
                    e.kind(),
                    format!("workspace root {}: {}", root.as_path().display(), e),
                ))
            })?;
            // Find the deepest existing ancestor of the target path.
            let mut check = abs_path.as_path();
            while !check.exists() {
                match check.parent() {
                    Some(p) => check = p,
                    None => break,
                }
            }
            if check.exists() {
                let canonical_ancestor = check.canonicalize().map_err(|e| {
                    ToolError::Io(std::io::Error::new(
                        e.kind(),
                        format!("{}: {}", check.display(), e),
                    ))
                })?;
                if !canonical_ancestor.starts_with(&canonical_root) {
                    return Err(ToolError::PermissionDenied {
                        operation: format!("write {}", input.path),
                        reason: "resolved path escapes workspace root (symlink traversal)".into(),
                    });
                }
            }
        }

        // Now safe to create parent directories — all existing ancestors
        // have been verified to be within the workspace.
        if let Some(parent) = abs_path.parent() {
            std::fs::create_dir_all(parent).map_err(|e| {
                ToolError::Io(std::io::Error::new(
                    e.kind(),
                    format!("creating directories for {}: {}", abs_path.display(), e),
                ))
            })?;
        }
        // Fully resolve symlink chains to the final target. This handles:
        // - Direct symlinks (escape → /outside/file.txt)
        // - Dangling symlinks (escape → /outside/nonexistent.txt)
        // - Chained symlinks (escape → inner → /outside/file.txt)
        // - Symlinked directories (link/ → /outside/)
        //
        // We follow read_link() repeatedly (up to a depth limit) to find
        // the final target, then verify it (or its deepest existing
        // ancestor) is within the workspace root.
        let final_target = resolve_through_symlinks(&abs_path, 32)?;
        if final_target != abs_path {
            let canonical_root = root.as_path().canonicalize().map_err(|e| {
                ToolError::Io(std::io::Error::new(
                    e.kind(),
                    format!("workspace root {}: {}", root.as_path().display(), e),
                ))
            })?;
            // Find the deepest existing ancestor of the final target.
            let check_path = if final_target.exists() {
                final_target.canonicalize().unwrap_or(final_target.clone())
            } else {
                let mut ancestor = final_target.as_path();
                while !ancestor.exists() {
                    match ancestor.parent() {
                        Some(p) => ancestor = p,
                        None => break,
                    }
                }
                if ancestor.exists() {
                    ancestor.canonicalize().unwrap_or(ancestor.to_path_buf())
                } else {
                    final_target.clone()
                }
            };
            if !check_path.starts_with(&canonical_root) {
                return Err(ToolError::PermissionDenied {
                    operation: format!("write {}", input.path),
                    reason: "resolved path escapes workspace root (symlink traversal)".into(),
                });
            }
        }

        // Also check existing non-symlink files via canonicalize.
        if abs_path.exists() && !abs_path.is_symlink() {
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
                    operation: format!("write {}", input.path),
                    reason: "resolved path escapes workspace root (symlink traversal)".into(),
                });
            }
        }

        let bytes_written = input.content.len();
        std::fs::write(&abs_path, &input.content).map_err(|e| {
            ToolError::Io(std::io::Error::new(
                e.kind(),
                format!("{}: {}", abs_path.display(), e),
            ))
        })?;

        Ok(format!("wrote {} bytes to {}", bytes_written, input.path))
    }
}

/// Follow symlink chains to the final target path.
///
/// Repeatedly calls `read_link` until the path is no longer a symlink
/// or `max_depth` is reached. Returns the final resolved path (which
/// may not exist on disk for dangling symlinks). Relative symlink
/// targets are resolved against the parent of the current link.
fn resolve_through_symlinks(
    path: &std::path::Path,
    max_depth: u32,
) -> Result<std::path::PathBuf, ToolError> {
    let mut current = path.to_path_buf();
    for _ in 0..max_depth {
        // Use symlink_metadata to detect symlinks (even dangling ones).
        match std::fs::symlink_metadata(&current) {
            Ok(meta) if meta.file_type().is_symlink() => {
                let target = std::fs::read_link(&current).map_err(|e| {
                    ToolError::Io(std::io::Error::new(
                        e.kind(),
                        format!("reading symlink {}: {}", current.display(), e),
                    ))
                })?;
                current = if target.is_absolute() {
                    target
                } else {
                    current
                        .parent()
                        .unwrap_or(std::path::Path::new("/"))
                        .join(&target)
                };
            }
            _ => return Ok(current), // Not a symlink (or doesn't exist) — done.
        }
    }
    // After max_depth iterations, check if we're still on a symlink.
    // If not, we successfully resolved (the last hop landed on a real path).
    match std::fs::symlink_metadata(&current) {
        Ok(meta) if meta.file_type().is_symlink() => {
            // Still a symlink after exhausting the budget — fail-closed.
            Err(ToolError::PermissionDenied {
                operation: format!("resolve symlinks for {}", path.display()),
                reason: format!(
                    "symlink chain exceeds maximum depth of {max_depth} hops (possible symlink loop)"
                ),
            })
        }
        _ => Ok(current), // Resolved to a non-symlink (or non-existent) — done.
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
        let input = WriteFileInput {
            path: "out/result.txt".into(),
            content: "data".into(),
        };
        let effects = input.classify().unwrap();
        assert_eq!(effects.len(), 1);
        assert!(
            matches!(&effects[0], Effect::FsWrite(wp) if wp.as_path().to_str() == Some("out/result.txt"))
        );
    }

    #[test]
    fn classify_rejects_absolute_path() {
        let input = WriteFileInput {
            path: "/tmp/evil".into(),
            content: "data".into(),
        };
        assert!(matches!(
            input.classify().unwrap_err(),
            ToolError::PathValidation(_)
        ));
    }

    #[test]
    fn classify_rejects_dotdot() {
        let input = WriteFileInput {
            path: "../escape".into(),
            content: "data".into(),
        };
        assert!(matches!(
            input.classify().unwrap_err(),
            ToolError::PathValidation(_)
        ));
    }

    #[tokio::test]
    async fn writes_file_content() {
        let dir = TempDir::new().unwrap();
        let root = workspace(&dir);

        let result = WriteFileTool
            .execute(
                WriteFileInput {
                    path: "hello.txt".into(),
                    content: "world".into(),
                },
                &root,
            )
            .await
            .unwrap();
        assert!(result.contains("5 bytes"));

        let content = fs::read_to_string(dir.path().join("hello.txt")).unwrap();
        assert_eq!(content, "world");
    }

    #[tokio::test]
    async fn creates_parent_directories() {
        let dir = TempDir::new().unwrap();
        let root = workspace(&dir);

        WriteFileTool
            .execute(
                WriteFileInput {
                    path: "deep/nested/dir/file.txt".into(),
                    content: "nested".into(),
                },
                &root,
            )
            .await
            .unwrap();

        let content = fs::read_to_string(dir.path().join("deep/nested/dir/file.txt")).unwrap();
        assert_eq!(content, "nested");
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn rejects_symlink_escaping_workspace() {
        let dir = TempDir::new().unwrap();
        let root = workspace(&dir);
        let outside = TempDir::new().unwrap();
        let target = outside.path().join("target.txt");
        fs::write(&target, "original").unwrap();
        std::os::unix::fs::symlink(&target, dir.path().join("escape")).unwrap();

        let err = WriteFileTool
            .execute(
                WriteFileInput {
                    path: "escape".into(),
                    content: "pwned".into(),
                },
                &root,
            )
            .await
            .unwrap_err();
        assert!(matches!(err, ToolError::PermissionDenied { .. }));
        // Verify original file was not modified.
        assert_eq!(fs::read_to_string(&target).unwrap(), "original");
    }

    #[tokio::test]
    async fn rejects_dangling_symlink_escaping_workspace() {
        let dir = TempDir::new().unwrap();
        let root = workspace(&dir);
        let outside = TempDir::new().unwrap();
        // Create a dangling symlink: target doesn't exist yet
        let target = outside.path().join("dangling.txt");
        std::os::unix::fs::symlink(&target, dir.path().join("escape_dangling")).unwrap();

        let err = WriteFileTool
            .execute(
                WriteFileInput {
                    path: "escape_dangling".into(),
                    content: "pwned".into(),
                },
                &root,
            )
            .await
            .unwrap_err();
        assert!(matches!(err, ToolError::PermissionDenied { .. }));
        // Verify the file was NOT created at the outside target.
        assert!(!target.exists());
    }

    #[tokio::test]
    async fn rejects_symlink_dir_traversal_to_new_file() {
        let dir = TempDir::new().unwrap();
        let root = workspace(&dir);
        let outside = TempDir::new().unwrap();
        // Create a symlink to an outside directory
        std::os::unix::fs::symlink(outside.path(), dir.path().join("link")).unwrap();

        let err = WriteFileTool
            .execute(
                WriteFileInput {
                    path: "link/new.txt".into(),
                    content: "pwned".into(),
                },
                &root,
            )
            .await
            .unwrap_err();
        assert!(matches!(err, ToolError::PermissionDenied { .. }));
        // Verify no file was created outside
        assert!(!outside.path().join("new.txt").exists());
    }

    #[tokio::test]
    async fn rejects_chained_dangling_symlink_escape() {
        let dir = TempDir::new().unwrap();
        let root = workspace(&dir);
        let outside = TempDir::new().unwrap();
        // Chain: escape → inner → /outside/dangling.txt
        std::os::unix::fs::symlink(
            outside.path().join("dangling.txt"),
            dir.path().join("inner"),
        )
        .unwrap();
        std::os::unix::fs::symlink("inner", dir.path().join("escape")).unwrap();

        let err = WriteFileTool
            .execute(
                WriteFileInput {
                    path: "escape".into(),
                    content: "pwned".into(),
                },
                &root,
            )
            .await
            .unwrap_err();
        assert!(matches!(err, ToolError::PermissionDenied { .. }));
        // Verify no file created outside workspace.
        assert!(!outside.path().join("dangling.txt").exists());
    }

    #[test]
    fn has_description_and_schema() {
        let tool = WriteFileTool;
        assert!(!tool.description().is_empty());
        let schema = tool.input_schema();
        assert_eq!(schema["type"], "object");
        assert!(schema["properties"]["path"].is_object());
        assert!(schema["properties"]["content"].is_object());
    }
}
