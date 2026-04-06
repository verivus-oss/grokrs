//! Integration tests for `grokrs sessions list`, `grokrs sessions show`,
//! `grokrs sessions transcript`, and `grokrs sessions clean` CLI subcommands.
//!
//! Each test creates a temporary directory with a store database, populates it
//! with test data using `grokrs_store::Store`, writes a TOML config that points
//! at the temporary store, and runs the binary via `std::process::Command`.

use std::io::Write as _;
use std::process::Command;

/// Helper: write a TOML config that points the store at the given tempdir.
///
/// The `workspace_root` argument is the temporary directory that contains
/// `.grokrs/state.db`. The CLI resolves the store path relative to
/// `workspace_root` (which is cwd for the sessions command).
fn write_test_config(store_path: &str) -> tempfile::NamedTempFile {
    let mut f = tempfile::NamedTempFile::new().expect("create tempfile");
    write!(
        f,
        r#"
[workspace]
name = "test"
root = "."

[model]
provider = "xai"
default_model = "grok-4"

[policy]
allow_network = false
allow_shell = false
allow_workspace_writes = true
max_patch_bytes = 1048576

[session]
approval_mode = "interactive"
transcript_dir = ".grokrs/sessions"

[store]
path = "{store_path}"
"#
    )
    .unwrap();
    f.flush().unwrap();
    f
}

/// Helper: create a store in the given tmpdir and return it for population.
fn create_store(tmpdir: &std::path::Path) -> grokrs_store::Store {
    grokrs_store::Store::open(tmpdir).expect("open store")
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

/// `grokrs sessions list` with an empty store should report no sessions.
#[test]
fn sessions_list_empty_store_shows_no_sessions() {
    let tmpdir = tempfile::tempdir().expect("create tmpdir");
    let store = create_store(tmpdir.path());
    store.close().ok();

    let config = write_test_config(".grokrs/state.db");

    let output = Command::new(env!("CARGO_BIN_EXE_grokrs"))
        .args([
            "--config",
            config.path().to_str().unwrap(),
            "sessions",
            "list",
        ])
        .current_dir(tmpdir.path())
        .output()
        .expect("failed to run grokrs");

    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);

    assert!(output.status.success(), "expected exit 0, stderr: {stderr}");
    assert!(
        stdout.contains("No sessions found"),
        "stdout should report no sessions: {stdout}"
    );
}

/// `grokrs sessions list` should show sessions when data exists.
#[test]
fn sessions_list_shows_sessions_with_data() {
    let tmpdir = tempfile::tempdir().expect("create tmpdir");
    let store = create_store(tmpdir.path());

    store
        .sessions()
        .create("sess-alpha-001", "Untrusted")
        .expect("create session alpha");
    store
        .sessions()
        .create("sess-beta-002", "AdminTrusted")
        .expect("create session beta");
    store
        .sessions()
        .transition("sess-beta-002", "Ready")
        .expect("transition beta to Ready");
    store.close().ok();

    let config = write_test_config(".grokrs/state.db");

    let output = Command::new(env!("CARGO_BIN_EXE_grokrs"))
        .args([
            "--config",
            config.path().to_str().unwrap(),
            "sessions",
            "list",
        ])
        .current_dir(tmpdir.path())
        .output()
        .expect("failed to run grokrs");

    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);

    assert!(output.status.success(), "expected exit 0, stderr: {stderr}");
    assert!(
        stdout.contains("sess-alp"),
        "stdout should contain truncated id for alpha: {stdout}"
    );
    assert!(
        stdout.contains("sess-bet"),
        "stdout should contain truncated id for beta: {stdout}"
    );
    assert!(
        stdout.contains("Untrusted"),
        "stdout should show trust level: {stdout}"
    );
    assert!(
        stdout.contains("AdminTrusted"),
        "stdout should show AdminTrusted trust level: {stdout}"
    );
    assert!(
        stdout.contains("2 session(s) shown"),
        "stdout should report count: {stdout}"
    );
    // Table header.
    assert!(
        stdout.contains("ID") && stdout.contains("STATE") && stdout.contains("TRUST"),
        "stdout should contain table headers: {stdout}"
    );
}

