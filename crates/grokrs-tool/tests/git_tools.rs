//! Integration tests for git tools operating on a real temp git repository.
//!
//! These tests exercise the full workflow: init repo -> create files -> `git_status`
//! -> `git_add` -> `git_diff` -> `git_commit`, validating cross-tool interaction in
//! a realistic sequence.
//!
//! Each test uses `tempfile::tempdir` for isolation. No external git dependencies
//! beyond `libgit2` (bundled via `git2` crate).

use grokrs_cap::WorkspaceRoot;
use grokrs_tool::tools::git_add::{GitAddInput, GitAddTool};
use grokrs_tool::tools::git_commit::{GitCommitInput, GitCommitTool};
use grokrs_tool::tools::git_diff::{GitDiffInput, GitDiffTool};
use grokrs_tool::tools::git_status::{GitStatusInput, GitStatusTool};
use grokrs_tool::{Classify, ToolSpec};

use tempfile::TempDir;

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn init_repo() -> (TempDir, WorkspaceRoot) {
    let dir = TempDir::new().unwrap();
    git2::Repository::init(dir.path()).unwrap();
    let root = WorkspaceRoot::new(dir.path()).unwrap();
    (dir, root)
}

fn write_file(dir: &TempDir, name: &str, content: &str) {
    std::fs::write(dir.path().join(name), content).unwrap();
}

// ---------------------------------------------------------------------------
// Full workflow: create -> status -> add -> diff -> commit -> status
// ---------------------------------------------------------------------------

#[tokio::test]
async fn full_git_workflow_create_add_commit() {
    let (dir, root) = init_repo();

    // 1. Create a file.
    write_file(&dir, "hello.txt", "Hello, world!\n");

    // 2. Check status — should show untracked.
    let status = GitStatusTool
        .execute(GitStatusInput {}, &root)
        .await
        .unwrap();
    assert!(
        status.contains("untracked"),
        "expected untracked, got: {status}"
    );
    assert!(
        status.contains("hello.txt"),
        "expected hello.txt, got: {status}"
    );

    // 3. Stage the file.
    let add_result = GitAddTool
        .execute(
            GitAddInput {
                paths: vec!["hello.txt".into()],
            },
            &root,
        )
        .await
        .unwrap();
    assert!(
        add_result.contains("1 file(s)"),
        "expected 1 file staged, got: {add_result}"
    );

    // 4. Check status — should show staged.
    let status = GitStatusTool
        .execute(GitStatusInput {}, &root)
        .await
        .unwrap();
    assert!(
        status.contains("staged:new"),
        "expected staged:new, got: {status}"
    );

    // 5. Diff staged changes.
    let diff = GitDiffTool
        .execute(GitDiffInput { staged: true }, &root)
        .await
        .unwrap();
    assert!(
        diff.contains("+Hello, world!"),
        "expected +Hello in diff, got: {diff}"
    );

    // 6. Commit.
    let commit_result = GitCommitTool
        .execute(
            GitCommitInput {
                message: "add hello file".into(),
            },
            &root,
        )
        .await
        .unwrap();
    assert!(
        commit_result.contains("[grokrs-agent] add hello file"),
        "got: {commit_result}"
    );
    assert!(commit_result.contains("committed"), "got: {commit_result}");

    // 7. Status after commit — should be clean.
    let status = GitStatusTool
        .execute(GitStatusInput {}, &root)
        .await
        .unwrap();
    assert!(
        status.contains("clean"),
        "expected clean after commit, got: {status}"
    );
}

// ---------------------------------------------------------------------------
// Multi-file add and commit
// ---------------------------------------------------------------------------

#[tokio::test]
async fn multi_file_add_and_commit() {
    let (dir, root) = init_repo();

    write_file(&dir, "a.rs", "fn a() {}");
    write_file(&dir, "b.rs", "fn b() {}");
    write_file(&dir, "c.rs", "fn c() {}");

    // Stage all three files.
    let result = GitAddTool
        .execute(
            GitAddInput {
                paths: vec!["a.rs".into(), "b.rs".into(), "c.rs".into()],
            },
            &root,
        )
        .await
        .unwrap();
    assert!(result.contains("3 file(s)"), "result: {result}");

    // Commit.
    let commit = GitCommitTool
        .execute(
            GitCommitInput {
                message: "add three files".into(),
            },
            &root,
        )
        .await
        .unwrap();
    assert!(commit.contains("committed"), "commit: {commit}");

    // Verify all files are committed by checking status.
    let status = GitStatusTool
        .execute(GitStatusInput {}, &root)
        .await
        .unwrap();
    assert!(status.contains("clean"), "status: {status}");

    // Verify the commit exists.
    let repo = git2::Repository::open(dir.path()).unwrap();
    let head = repo.head().unwrap().peel_to_commit().unwrap();
    assert_eq!(head.message().unwrap(), "[grokrs-agent] add three files");
}

