//! Integration tests for `grokrs generate image` and `grokrs generate video`
//! CLI subcommands.
//!
//! Each test starts a wiremock `MockServer`, writes a TOML config pointing at
//! the mock's URL, sets the API key env var, and runs the binary via
//! `std::process::Command`. Assertions are against stdout/stderr text, exit
//! codes, and output files.

use std::io::Write as _;
use std::process::Command;

use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

/// Helper: write a TOML config with network enabled, workspace writes enabled,
/// and the API base URL pointing at the given mock server.
fn write_test_config(base_url: &str) -> tempfile::NamedTempFile {
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
allow_network = true
allow_shell = false
allow_workspace_writes = true
max_patch_bytes = 1048576

[session]
approval_mode = "allow"
transcript_dir = ".grokrs/sessions"

[api]
api_key_env = "GROKRS_GEN_TEST_KEY"
base_url = "{base_url}"
timeout_secs = 10
max_retries = 0
"#
    )
    .unwrap();
    f.flush().unwrap();
    f
}

/// Helper: write a config with network denied.
fn write_network_denied_config() -> tempfile::NamedTempFile {
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
approval_mode = "deny"
transcript_dir = ".grokrs/sessions"
"#
    )
    .unwrap();
    f.flush().unwrap();
    f
}

/// Helper: write a config with workspace writes denied.
fn write_writes_denied_config(base_url: &str) -> tempfile::NamedTempFile {
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
allow_network = true
allow_shell = false
allow_workspace_writes = false
max_patch_bytes = 0

[session]
approval_mode = "deny"
transcript_dir = ".grokrs/sessions"

[api]
api_key_env = "GROKRS_GEN_TEST_KEY"
base_url = "{base_url}"
timeout_secs = 10
max_retries = 0
"#
    )
    .unwrap();
    f.flush().unwrap();
    f
}

// ---------------------------------------------------------------------------
// Image generation tests
// ---------------------------------------------------------------------------

/// `grokrs generate image '<prompt>' -o <path>` should download the image from
/// the URL returned by the API and write it to the output file.
#[tokio::test]
async fn generate_image_downloads_and_saves_to_output() {
    let server = MockServer::start().await;

    // Fake image bytes.
    let fake_image_bytes: Vec<u8> = vec![0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A];

    // Mock: POST /v1/images/generations returns a URL pointing at the mock.
    let image_url = format!("{}/fake-images/cat.png", server.uri());
    let gen_response = serde_json::json!({
        "created": 1_700_000_000,
        "data": [
            {
                "url": image_url,
                "revised_prompt": "A fluffy cat sitting on a windowsill"
            }
        ]
    });

    Mock::given(method("POST"))
        .and(path("/v1/images/generations"))
        .respond_with(ResponseTemplate::new(200).set_body_json(&gen_response))
        .expect(1)
        .mount(&server)
        .await;

    // Mock: GET /fake-images/cat.png returns fake image bytes.
    Mock::given(method("GET"))
        .and(path("/fake-images/cat.png"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_bytes(fake_image_bytes.clone())
                .append_header("content-type", "image/png"),
        )
        .expect(1)
        .mount(&server)
        .await;

    let config = write_test_config(&server.uri());
    let tmpdir = tempfile::tempdir().expect("create tmpdir");
    let output_path = "output/cat.png";

    let output = Command::new(env!("CARGO_BIN_EXE_grokrs"))
        .args([
            "--config",
            config.path().to_str().unwrap(),
            "generate",
            "image",
            "a cat sitting on a windowsill",
            "-o",
            output_path,
        ])
        .env("GROKRS_GEN_TEST_KEY", "test-key-gen-image")
        .current_dir(tmpdir.path())
        .output()
        .expect("failed to run grokrs");

    let stderr = String::from_utf8_lossy(&output.stderr);

    assert!(output.status.success(), "expected exit 0, stderr: {stderr}");

    // Verify the output file was created with the correct content.
    let saved_path = tmpdir.path().join(output_path);
    assert!(
        saved_path.exists(),
        "output file should exist at {}",
        saved_path.display()
    );

    let saved_bytes = std::fs::read(&saved_path).expect("read saved file");
    assert_eq!(
        saved_bytes, fake_image_bytes,
        "saved file should contain the downloaded image bytes"
    );

    // Stderr should mention the download and write.
    assert!(
        stderr.contains("downloading"),
        "stderr should mention downloading: {stderr}"
    );
    assert!(
        stderr.contains("wrote") && stderr.contains("bytes"),
        "stderr should mention bytes written: {stderr}"
    );
}

/// `grokrs generate image` with network denied should fail with a clear error.
#[tokio::test]
async fn generate_image_network_denied_fails() {
    let config = write_network_denied_config();
    let tmpdir = tempfile::tempdir().expect("create tmpdir");

    let output = Command::new(env!("CARGO_BIN_EXE_grokrs"))
        .args([
            "--config",
            config.path().to_str().unwrap(),
            "generate",
            "image",
            "a sunset",
            "-o",
            "sunset.png",
        ])
        .current_dir(tmpdir.path())
        .output()
        .expect("failed to run grokrs");

    assert!(
        !output.status.success(),
        "should fail when network is denied"
    );
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("Network access is denied") || stderr.contains("allow_network"),
        "stderr should mention network policy denial: {stderr}"
    );
}