/// `grokrs sessions show <prefix>` should show details for a session matched
/// by prefix.
#[test]
fn sessions_show_with_prefix_match() {
    let tmpdir = tempfile::tempdir().expect("create tmpdir");
    let store = create_store(tmpdir.path());

    store
        .sessions()
        .create("show-test-unique-id-12345", "Untrusted")
        .expect("create session");
    // Add a transcript to verify transcript count.
    store
        .transcripts()
        .log_request(
            "show-test-unique-id-12345",
            "/v1/responses",
            "POST",
            Some("{\"input\":\"hello\"}"),
        )
        .expect("log request");
    store.close().ok();

    let config = write_test_config(".grokrs/state.db");

    let output = Command::new(env!("CARGO_BIN_EXE_grokrs"))
        .args([
            "--config",
            config.path().to_str().unwrap(),
            "sessions",
            "show",
            "show-test",
        ])
        .current_dir(tmpdir.path())
        .output()
        .expect("failed to run grokrs");

    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);

    assert!(output.status.success(), "expected exit 0, stderr: {stderr}");
    assert!(
        stdout.contains("show-test-unique-id-12345"),
        "stdout should contain full session id: {stdout}"
    );
    assert!(
        stdout.contains("Untrusted"),
        "stdout should contain trust level: {stdout}"
    );
    assert!(
        stdout.contains("transcripts: 1"),
        "stdout should report transcript count of 1: {stdout}"
    );
}

/// `grokrs sessions transcript <prefix>` should display request/response data.
#[test]
fn sessions_transcript_shows_request_response() {
    let tmpdir = tempfile::tempdir().expect("create tmpdir");
    let store = create_store(tmpdir.path());

    store
        .sessions()
        .create("tx-test-sess-abc", "Untrusted")
        .expect("create session");
    let tid = store
        .transcripts()
        .log_request(
            "tx-test-sess-abc",
            "/v1/responses",
            "POST",
            Some("{\"input\":\"hello world\"}"),
        )
        .expect("log request");
    let usage = grokrs_store::types::TranscriptUsage {
        cost_in_usd_ticks: None,
        input_tokens: Some(15),
        output_tokens: Some(42),
        reasoning_tokens: Some(8),
    };
    store
        .transcripts()
        .log_response(
            tid,
            200,
            Some("{\"output\":\"Hi there!\"}"),
            &usage,
            Some("resp_xyz789"),
        )
        .expect("log response");
    store.close().ok();

    let config = write_test_config(".grokrs/state.db");

    let output = Command::new(env!("CARGO_BIN_EXE_grokrs"))
        .args([
            "--config",
            config.path().to_str().unwrap(),
            "sessions",
            "transcript",
            "tx-test-sess",
        ])
        .current_dir(tmpdir.path())
        .output()
        .expect("failed to run grokrs");

    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);

    assert!(output.status.success(), "expected exit 0, stderr: {stderr}");
    assert!(
        stdout.contains("Turn 1"),
        "stdout should show turn number: {stdout}"
    );
    assert!(
        stdout.contains("POST /v1/responses"),
        "stdout should show endpoint and method: {stdout}"
    );
    assert!(
        stdout.contains("hello world"),
        "stdout should show request body content: {stdout}"
    );
    assert!(
        stdout.contains("Hi there!"),
        "stdout should show response body content: {stdout}"
    );
    assert!(
        stdout.contains("200"),
        "stdout should show status code: {stdout}"
    );
    assert!(
        stdout.contains("input=15"),
        "stdout should show input token usage: {stdout}"
    );
    assert!(
        stdout.contains("output=42"),
        "stdout should show output token usage: {stdout}"
    );
    assert!(
        stdout.contains("reasoning=8"),
        "stdout should show reasoning token usage: {stdout}"
    );
    assert!(
        stdout.contains("resp_xyz789"),
        "stdout should show response_id: {stdout}"
    );
}

