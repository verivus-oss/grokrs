use grokrs_cap::{WorkspacePath, WorkspaceRoot};
use grokrs_policy::Effect;
use serde_json::json;

use crate::error::ToolError;
use crate::{Classify, ToolSpec};

/// Input for the `git_add` tool.
#[derive(Debug, Clone, serde::Deserialize)]
pub struct GitAddInput {
    /// Workspace-relative paths to stage. Each path is validated through
    /// `WorkspacePath` before being added to the index.
    pub paths: Vec<String>,
}

impl Classify for GitAddInput {
    fn classify(&self) -> Result<Vec<Effect>, ToolError> {
        if self.paths.is_empty() {
            return Err(ToolError::Other("paths must not be empty".into()));
        }
        let mut effects = Vec::with_capacity(self.paths.len());
        for p in &self.paths {
            let wp = WorkspacePath::new(p)?;
            effects.push(Effect::FsWrite(wp));
        }
        Ok(effects)
    }
}

/// Stages specified files in the git index (equivalent to `git add`).
///
/// Each path is validated as a workspace-relative path before staging. Files
/// that do not exist on disk or are outside the workspace are rejected.
#[derive(Debug, Clone)]
pub struct GitAddTool;

impl ToolSpec for GitAddTool {
    type Input = GitAddInput;
    type Output = String;

