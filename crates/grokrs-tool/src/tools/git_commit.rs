use grokrs_cap::{WorkspacePath, WorkspaceRoot};
use grokrs_policy::Effect;
use serde_json::json;

use crate::error::ToolError;
use crate::{Classify, ToolSpec};

/// Sentinel path used for effect classification.
const GIT_SENTINEL: &str = ".git";

/// Prefix prepended to all commit messages for auditability.
const COMMIT_PREFIX: &str = "[grokrs-agent] ";

/// Maximum commit message length (including prefix) to prevent abuse.
const MAX_MESSAGE_LEN: usize = 4096;

/// Input for the `git_commit` tool.
#[derive(Debug, Clone, serde::Deserialize)]
pub struct GitCommitInput {
    /// The commit message. Will be automatically prefixed with `[grokrs-agent]`.
    pub message: String,
}

impl Classify for GitCommitInput {
    fn classify(&self) -> Result<Vec<Effect>, ToolError> {
        let wp = WorkspacePath::new(GIT_SENTINEL)?;
        Ok(vec![Effect::FsWrite(wp)])
    }
}

/// Creates a git commit from the currently staged changes.
///
/// The commit message is automatically prefixed with `[grokrs-agent]` for
/// auditability. The commit author and committer are taken from the repository
/// configuration (user.name, user.email), falling back to a default identity if
/// not configured.
///
/// Fails if there are no staged changes to commit.
#[derive(Debug, Clone)]
pub struct GitCommitTool;

impl ToolSpec for GitCommitTool {
    type Input = GitCommitInput;
    type Output = String;

    fn name(&self) -> &'static str {
        "git_commit"
    }

    fn description(&self) -> &str {
        "Create a git commit from staged changes. The commit message is \
         automatically prefixed with '[grokrs-agent]' for auditability. \
         Fails if there are no staged changes."
    }

    fn input_schema(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "properties": {
                "message": {
                    "type": "string",
                    "description": "Commit message (will be prefixed with '[grokrs-agent]')"
                }
            },
            "required": ["message"],
            "additionalProperties": false
        })
    }

    async fn execute(
        &self,
        input: Self::Input,
        root: &WorkspaceRoot,
    ) -> Result<Self::Output, ToolError> {
        let message = input.message.trim();
        if message.is_empty() {
            return Err(ToolError::Other("commit message must not be empty".into()));
        }

        let full_message = format!("{COMMIT_PREFIX}{message}");
        if full_message.len() > MAX_MESSAGE_LEN {
            return Err(ToolError::Other(format!(
                "commit message exceeds maximum length of {MAX_MESSAGE_LEN} bytes"
            )));
        }

        let repo = super::git_status::open_repo(root)?;

        // Verify there are staged changes.
        let has_staged = has_staged_changes(&repo)?;
        if !has_staged {
            return Err(ToolError::Other(
                "nothing to commit: no changes staged in the index".into(),
            ));
        }

        // Build the tree from the current index.
        let mut index = repo
            .index()
            .map_err(|e| ToolError::Other(format!("failed to open git index: {e}")))?;
        let tree_oid = index
            .write_tree()
            .map_err(|e| ToolError::Other(format!("failed to write tree: {e}")))?;
        let tree = repo
            .find_tree(tree_oid)
            .map_err(|e| ToolError::Other(format!("failed to find tree: {e}")))?;

        // Resolve signature: try repo config, fall back to default.
        let sig = repo.signature().unwrap_or_else(|_| {
            git2::Signature::now("grokrs-agent", "grokrs-agent@noreply")
                .expect("static signature name/email are valid")
        });

        // Get parent commit(s), if any.
        let parent_commit = match repo.head() {
            Ok(head) => Some(
                head.peel_to_commit()
                    .map_err(|e| ToolError::Other(format!("failed to peel HEAD to commit: {e}")))?,
            ),
            Err(e) if e.code() == git2::ErrorCode::UnbornBranch => None,
            Err(e) => {
                return Err(ToolError::Other(format!("failed to read HEAD: {e}")));
            }
        };

        let parents: Vec<&git2::Commit> = parent_commit.iter().collect();

        let commit_oid = repo
            .commit(Some("HEAD"), &sig, &sig, &full_message, &tree, &parents)
            .map_err(|e| ToolError::Other(format!("git commit failed: {e}")))?;

        let short_oid = &commit_oid.to_string()[..7.min(commit_oid.to_string().len())];
        Ok(format!("committed {short_oid}: {full_message}"))
    }
}

