use std::process::Command;

#[test]
fn doctor_command_reports_posture() {
    // Resolve the workspace root so the default config path works.
    let manifest_dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
    let workspace_root = manifest_dir.parent().unwrap().parent().unwrap();
    let config_path = workspace_root.join("configs/grokrs.example.toml");

    let output = Command::new(env!("CARGO_BIN_EXE_grokrs"))
        .args(["--config", config_path.to_str().unwrap(), "doctor"])
        .output()
        .expect("doctor command should run");

    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("grokrs doctor"));
    assert!(stdout.contains("deny_by_default"));

    // Verify competitive feature status reporting.
    assert!(stdout.contains("--- feature status ---"));
    assert!(stdout.contains("chat="));
    assert!(stdout.contains("agent="));
    assert!(stdout.contains("search="));
    assert!(stdout.contains("generate="));
    assert!(stdout.contains("models="));
    assert!(stdout.contains("sessions="));
    assert!(stdout.contains("approval_mode="));
    assert!(stdout.contains("policy_network="));
    assert!(stdout.contains("policy_fs_write="));
    assert!(stdout.contains("policy_process_spawn="));

    // R2 feature checks.
    assert!(
        stdout.contains("voice_agent="),
        "doctor should report voice_agent status, got: {stdout}"
    );
    assert!(
        stdout.contains("otel="),
        "doctor should report otel status, got: {stdout}"
    );
    assert!(
        stdout.contains("mcp="),
        "doctor should report mcp status, got: {stdout}"
    );
    assert!(
        stdout.contains("git="),
        "doctor should report git repo status, got: {stdout}"
    );
    assert!(
        stdout.contains("model_freshness="),
        "doctor should report model freshness, got: {stdout}"
    );
    assert!(
        stdout.contains("memory="),
        "doctor should report memory status, got: {stdout}"
    );
}

/// Verify `grokrs api chat` prints deprecation notice to stderr and that
/// the notice text references `grokrs chat`. The command will fail (network
/// denied by default config), but the deprecation notice is emitted before
/// the API call, so stderr should contain it regardless.
#[test]
fn api_chat_prints_deprecation_notice_to_stderr() {
    // Write a minimal config that enables network (so we get past the policy
    // check to the deprecation notice). The command will still fail because
    // no API key is set, but the notice is printed before the API call.
    let tmp = tempfile::NamedTempFile::new().expect("tempfile");
    std::fs::write(
        tmp.path(),
        r#"
[workspace]
name = "test"
root = "."

[model]
provider = "xai"
default_model = "grok-4"

[policy]
allow_network = true
allow_shell = false
allow_workspace_writes = false
max_patch_bytes = 0

[session]
approval_mode = "allow"
transcript_dir = ".grokrs/sessions"
"#,
    )
    .unwrap();

    let output = Command::new(env!("CARGO_BIN_EXE_grokrs"))
        .args([
            "--config",
            tmp.path().to_str().unwrap(),
            "api",
            "chat",
            "hello",
        ])
        .output()
        .expect("api chat command should run");

    // The command will fail because no API key is set, but the deprecation
    // notice is printed before the API call attempt.
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("grokrs chat"),
        "stderr should contain deprecation notice mentioning 'grokrs chat', got: {stderr}"
    );
    assert!(
        stderr.contains("scripting"),
        "stderr should mention scripting use case, got: {stderr}"
    );

    // Verify stdout is clean (notice goes to stderr, not stdout).
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        !stdout.contains("grokrs chat"),
        "stdout should NOT contain the deprecation notice (it goes to stderr)"
    );
}

/// Verify `grokrs --help` lists all commands including new competitive features.
#[test]
fn top_level_help_lists_all_commands() {
    let output = Command::new(env!("CARGO_BIN_EXE_grokrs"))
        .arg("--help")
        .output()
        .expect("--help should run");

    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    for cmd in &[
        "doctor",
        "show-config",
        "eval",
        "chat",
        "agent",
        "generate",
        "models",
        "api",
        "sessions",
        "store",
    ] {
        assert!(
            stdout.contains(cmd),
            "top-level --help should list '{cmd}' command"
        );
    }
    // Verify examples section is present.
    assert!(
        stdout.contains("Examples:"),
        "top-level --help should include Examples section"
    );
}

/// Verify each new command's --help includes an Examples section.
#[test]
fn command_help_includes_examples() {
    for cmd in &["chat", "agent", "models", "sessions"] {
        let output = Command::new(env!("CARGO_BIN_EXE_grokrs"))
            .args([*cmd, "--help"])
            .output()
            .unwrap_or_else(|_| panic!("{cmd} --help should run"));

        assert!(output.status.success(), "{cmd} --help should succeed");
        let stdout = String::from_utf8_lossy(&output.stdout);
        assert!(
            stdout.contains("Examples:"),
            "{cmd} --help should include Examples section, got: {stdout}"
        );
    }
}

/// Verify `grokrs generate --help` includes examples.
#[test]
fn generate_help_includes_examples() {
    let output = Command::new(env!("CARGO_BIN_EXE_grokrs"))
        .args(["generate", "--help"])
        .output()
        .expect("generate --help should run");

    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("Examples:"),
        "generate --help should include Examples section"
    );
}