/// `grokrs generate image` with workspace writes denied should fail with a
/// clear error about write policy.
#[tokio::test]
async fn generate_image_writes_denied_fails() {
    let server = MockServer::start().await;
    let config = write_writes_denied_config(&server.uri());
    let tmpdir = tempfile::tempdir().expect("create tmpdir");

    let output = Command::new(env!("CARGO_BIN_EXE_grokrs"))
        .args([
            "--config",
            config.path().to_str().unwrap(),
            "generate",
            "image",
            "a mountain",
            "-o",
            "mountain.png",
        ])
        .env("GROKRS_GEN_TEST_KEY", "test-key-gen-writes-denied")
        .current_dir(tmpdir.path())
        .output()
        .expect("failed to run grokrs");

    assert!(
        !output.status.success(),
        "should fail when writes are denied"
    );
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("denied") || stderr.contains("approval"),
        "stderr should mention write denial: {stderr}"
    );
}

/// `grokrs generate video '<prompt>'` should submit a generation request, poll
/// until done, and print the video URL to stdout.
#[tokio::test]
async fn generate_video_prints_url_on_success() {
    let server = MockServer::start().await;

    // Mock: POST /v1/videos/generations returns a request_id.
    Mock::given(method("POST"))
        .and(path("/v1/videos/generations"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "request_id": "vid_gen_test_001"
        })))
        .expect(1)
        .mount(&server)
        .await;

    // Mock: GET /v1/videos/vid_gen_test_001 returns done with a video URL.
    // The first poll returns done immediately to keep the test fast.
    Mock::given(method("GET"))
        .and(path("/v1/videos/vid_gen_test_001"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "status": "done",
            "video": {
                "url": "https://cdn.example.com/result-video.mp4"
            }
        })))
        .mount(&server)
        .await;

    let config = write_test_config(&server.uri());

    let output = Command::new(env!("CARGO_BIN_EXE_grokrs"))
        .args([
            "--config",
            config.path().to_str().unwrap(),
            "generate",
            "video",
            "a cat walking across a field",
        ])
        .env("GROKRS_GEN_TEST_KEY", "test-key-gen-video")
        .output()
        .expect("failed to run grokrs");

    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);

    assert!(output.status.success(), "expected exit 0, stderr: {stderr}");
    assert!(
        stdout.contains("https://cdn.example.com/result-video.mp4"),
        "stdout should contain the video URL: {stdout}"
    );
    // Stderr should have progress info.
    assert!(
        stderr.contains("request_id=vid_gen_test_001"),
        "stderr should show request_id: {stderr}"
    );
    assert!(
        stderr.contains("video ready"),
        "stderr should report video ready: {stderr}"
    );
}

/// `grokrs generate video` should handle polling (pending then done).
#[tokio::test]
async fn generate_video_polls_pending_then_done() {
    let server = MockServer::start().await;

    // Submit endpoint.
    Mock::given(method("POST"))
        .and(path("/v1/videos/generations"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "request_id": "vid_poll_test"
        })))
        .expect(1)
        .mount(&server)
        .await;

    // First poll returns pending, second returns done.
    Mock::given(method("GET"))
        .and(path("/v1/videos/vid_poll_test"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "status": "pending",
            "progress": 50
        })))
        .up_to_n_times(1)
        .mount(&server)
        .await;

    Mock::given(method("GET"))
        .and(path("/v1/videos/vid_poll_test"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "status": "done",
            "video": {
                "url": "https://cdn.example.com/polled-video.mp4"
            }
        })))
        .mount(&server)
        .await;

    let config = write_test_config(&server.uri());

    let output = Command::new(env!("CARGO_BIN_EXE_grokrs"))
        .args([
            "--config",
            config.path().to_str().unwrap(),
            "generate",
            "video",
            "a dog running",
        ])
        .env("GROKRS_GEN_TEST_KEY", "test-key-gen-video-poll")
        .output()
        .expect("failed to run grokrs");

    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);

    assert!(output.status.success(), "expected exit 0, stderr: {stderr}");
    assert!(
        stdout.contains("https://cdn.example.com/polled-video.mp4"),
        "stdout should contain the video URL after polling: {stdout}"
    );
}

/// `grokrs generate image` with an absolute output path should fail because
/// `WorkspacePath` rejects absolute paths.
#[tokio::test]
async fn generate_image_rejects_absolute_output_path() {
    let server = MockServer::start().await;
    let config = write_test_config(&server.uri());

    let output = Command::new(env!("CARGO_BIN_EXE_grokrs"))
        .args([
            "--config",
            config.path().to_str().unwrap(),
            "generate",
            "image",
            "test prompt",
            "-o",
            "/tmp/absolute.png",
        ])
        .env("GROKRS_GEN_TEST_KEY", "test-key-gen-abs-path")
        .output()
        .expect("failed to run grokrs");

    assert!(
        !output.status.success(),
        "should fail with absolute output path"
    );
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("workspace-relative") || stderr.contains("invalid"),
        "stderr should mention workspace-relative path requirement: {stderr}"
    );
}

/// `grokrs generate image` with a path traversal attempt should fail.
#[tokio::test]
async fn generate_image_rejects_path_traversal() {
    let server = MockServer::start().await;
    let config = write_test_config(&server.uri());

    let output = Command::new(env!("CARGO_BIN_EXE_grokrs"))
        .args([
            "--config",
            config.path().to_str().unwrap(),
            "generate",
            "image",
            "test prompt",
            "-o",
            "../escape/image.png",
        ])
        .env("GROKRS_GEN_TEST_KEY", "test-key-gen-traversal")
        .output()
        .expect("failed to run grokrs");

    assert!(
        !output.status.success(),
        "should fail with path traversal attempt"
    );
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("workspace-relative")
            || stderr.contains("invalid")
            || stderr.contains("traversal"),
        "stderr should mention path validation failure: {stderr}"
    );
}