    fn name(&self) -> &'static str {
        "git_add"
    }

    fn description(&self) -> &str {
        "Stage specified files in the git index (equivalent to 'git add'). \
         Accepts a list of workspace-relative paths. Each path is validated \
         before staging."
    }

    fn input_schema(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "properties": {
                "paths": {
                    "type": "array",
                    "items": { "type": "string" },
                    "description": "Workspace-relative file paths to stage"
                }
            },
            "required": ["paths"],
            "additionalProperties": false
        })
    }

    async fn execute(
        &self,
        input: Self::Input,
        root: &WorkspaceRoot,
    ) -> Result<Self::Output, ToolError> {
        if input.paths.is_empty() {
            return Err(ToolError::Other("paths must not be empty".into()));
        }

        let repo = super::git_status::open_repo(root)?;
        let mut index = repo
            .index()
            .map_err(|e| ToolError::Other(format!("failed to open git index: {e}")))?;

        let mut staged = Vec::new();
        for p in &input.paths {
            // Validate through WorkspacePath first — rejects absolute, `..`, empty.
            let wp = WorkspacePath::new(p)?;

            // Verify the file exists on disk within the workspace before staging.
            let abs_path = root.join(&wp);
            if !abs_path.exists() {
                return Err(ToolError::Other(format!(
                    "cannot stage '{}': file does not exist",
                    p
                )));
            }

            // Symlink escape check: canonicalize and verify under workspace root.
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
                    operation: format!("stage {}", p),
                    reason: "resolved path escapes workspace root (symlink traversal)".into(),
                });
            }

            index
                .add_path(std::path::Path::new(wp.as_path()))
                .map_err(|e| ToolError::Other(format!("failed to stage '{}': {}", p, e)))?;
            staged.push(p.clone());
        }

        index
            .write()
            .map_err(|e| ToolError::Other(format!("failed to write index: {e}")))?;

        Ok(format!(
            "staged {} file(s): {}",
            staged.len(),
            staged.join(", ")
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn init_repo() -> (TempDir, WorkspaceRoot) {
        let dir = TempDir::new().unwrap();
        git2::Repository::init(dir.path()).unwrap();
        let root = WorkspaceRoot::new(dir.path()).unwrap();
        (dir, root)
    }

    #[test]
    fn classify_produces_fs_write_per_path() {
        let input = GitAddInput {
            paths: vec!["a.txt".into(), "b.txt".into()],
        };
        let effects = input.classify().unwrap();
        assert_eq!(effects.len(), 2);
        assert!(
            matches!(&effects[0], Effect::FsWrite(wp) if wp.as_path().to_str() == Some("a.txt"))
        );
        assert!(
            matches!(&effects[1], Effect::FsWrite(wp) if wp.as_path().to_str() == Some("b.txt"))
        );
    }

    #[test]
    fn classify_rejects_empty_paths() {
        let input = GitAddInput { paths: vec![] };
        let err = input.classify().unwrap_err();
        assert!(matches!(err, ToolError::Other(_)));
    }

    #[test]
    fn classify_rejects_absolute_path() {
        let input = GitAddInput {
            paths: vec!["/etc/passwd".into()],
        };
        let err = input.classify().unwrap_err();
        assert!(matches!(err, ToolError::PathValidation(_)));
    }

    #[test]
    fn classify_rejects_dotdot_traversal() {
        let input = GitAddInput {
            paths: vec!["../escape".into()],
        };
        let err = input.classify().unwrap_err();
        assert!(matches!(err, ToolError::PathValidation(_)));
    }

    #[tokio::test]
    async fn stages_a_file() {
        let (dir, root) = init_repo();
        std::fs::write(dir.path().join("new.txt"), "content").unwrap();

        let result = GitAddTool
            .execute(
                GitAddInput {
                    paths: vec!["new.txt".into()],
                },
                &root,
            )
            .await
            .unwrap();
        assert!(result.contains("1 file(s)"), "result: {result}");
        assert!(result.contains("new.txt"), "result: {result}");

        // Verify the file is staged in the index.
        let repo = git2::Repository::open(dir.path()).unwrap();
        let statuses = repo.statuses(None).unwrap();
        let entry = statuses
            .iter()
            .find(|e| e.path() == Some("new.txt"))
            .unwrap();
        assert!(entry.status().contains(git2::Status::INDEX_NEW));
    }

    #[tokio::test]
    async fn stages_multiple_files() {
        let (dir, root) = init_repo();
        std::fs::write(dir.path().join("a.txt"), "a").unwrap();
        std::fs::write(dir.path().join("b.txt"), "b").unwrap();

        let result = GitAddTool
            .execute(
                GitAddInput {
                    paths: vec!["a.txt".into(), "b.txt".into()],
                },
                &root,
            )
            .await
            .unwrap();
        assert!(result.contains("2 file(s)"), "result: {result}");
    }

    #[tokio::test]
    async fn rejects_nonexistent_file() {
        let (_dir, root) = init_repo();
        let err = GitAddTool
            .execute(
                GitAddInput {
                    paths: vec!["missing.txt".into()],
                },
                &root,
            )
            .await
            .unwrap_err();
        assert!(
            matches!(&err, ToolError::Other(msg) if msg.contains("does not exist")),
            "expected does-not-exist error, got: {err:?}"
        );
    }

    #[tokio::test]
    async fn rejects_empty_paths_at_execute() {
        let (_dir, root) = init_repo();
        let err = GitAddTool
            .execute(GitAddInput { paths: vec![] }, &root)
            .await
            .unwrap_err();
        assert!(matches!(err, ToolError::Other(_)));
    }

    #[tokio::test]
    async fn error_when_not_a_repo() {
        let dir = TempDir::new().unwrap();
        let root = WorkspaceRoot::new(dir.path()).unwrap();
        std::fs::write(dir.path().join("file.txt"), "data").unwrap();
        let err = GitAddTool
            .execute(
                GitAddInput {
                    paths: vec!["file.txt".into()],
                },
                &root,
            )
            .await
            .unwrap_err();
        assert!(
            matches!(&err, ToolError::Other(msg) if msg.contains("not a git repository")),
            "expected not-a-repo error, got: {err:?}"
        );
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn rejects_symlink_escaping_workspace() {
        let (dir, root) = init_repo();
        let outside = TempDir::new().unwrap();
        let target = outside.path().join("secret.txt");
        std::fs::write(&target, "secret").unwrap();
        std::os::unix::fs::symlink(&target, dir.path().join("escape")).unwrap();

        let err = GitAddTool
            .execute(
                GitAddInput {
                    paths: vec!["escape".into()],
                },
                &root,
            )
            .await
            .unwrap_err();
        assert!(
            matches!(err, ToolError::PermissionDenied { .. }),
            "expected PermissionDenied, got: {err:?}"
        );
    }

    #[test]
    fn has_description_and_schema() {
        let tool = GitAddTool;
        assert!(!tool.description().is_empty());
        let schema = tool.input_schema();
        assert_eq!(schema["type"], "object");
        assert!(schema["properties"]["paths"].is_object());
    }
}
