use grokrs_cap::{WorkspacePath, WorkspaceRoot};
use grokrs_policy::Effect;
use serde_json::json;

use crate::error::ToolError;
use crate::{Classify, ToolSpec};

/// Sentinel path used for effect classification.
const GIT_SENTINEL: &str = ".git";

/// Maximum diff output size (4 MiB) to prevent OOM on massive diffs.
const MAX_DIFF_BYTES: usize = 4 * 1024 * 1024;

/// Input for the `git_diff` tool.
#[derive(Debug, Clone, serde::Deserialize)]
pub struct GitDiffInput {
    /// If `true`, show only staged (index) changes. Otherwise show unstaged
    /// (workdir) changes. Defaults to `false`.
    #[serde(default)]
    pub staged: bool,
}

impl Classify for GitDiffInput {
    fn classify(&self) -> Result<Vec<Effect>, ToolError> {
        let wp = WorkspacePath::new(GIT_SENTINEL)?;
        Ok(vec![Effect::FsRead(wp)])
    }
}

/// Returns the diff of staged or unstaged changes in the git repository at the
/// workspace root.
///
/// Produces unified diff output. Truncates output exceeding 4 MiB.
#[derive(Debug, Clone)]
pub struct GitDiffTool;

impl ToolSpec for GitDiffTool {
    type Input = GitDiffInput;
    type Output = String;

