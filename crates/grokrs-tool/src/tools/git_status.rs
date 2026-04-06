use grokrs_cap::{WorkspacePath, WorkspaceRoot};
use grokrs_policy::Effect;
use serde_json::json;

use crate::error::ToolError;
use crate::{Classify, ToolSpec};

/// Sentinel path used for effect classification when no specific file is targeted.
const GIT_SENTINEL: &str = ".git";

/// Input for the `git_status` tool.
#[derive(Debug, Clone, serde::Deserialize)]
pub struct GitStatusInput {
    // No fields needed — status operates on the entire workspace repo.
}

impl Classify for GitStatusInput {
    fn classify(&self) -> Result<Vec<Effect>, ToolError> {
        let wp = WorkspacePath::new(GIT_SENTINEL)?;
        Ok(vec![Effect::FsRead(wp)])
    }
}

/// Returns the working tree status of the git repository at the workspace root.
///
/// Reports modified, staged, untracked, deleted, renamed, and conflicted files.
/// Fails gracefully when the workspace is not a git repository.
#[derive(Debug, Clone)]
pub struct GitStatusTool;

impl ToolSpec for GitStatusTool {
    type Input = GitStatusInput;
    type Output = String;

    fn name(&self) -> &'static str {
        "git_status"
    }

    fn description(&self) -> &str {
        "Show the working tree status of the git repository at the workspace root. \
         Reports modified, staged, untracked, deleted, renamed, and conflicted files."
    }

    fn input_schema(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "properties": {},
            "required": [],
            "additionalProperties": false
        })
    }

    async fn execute(
        &self,
        _input: Self::Input,
        root: &WorkspaceRoot,
    ) -> Result<Self::Output, ToolError> {
        let repo = open_repo(root)?;
        let statuses = repo
            .statuses(Some(
                git2::StatusOptions::new()
                    .include_untracked(true)
                    .recurse_untracked_dirs(true),
            ))
            .map_err(|e| ToolError::Other(format!("git status failed: {e}")))?;

        if statuses.is_empty() {
            return Ok("nothing to report, working tree clean".to_string());
        }

        let mut output = String::new();
        for entry in statuses.iter() {
            let path = entry.path().unwrap_or("<non-utf8 path>");
            let status = entry.status();
            let label = status_label(status);
            output.push_str(&format!("{label}\t{path}\n"));
        }

        Ok(output)
    }
}

/// Open the git repository rooted at `root`, returning a friendly error when
/// the workspace is not a git repository.
pub(crate) fn open_repo(root: &WorkspaceRoot) -> Result<git2::Repository, ToolError> {
    git2::Repository::open(root.as_path()).map_err(|e| {
        if e.code() == git2::ErrorCode::NotFound {
            ToolError::Other(format!(
                "workspace '{}' is not a git repository",
                root.as_path().display()
            ))
        } else {
            ToolError::Other(format!("failed to open git repository: {e}"))
        }
    })
}

/// Map a `git2::Status` bitflags value to a human-readable label similar to
/// `git status --porcelain`.
fn status_label(status: git2::Status) -> &'static str {
    // Order matters — check index (staged) states first, then workdir states.
    if status.contains(git2::Status::CONFLICTED) {
        return "conflicted";
    }
    // Staged changes.
    if status.contains(git2::Status::INDEX_NEW) {
        return "staged:new";
    }
    if status.contains(git2::Status::INDEX_MODIFIED) {
        return "staged:modified";
    }
    if status.contains(git2::Status::INDEX_DELETED) {
        return "staged:deleted";
    }
    if status.contains(git2::Status::INDEX_RENAMED) {
        return "staged:renamed";
    }
    if status.contains(git2::Status::INDEX_TYPECHANGE) {
        return "staged:typechange";
    }
    // Workdir (unstaged) changes.
    if status.contains(git2::Status::WT_NEW) {
        return "untracked";
    }
    if status.contains(git2::Status::WT_MODIFIED) {
        return "modified";
    }
    if status.contains(git2::Status::WT_DELETED) {
        return "deleted";
    }
    if status.contains(git2::Status::WT_RENAMED) {
        return "renamed";
    }
    if status.contains(git2::Status::WT_TYPECHANGE) {
        return "typechange";
    }
    "unknown"
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
    fn classify_produces_fs_read() {
        let input = GitStatusInput {};
        let effects = input.classify().unwrap();
        assert_eq!(effects.len(), 1);
        assert!(matches!(&effects[0], Effect::FsRead(wp) if wp.as_path().to_str() == Some(".git")));
    }

    #[tokio::test]
    async fn clean_working_tree() {
        let (_dir, root) = init_repo();
        let result = GitStatusTool
            .execute(GitStatusInput {}, &root)
            .await
            .unwrap();
        assert!(
            result.contains("clean"),
            "expected clean status, got: {result}"
        );
    }

    #[tokio::test]
    async fn reports_untracked_files() {
        let (dir, root) = init_repo();
        std::fs::write(dir.path().join("new.txt"), "hello").unwrap();
        let result = GitStatusTool
            .execute(GitStatusInput {}, &root)
            .await
            .unwrap();
        assert!(
            result.contains("untracked"),
            "expected untracked, got: {result}"
        );
        assert!(
            result.contains("new.txt"),
            "expected new.txt, got: {result}"
        );
    }

    #[tokio::test]
    async fn reports_staged_files() {
        let (dir, root) = init_repo();
        std::fs::write(dir.path().join("staged.txt"), "content").unwrap();
        let repo = git2::Repository::open(dir.path()).unwrap();
        let mut index = repo.index().unwrap();
        index.add_path(std::path::Path::new("staged.txt")).unwrap();
        index.write().unwrap();

        let result = GitStatusTool
            .execute(GitStatusInput {}, &root)
            .await
            .unwrap();
        assert!(
            result.contains("staged:new"),
            "expected staged:new, got: {result}"
        );
        assert!(
            result.contains("staged.txt"),
            "expected staged.txt, got: {result}"
        );
    }

    #[tokio::test]
    async fn reports_modified_files() {
        let (dir, root) = init_repo();
        // Create, stage, and commit a file, then modify it.
        std::fs::write(dir.path().join("file.txt"), "v1").unwrap();
        let repo = git2::Repository::open(dir.path()).unwrap();
        let mut index = repo.index().unwrap();
        index.add_path(std::path::Path::new("file.txt")).unwrap();
        index.write().unwrap();
        let oid = index.write_tree().unwrap();
        let tree = repo.find_tree(oid).unwrap();
        let sig = git2::Signature::now("test", "test@test.com").unwrap();
        repo.commit(Some("HEAD"), &sig, &sig, "initial", &tree, &[])
            .unwrap();

        // Modify.
        std::fs::write(dir.path().join("file.txt"), "v2").unwrap();
        let result = GitStatusTool
            .execute(GitStatusInput {}, &root)
            .await
            .unwrap();
        assert!(
            result.contains("modified"),
            "expected modified, got: {result}"
        );
    }

    #[tokio::test]
    async fn error_when_not_a_repo() {
        let dir = TempDir::new().unwrap();
        let root = WorkspaceRoot::new(dir.path()).unwrap();
        let err = GitStatusTool
            .execute(GitStatusInput {}, &root)
            .await
            .unwrap_err();
        assert!(
            matches!(&err, ToolError::Other(msg) if msg.contains("not a git repository")),
            "expected not-a-repo error, got: {err:?}"
        );
    }

    #[test]
    fn has_description_and_schema() {
        let tool = GitStatusTool;
        assert!(!tool.description().is_empty());
        let schema = tool.input_schema();
        assert_eq!(schema["type"], "object");
    }
}