// ---------------------------------------------------------------------------
// Modify-after-commit workflow
// ---------------------------------------------------------------------------

#[tokio::test]
async fn modify_after_commit_shows_diff() {
    let (dir, root) = init_repo();

    // Initial commit.
    write_file(&dir, "data.txt", "line one\n");
    GitAddTool
        .execute(
            GitAddInput {
                paths: vec!["data.txt".into()],
            },
            &root,
        )
        .await
        .unwrap();
    GitCommitTool
        .execute(
            GitCommitInput {
                message: "initial data".into(),
            },
            &root,
        )
        .await
        .unwrap();

    // Modify the file.
    write_file(&dir, "data.txt", "line one\nline two\n");

    // Status should show modified.
    let status = GitStatusTool
        .execute(GitStatusInput {}, &root)
        .await
        .unwrap();
    assert!(status.contains("modified"), "status: {status}");

    // Unstaged diff should show the change.
    let diff = GitDiffTool
        .execute(GitDiffInput { staged: false }, &root)
        .await
        .unwrap();
    assert!(diff.contains("+line two"), "diff: {diff}");

    // Staged diff should be empty (nothing staged yet).
    let staged_diff = GitDiffTool
        .execute(GitDiffInput { staged: true }, &root)
        .await
        .unwrap();
    assert!(
        staged_diff.contains("no changes"),
        "staged_diff: {staged_diff}"
    );

    // Stage and check staged diff.
    GitAddTool
        .execute(
            GitAddInput {
                paths: vec!["data.txt".into()],
            },
            &root,
        )
        .await
        .unwrap();
    let staged_diff = GitDiffTool
        .execute(GitDiffInput { staged: true }, &root)
        .await
        .unwrap();
    assert!(
        staged_diff.contains("+line two"),
        "staged_diff after add: {staged_diff}"
    );
}

// ---------------------------------------------------------------------------
// Sequential commits build history
// ---------------------------------------------------------------------------

#[tokio::test]
async fn sequential_commits_build_history() {
    let (dir, root) = init_repo();

    // Commit 1.
    write_file(&dir, "first.txt", "first");
    GitAddTool
        .execute(
            GitAddInput {
                paths: vec!["first.txt".into()],
            },
            &root,
        )
        .await
        .unwrap();
    GitCommitTool
        .execute(
            GitCommitInput {
                message: "first commit".into(),
            },
            &root,
        )
        .await
        .unwrap();

    // Commit 2.
    write_file(&dir, "second.txt", "second");
    GitAddTool
        .execute(
            GitAddInput {
                paths: vec!["second.txt".into()],
            },
            &root,
        )
        .await
        .unwrap();
    GitCommitTool
        .execute(
            GitCommitInput {
                message: "second commit".into(),
            },
            &root,
        )
        .await
        .unwrap();

    // Commit 3.
    write_file(&dir, "third.txt", "third");
    GitAddTool
        .execute(
            GitAddInput {
                paths: vec!["third.txt".into()],
            },
            &root,
        )
        .await
        .unwrap();
    GitCommitTool
        .execute(
            GitCommitInput {
                message: "third commit".into(),
            },
            &root,
        )
        .await
        .unwrap();

    // Verify the commit chain.
    let repo = git2::Repository::open(dir.path()).unwrap();
    let head = repo.head().unwrap().peel_to_commit().unwrap();
    assert_eq!(head.message().unwrap(), "[grokrs-agent] third commit");
    assert_eq!(head.parent_count(), 1);

    let parent = head.parent(0).unwrap();
    assert_eq!(parent.message().unwrap(), "[grokrs-agent] second commit");
    assert_eq!(parent.parent_count(), 1);

    let grandparent = parent.parent(0).unwrap();
    assert_eq!(
        grandparent.message().unwrap(),
        "[grokrs-agent] first commit"
    );
    assert_eq!(grandparent.parent_count(), 0); // root commit
}