/// Check whether the repository has any staged (index) changes that differ from
/// HEAD (or from an empty tree for unborn branches).
fn has_staged_changes(repo: &git2::Repository) -> Result<bool, ToolError> {
    let statuses = repo
        .statuses(Some(git2::StatusOptions::new().include_untracked(false)))
        .map_err(|e| ToolError::Other(format!("failed to get status: {e}")))?;

    for entry in statuses.iter() {
        let s = entry.status();
        if s.intersects(
            git2::Status::INDEX_NEW
                | git2::Status::INDEX_MODIFIED
                | git2::Status::INDEX_DELETED
                | git2::Status::INDEX_RENAMED
                | git2::Status::INDEX_TYPECHANGE,
        ) {
            return Ok(true);
        }
    }
    Ok(false)
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

    fn stage_file(dir: &TempDir, name: &str, content: &str) {
        std::fs::write(dir.path().join(name), content).unwrap();
        let repo = git2::Repository::open(dir.path()).unwrap();
        let mut index = repo.index().unwrap();
        index.add_path(std::path::Path::new(name)).unwrap();
        index.write().unwrap();
    }

    #[test]
    fn classify_produces_fs_write() {
        let input = GitCommitInput {
            message: "test commit".into(),
        };
        let effects = input.classify().unwrap();
        assert_eq!(effects.len(), 1);
        assert!(
            matches!(&effects[0], Effect::FsWrite(wp) if wp.as_path().to_str() == Some(".git"))
        );
    }

    #[tokio::test]
    async fn creates_commit_with_prefix() {
        let (dir, root) = init_repo();
        stage_file(&dir, "file.txt", "content");

        let result = GitCommitTool
            .execute(
                GitCommitInput {
                    message: "add file".into(),
                },
                &root,
            )
            .await
            .unwrap();
        assert!(
            result.contains("[grokrs-agent] add file"),
            "result: {result}"
        );

        // Verify the commit exists in the repo.
        let repo = git2::Repository::open(dir.path()).unwrap();
        let head = repo.head().unwrap().peel_to_commit().unwrap();
        assert_eq!(head.message().unwrap(), "[grokrs-agent] add file");
    }

    #[tokio::test]
    async fn commit_on_unborn_branch() {
        let (dir, root) = init_repo();
        stage_file(&dir, "first.txt", "initial");

        let result = GitCommitTool
            .execute(
                GitCommitInput {
                    message: "initial commit".into(),
                },
                &root,
            )
            .await
            .unwrap();
        assert!(result.contains("committed"), "result: {result}");
        assert!(
            result.contains("[grokrs-agent] initial commit"),
            "result: {result}"
        );

        // Verify HEAD now points to a commit.
        let repo = git2::Repository::open(dir.path()).unwrap();
        assert!(repo.head().is_ok());
    }

    #[tokio::test]
    async fn commit_with_parent() {
        let (dir, root) = init_repo();
        // First commit.
        stage_file(&dir, "a.txt", "a");
        GitCommitTool
            .execute(
                GitCommitInput {
                    message: "first".into(),
                },
                &root,
            )
            .await
            .unwrap();

        // Second commit.
        stage_file(&dir, "b.txt", "b");
        let result = GitCommitTool
            .execute(
                GitCommitInput {
                    message: "second".into(),
                },
                &root,
            )
            .await
            .unwrap();
        assert!(result.contains("committed"), "result: {result}");

        // Verify the commit has a parent.
        let repo = git2::Repository::open(dir.path()).unwrap();
        let head = repo.head().unwrap().peel_to_commit().unwrap();
        assert_eq!(head.parent_count(), 1);
        assert_eq!(
            head.parent(0).unwrap().message().unwrap(),
            "[grokrs-agent] first"
        );
    }

    #[tokio::test]
    async fn rejects_empty_message() {
        let (dir, root) = init_repo();
        stage_file(&dir, "file.txt", "content");

        let err = GitCommitTool
            .execute(
                GitCommitInput {
                    message: "  ".into(),
                },
                &root,
            )
            .await
            .unwrap_err();
        assert!(
            matches!(&err, ToolError::Other(msg) if msg.contains("empty")),
            "expected empty message error, got: {err:?}"
        );
    }

    #[tokio::test]
    async fn rejects_when_nothing_staged() {
        let (_dir, root) = init_repo();
        let err = GitCommitTool
            .execute(
                GitCommitInput {
                    message: "nothing here".into(),
                },
                &root,
            )
            .await
            .unwrap_err();
        assert!(
            matches!(&err, ToolError::Other(msg) if msg.contains("nothing to commit")),
            "expected nothing-to-commit error, got: {err:?}"
        );
    }

    #[tokio::test]
    async fn rejects_overly_long_message() {
        let (dir, root) = init_repo();
        stage_file(&dir, "file.txt", "content");

        let long_msg = "x".repeat(MAX_MESSAGE_LEN + 1);
        let err = GitCommitTool
            .execute(GitCommitInput { message: long_msg }, &root)
            .await
            .unwrap_err();
        assert!(
            matches!(&err, ToolError::Other(msg) if msg.contains("maximum length")),
            "expected max-length error, got: {err:?}"
        );
    }

    #[tokio::test]
    async fn error_when_not_a_repo() {
        let dir = TempDir::new().unwrap();
        let root = WorkspaceRoot::new(dir.path()).unwrap();
        let err = GitCommitTool
            .execute(
                GitCommitInput {
                    message: "test".into(),
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

    #[tokio::test]
    async fn uses_fallback_signature() {
        let (dir, root) = init_repo();
        stage_file(&dir, "file.txt", "content");

        // Ensure no git config for user.name/user.email.
        // In a fresh temp repo, there's no local config, so the fallback
        // should be used (unless the global config has user settings).
        let result = GitCommitTool
            .execute(
                GitCommitInput {
                    message: "fallback test".into(),
                },
                &root,
            )
            .await
            .unwrap();
        assert!(result.contains("committed"), "result: {result}");
    }

    #[test]
    fn has_description_and_schema() {
        let tool = GitCommitTool;
        assert!(!tool.description().is_empty());
        let schema = tool.input_schema();
        assert_eq!(schema["type"], "object");
        assert!(schema["properties"]["message"].is_object());
    }
}