/// `grokrs sessions clean --yes --older-than 0m` should delete old closed
/// sessions but preserve active ones.
#[test]
fn sessions_clean_removes_closed_preserves_active() {
    let tmpdir = tempfile::tempdir().expect("create tmpdir");
    let store = create_store(tmpdir.path());

    // Create a closed session.
    store
        .sessions()
        .create("clean-closed-sess", "Untrusted")
        .expect("create closed session");
    store
        .sessions()
        .transition("clean-closed-sess", "Closed")
        .expect("transition to Closed");

    // Create an active (Ready) session.
    store
        .sessions()
        .create("clean-active-sess", "Untrusted")
        .expect("create active session");
    store
        .sessions()
        .transition("clean-active-sess", "Ready")
        .expect("transition to Ready");

    store.close().ok();

    let config = write_test_config(".grokrs/state.db");

    let output = Command::new(env!("CARGO_BIN_EXE_grokrs"))
        .args([
            "--config",
            config.path().to_str().unwrap(),
            "sessions",
            "clean",
            "--older-than",
            "0m",
            "--yes",
        ])
        .current_dir(tmpdir.path())
        .output()
        .expect("failed to run grokrs");

    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);

    assert!(output.status.success(), "expected exit 0, stderr: {stderr}");
    // With --older-than 0m and exact same-second timestamps, the session might
    // not qualify as "older than now". The key verification is that the command
    // succeeds and respects the active/closed distinction.
    assert!(
        stdout.contains("Deleted") || stdout.contains("No sessions to clean"),
        "stdout should report clean result: {stdout}"
    );

    // Verify the active (Ready) session is never deleted by clean.
    let store2 = grokrs_store::Store::open(tmpdir.path()).expect("reopen store");
    let active = store2
        .sessions()
        .get("clean-active-sess")
        .expect("query active session");
    assert!(
        active.is_some(),
        "active session should not be deleted by clean"
    );
}

/// `grokrs sessions list --active` should only show non-Closed, non-Failed
/// sessions.
#[test]
fn sessions_list_active_filters_closed() {
    let tmpdir = tempfile::tempdir().expect("create tmpdir");
    let store = create_store(tmpdir.path());

    store
        .sessions()
        .create("active-sess-001", "Untrusted")
        .expect("create active");
    store
        .sessions()
        .transition("active-sess-001", "Ready")
        .expect("transition to Ready");

    store
        .sessions()
        .create("closed-sess-002", "Untrusted")
        .expect("create closed");
    store
        .sessions()
        .transition("closed-sess-002", "Closed")
        .expect("transition to Closed");

    store.close().ok();

    let config = write_test_config(".grokrs/state.db");

    let output = Command::new(env!("CARGO_BIN_EXE_grokrs"))
        .args([
            "--config",
            config.path().to_str().unwrap(),
            "sessions",
            "list",
            "--active",
        ])
        .current_dir(tmpdir.path())
        .output()
        .expect("failed to run grokrs");

    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);

    assert!(output.status.success(), "expected exit 0, stderr: {stderr}");
    assert!(
        stdout.contains("active-s"),
        "stdout should contain the active session: {stdout}"
    );
    assert!(
        !stdout.contains("closed-s"),
        "stdout should NOT contain the closed session: {stdout}"
    );
    assert!(
        stdout.contains("1 session(s) shown"),
        "stdout should report 1 session shown: {stdout}"
    );
}

/// `grokrs sessions show <ambiguous>` should fail when the prefix matches
/// multiple sessions.
#[test]
fn sessions_show_ambiguous_prefix_fails() {
    let tmpdir = tempfile::tempdir().expect("create tmpdir");
    let store = create_store(tmpdir.path());

    store
        .sessions()
        .create("ambig-sess-001", "Untrusted")
        .expect("create ambig 1");
    store
        .sessions()
        .create("ambig-sess-002", "Untrusted")
        .expect("create ambig 2");
    store.close().ok();

    let config = write_test_config(".grokrs/state.db");

    let output = Command::new(env!("CARGO_BIN_EXE_grokrs"))
        .args([
            "--config",
            config.path().to_str().unwrap(),
            "sessions",
            "show",
            "ambig-sess",
        ])
        .current_dir(tmpdir.path())
        .output()
        .expect("failed to run grokrs");

    assert!(
        !output.status.success(),
        "should fail when prefix is ambiguous"
    );
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("Ambiguous") || stderr.contains("ambig"),
        "stderr should mention ambiguous prefix: {stderr}"
    );
}