// ---------------------------------------------------------------------------
// Error cases
// ---------------------------------------------------------------------------

#[tokio::test]
async fn commit_without_staged_changes_fails() {
    let (_dir, root) = init_repo();

    let err = GitCommitTool
        .execute(
            GitCommitInput {
                message: "empty commit".into(),
            },
            &root,
        )
        .await
        .unwrap_err();
    assert!(
        format!("{err:?}").contains("nothing to commit"),
        "err: {err:?}"
    );
}

#[tokio::test]
async fn add_nonexistent_file_fails() {
    let (_dir, root) = init_repo();

    let err = GitAddTool
        .execute(
            GitAddInput {
                paths: vec!["nonexistent.txt".into()],
            },
            &root,
        )
        .await
        .unwrap_err();
    assert!(
        format!("{err:?}").contains("does not exist"),
        "err: {err:?}"
    );
}

#[tokio::test]
async fn status_on_non_repo_fails() {
    let dir = TempDir::new().unwrap();
    let root = WorkspaceRoot::new(dir.path()).unwrap();

    let err = GitStatusTool
        .execute(GitStatusInput {}, &root)
        .await
        .unwrap_err();
    assert!(
        format!("{err:?}").contains("not a git repository"),
        "err: {err:?}"
    );
}

// ---------------------------------------------------------------------------
// Effect classification
// ---------------------------------------------------------------------------

#[test]
fn git_status_classifies_as_fs_read() {
    let input = GitStatusInput {};
    let effects = input.classify().unwrap();
    assert_eq!(effects.len(), 1);
    assert!(matches!(&effects[0], grokrs_policy::Effect::FsRead(_)));
}

#[test]
fn git_add_classifies_as_fs_write() {
    let input = GitAddInput {
        paths: vec!["file.txt".into()],
    };
    let effects = input.classify().unwrap();
    assert_eq!(effects.len(), 1);
    assert!(matches!(&effects[0], grokrs_policy::Effect::FsWrite(_)));
}

#[test]
fn git_commit_classifies_as_fs_write() {
    let input = GitCommitInput {
        message: "test".into(),
    };
    let effects = input.classify().unwrap();
    assert_eq!(effects.len(), 1);
    assert!(matches!(&effects[0], grokrs_policy::Effect::FsWrite(_)));
}

#[test]
fn git_diff_classifies_as_fs_read() {
    let input = GitDiffInput { staged: false };
    let effects = input.classify().unwrap();
    assert_eq!(effects.len(), 1);
    assert!(matches!(&effects[0], grokrs_policy::Effect::FsRead(_)));
}

// ---------------------------------------------------------------------------
// Subdirectory files
// ---------------------------------------------------------------------------

#[tokio::test]
async fn add_and_commit_files_in_subdirectory() {
    let (dir, root) = init_repo();

    // Create subdirectory and files.
    std::fs::create_dir_all(dir.path().join("src")).unwrap();
    write_file(&dir, "src/lib.rs", "pub fn hello() {}");
    write_file(&dir, "src/main.rs", "fn main() {}");

    // Stage subdirectory files.
    let result = GitAddTool
        .execute(
            GitAddInput {
                paths: vec!["src/lib.rs".into(), "src/main.rs".into()],
            },
            &root,
        )
        .await
        .unwrap();
    assert!(result.contains("2 file(s)"), "result: {result}");

    // Commit.
    let commit = GitCommitTool
        .execute(
            GitCommitInput {
                message: "add src files".into(),
            },
            &root,
        )
        .await
        .unwrap();
    assert!(commit.contains("committed"), "commit: {commit}");

    // Status should be clean.
    let status = GitStatusTool
        .execute(GitStatusInput {}, &root)
        .await
        .unwrap();
    assert!(status.contains("clean"), "status: {status}");
}

// ---------------------------------------------------------------------------
// Path validation in git_add
// ---------------------------------------------------------------------------

#[test]
fn add_rejects_absolute_path() {
    let input = GitAddInput {
        paths: vec!["/etc/passwd".into()],
    };
    assert!(input.classify().is_err());
}

#[test]
fn add_rejects_dotdot_traversal() {
    let input = GitAddInput {
        paths: vec!["../escape.txt".into()],
    };
    assert!(input.classify().is_err());
}

#[test]
fn add_rejects_empty_paths() {
    let input = GitAddInput { paths: vec![] };
    assert!(input.classify().is_err());
}