    fn name(&self) -> &'static str {
        "git_diff"
    }

    fn description(&self) -> &str {
        "Show the diff of changes in the git repository. \
         Set 'staged' to true for staged (index) changes, \
         or false (default) for unstaged (working directory) changes."
    }

    fn input_schema(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "properties": {
                "staged": {
                    "type": "boolean",
                    "description": "If true, show staged (index) changes. If false (default), show unstaged working directory changes.",
                    "default": false
                }
            },
            "required": [],
            "additionalProperties": false
        })
    }

    async fn execute(
        &self,
        input: Self::Input,
        root: &WorkspaceRoot,
    ) -> Result<Self::Output, ToolError> {
        let repo = super::git_status::open_repo(root)?;

        let diff = if input.staged {
            // Staged: diff between HEAD tree and index.
            let head_tree =
                match repo.head() {
                    Ok(head) => {
                        let commit = head.peel_to_commit().map_err(|e| {
                            ToolError::Other(format!("failed to peel HEAD to commit: {e}"))
                        })?;
                        Some(commit.tree().map_err(|e| {
                            ToolError::Other(format!("failed to get HEAD tree: {e}"))
                        })?)
                    }
                    Err(e) if e.code() == git2::ErrorCode::UnbornBranch => {
                        // No commits yet — diff index against empty tree.
                        None
                    }
                    Err(e) => {
                        return Err(ToolError::Other(format!("failed to read HEAD: {e}")));
                    }
                };
            repo.diff_tree_to_index(
                head_tree.as_ref(),
                None, // current index
                None,
            )
            .map_err(|e| ToolError::Other(format!("git diff (staged) failed: {e}")))?
        } else {
            // Unstaged: diff between index and workdir.
            repo.diff_index_to_workdir(None, None)
                .map_err(|e| ToolError::Other(format!("git diff (unstaged) failed: {e}")))?
        };

        // Render the diff to a string buffer.
        let mut output = String::new();
        let mut truncated = false;
        diff.print(git2::DiffFormat::Patch, |_delta, _hunk, line| {
            if truncated {
                return false;
            }
            let prefix = match line.origin() {
                '+' => "+",
                '-' => "-",
                ' ' => " ",
                _ => "",
            };
            let content = std::str::from_utf8(line.content()).unwrap_or("<binary>");
            let addition = format!("{prefix}{content}");
            if output.len() + addition.len() > MAX_DIFF_BYTES {
                output.push_str("\n... diff output truncated (exceeds 4 MiB limit) ...\n");
                truncated = true;
                return false;
            }
            output.push_str(&addition);
            true
        })
        .map_err(|e| ToolError::Other(format!("diff print failed: {e}")))?;

        if output.is_empty() {
            return Ok("no changes".to_string());
        }

        Ok(output)
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

    fn commit_file(dir: &TempDir, name: &str, content: &str) {
        std::fs::write(dir.path().join(name), content).unwrap();
        let repo = git2::Repository::open(dir.path()).unwrap();
        let mut index = repo.index().unwrap();
        index.add_path(std::path::Path::new(name)).unwrap();
        index.write().unwrap();
        let oid = index.write_tree().unwrap();
        let tree = repo.find_tree(oid).unwrap();
        let sig = git2::Signature::now("test", "test@test.com").unwrap();
        let parents: Vec<git2::Commit> = match repo.head() {
            Ok(head) => vec![head.peel_to_commit().unwrap()],
            Err(_) => vec![],
        };
        let parent_refs: Vec<&git2::Commit> = parents.iter().collect();
        repo.commit(Some("HEAD"), &sig, &sig, "commit", &tree, &parent_refs)
            .unwrap();
    }

    #[test]
    fn classify_produces_fs_read() {
        let input = GitDiffInput { staged: false };
        let effects = input.classify().unwrap();
        assert_eq!(effects.len(), 1);
        assert!(matches!(&effects[0], Effect::FsRead(wp) if wp.as_path().to_str() == Some(".git")));
    }

    #[tokio::test]
    async fn no_changes_returns_no_changes() {
        let (_dir, root) = init_repo();
        let result = GitDiffTool
            .execute(GitDiffInput { staged: false }, &root)
            .await
            .unwrap();
        assert!(
            result.contains("no changes"),
            "expected 'no changes', got: {result}"
        );
    }

    #[tokio::test]
    async fn unstaged_diff_shows_modifications() {
        let (dir, root) = init_repo();
        commit_file(&dir, "file.txt", "line one\n");
        // Modify the file without staging.
        std::fs::write(dir.path().join("file.txt"), "line one\nline two\n").unwrap();

        let result = GitDiffTool
            .execute(GitDiffInput { staged: false }, &root)
            .await
            .unwrap();
        assert!(
            result.contains("+line two"),
            "expected '+line two' in diff, got: {result}"
        );
    }

    #[tokio::test]
    async fn staged_diff_shows_index_changes() {
        let (dir, root) = init_repo();
        commit_file(&dir, "file.txt", "original\n");
        // Modify and stage.
        std::fs::write(dir.path().join("file.txt"), "changed\n").unwrap();
        let repo = git2::Repository::open(dir.path()).unwrap();
        let mut index = repo.index().unwrap();
        index.add_path(std::path::Path::new("file.txt")).unwrap();
        index.write().unwrap();

        let result = GitDiffTool
            .execute(GitDiffInput { staged: true }, &root)
            .await
            .unwrap();
        assert!(
            result.contains("+changed"),
            "expected '+changed' in staged diff, got: {result}"
        );
        assert!(
            result.contains("-original"),
            "expected '-original' in staged diff, got: {result}"
        );
    }

    #[tokio::test]
    async fn staged_diff_on_unborn_branch() {
        let (dir, root) = init_repo();
        // Stage a file on an unborn branch (no commits yet).
        std::fs::write(dir.path().join("first.txt"), "content\n").unwrap();
        let repo = git2::Repository::open(dir.path()).unwrap();
        let mut index = repo.index().unwrap();
        index.add_path(std::path::Path::new("first.txt")).unwrap();
        index.write().unwrap();

        let result = GitDiffTool
            .execute(GitDiffInput { staged: true }, &root)
            .await
            .unwrap();
        assert!(
            result.contains("+content"),
            "expected '+content' in staged diff on unborn branch, got: {result}"
        );
    }

    #[tokio::test]
    async fn error_when_not_a_repo() {
        let dir = TempDir::new().unwrap();
        let root = WorkspaceRoot::new(dir.path()).unwrap();
        let err = GitDiffTool
            .execute(GitDiffInput { staged: false }, &root)
            .await
            .unwrap_err();
        assert!(
            matches!(&err, ToolError::Other(msg) if msg.contains("not a git repository")),
            "expected not-a-repo error, got: {err:?}"
        );
    }

    #[test]
    fn has_description_and_schema() {
        let tool = GitDiffTool;
        assert!(!tool.description().is_empty());
        let schema = tool.input_schema();
        assert_eq!(schema["type"], "object");
        assert!(schema["properties"]["staged"].is_object());
    }
}
