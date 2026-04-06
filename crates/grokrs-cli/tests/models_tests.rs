//! Integration tests for `grokrs models list`, `grokrs models info`, and
//! `grokrs models pricing` CLI subcommands.
//!
//! Each test starts a wiremock `MockServer`, writes a TOML config pointing at
//! the mock's URL, sets the `XAI_API_KEY` env var, and runs the binary via
//! `std::process::Command`. Assertions are against stdout/stderr text and exit
//! codes.

use std::io::Write as _;
use std::process::Command;

use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

/// Helper: write a TOML config to a temp file pointing at the given mock base
/// URL with network enabled and approval mode set to "allow".
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
api_key_env = "GROKRS_MODELS_TEST_KEY"
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
allow_workspace_writes = false
max_patch_bytes = 0

[session]
approval_mode = "deny"
transcript_dir = ".grokrs/sessions"
"#
    )
    .unwrap();
    f.flush().unwrap();
    f
}

/// JSON body for a `/v1/language-models` response with two models.
fn language_models_response() -> serde_json::Value {
    serde_json::json!({
        "models": [
            {
                "id": "grok-4",
                "created": 1_700_000_000,
                "owned_by": "xai",
                "aliases": ["grok-latest"],
                "input_modalities": ["text", "image"],
                "output_modalities": ["text"],
                "prompt_text_token_price": 300,
                "completion_text_token_price": 1500,
                "cached_prompt_text_token_price": 150,
                "max_prompt_length": 131_072
            },
            {
                "id": "grok-4-mini",
                "created": 1_700_000_001,
                "owned_by": "xai",
                "aliases": [],
                "input_modalities": ["text"],
                "output_modalities": ["text"],
                "prompt_text_token_price": 100,
                "completion_text_token_price": 500,
                "cached_prompt_text_token_price": 50,
                "max_prompt_length": 131_072
            }
        ]
    })
}

/// JSON body for a `/v1/image-generation-models` response.
fn image_models_response() -> serde_json::Value {
    serde_json::json!({
        "models": [
            {
                "id": "grok-2-image",
                "created": 1_700_000_000,
                "owned_by": "xai",
                "input_modalities": ["text"],
                "output_modalities": ["image"],
                "per_image_price": 7000
            }
        ]
    })
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

/// `grokrs models list` should produce table output containing model IDs from
/// the mock `/v1/language-models` endpoint.
#[tokio::test]
async fn models_list_produces_table_with_model_ids() {
    let server = MockServer::start().await;

    Mock::given(method("GET"))
        .and(path("/v1/language-models"))
        .respond_with(ResponseTemplate::new(200).set_body_json(language_models_response()))
        .expect(1)
        .mount(&server)
        .await;

    let config = write_test_config(&server.uri());

    let output = Command::new(env!("CARGO_BIN_EXE_grokrs"))
        .args([
            "--config",
            config.path().to_str().unwrap(),
            "models",
            "list",
        ])
        .env("GROKRS_MODELS_TEST_KEY", "test-key-models-list")
        .output()
        .expect("failed to run grokrs");

    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);

    assert!(output.status.success(), "expected exit 0, stderr: {stderr}");
    assert!(
        stdout.contains("grok-4"),
        "stdout should contain model id 'grok-4': {stdout}"
    );
    assert!(
        stdout.contains("grok-4-mini"),
        "stdout should contain model id 'grok-4-mini': {stdout}"
    );
    // Table header should be present.
    assert!(
        stdout.contains("MODEL ID"),
        "stdout should contain table header: {stdout}"
    );
    assert!(
        stdout.contains("PROMPT"),
        "stdout should contain PROMPT column header: {stdout}"
    );
}

/// `grokrs models list --json` should produce valid JSON output.
#[tokio::test]
async fn models_list_json_outputs_valid_json() {
    let server = MockServer::start().await;

    Mock::given(method("GET"))
        .and(path("/v1/language-models"))
        .respond_with(ResponseTemplate::new(200).set_body_json(language_models_response()))
        .expect(1)
        .mount(&server)
        .await;

    let config = write_test_config(&server.uri());

    let output = Command::new(env!("CARGO_BIN_EXE_grokrs"))
        .args([
            "--config",
            config.path().to_str().unwrap(),
            "models",
            "list",
            "--json",
        ])
        .env("GROKRS_MODELS_TEST_KEY", "test-key-models-json")
        .output()
        .expect("failed to run grokrs");

    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);

    assert!(output.status.success(), "expected exit 0, stderr: {stderr}");

    // The output should parse as valid JSON.
    let parsed: serde_json::Value = serde_json::from_str(stdout.trim()).unwrap_or_else(|e| {
        panic!("stdout is not valid JSON: {e}\nstdout: {stdout}");
    });

    // The JSON should contain the model data.
    assert!(
        parsed.get("models").is_some(),
        "JSON output should have 'models' key: {parsed}"
    );
    let models = parsed["models"].as_array().expect("models should be array");
    assert_eq!(models.len(), 2);
    assert_eq!(models[0]["id"], "grok-4");
    assert_eq!(models[1]["id"], "grok-4-mini");
}

/// `grokrs models info grok-4` should show details for the specified model.
/// The CLI tries language models first, so a mock on `/v1/language-models/grok-4`
/// should suffice.
#[tokio::test]
async fn models_info_shows_model_details() {
    let server = MockServer::start().await;

    let model_response = serde_json::json!({
        "id": "grok-4",
        "created": 1_700_000_000,
        "owned_by": "xai",
        "aliases": ["grok-latest"],
        "input_modalities": ["text", "image"],
        "output_modalities": ["text"],
        "prompt_text_token_price": 300,
        "completion_text_token_price": 1500,
        "cached_prompt_text_token_price": 150,
        "max_prompt_length": 131_072,
        "version": "2025-01-01"
    });

    Mock::given(method("GET"))
        .and(path("/v1/language-models/grok-4"))
        .respond_with(ResponseTemplate::new(200).set_body_json(&model_response))
        .expect(1)
        .mount(&server)
        .await;

    let config = write_test_config(&server.uri());

    let output = Command::new(env!("CARGO_BIN_EXE_grokrs"))
        .args([
            "--config",
            config.path().to_str().unwrap(),
            "models",
            "info",
            "grok-4",
        ])
        .env("GROKRS_MODELS_TEST_KEY", "test-key-models-info")
        .output()
        .expect("failed to run grokrs");

    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);

    assert!(output.status.success(), "expected exit 0, stderr: {stderr}");
    assert!(
        stdout.contains("grok-4"),
        "stdout should contain model id: {stdout}"
    );
    assert!(
        stdout.contains("Language Model"),
        "stdout should indicate this is a language model: {stdout}"
    );
    assert!(
        stdout.contains("xai"),
        "stdout should contain the owned_by value: {stdout}"
    );
    assert!(
        stdout.contains("131072"),
        "stdout should show max_prompt_length context window: {stdout}"
    );
}

/// `grokrs models pricing` should show a pricing table sorted by cost.
#[tokio::test]
async fn models_pricing_shows_pricing_table() {
    let server = MockServer::start().await;

    Mock::given(method("GET"))
        .and(path("/v1/language-models"))
        .respond_with(ResponseTemplate::new(200).set_body_json(language_models_response()))
        .expect(1)
        .mount(&server)
        .await;

    let config = write_test_config(&server.uri());

    let output = Command::new(env!("CARGO_BIN_EXE_grokrs"))
        .args([
            "--config",
            config.path().to_str().unwrap(),
            "models",
            "pricing",
        ])
        .env("GROKRS_MODELS_TEST_KEY", "test-key-models-pricing")
        .output()
        .expect("failed to run grokrs");

    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);

    assert!(output.status.success(), "expected exit 0, stderr: {stderr}");
    assert!(
        stdout.contains("MODEL ID"),
        "stdout should contain pricing table header: {stdout}"
    );
    assert!(
        stdout.contains("PROMPT"),
        "stdout should contain PROMPT column: {stdout}"
    );
    assert!(
        stdout.contains("COMPLETION"),
        "stdout should contain COMPLETION column: {stdout}"
    );
    // grok-4-mini is cheaper (100) so should appear before grok-4 (300).
    let mini_pos = stdout
        .find("grok-4-mini")
        .expect("grok-4-mini should be in output");
    // Find grok-4 that is NOT grok-4-mini: search after the header line.
    let after_header = stdout.find("---").unwrap_or(0);
    let grok3_pos = stdout[after_header..]
        .find("grok-4\n")
        .or_else(|| stdout[after_header..].find("grok-4 "))
        .map(|p| p + after_header);
    if let Some(grok3_pos) = grok3_pos {
        assert!(
            mini_pos < grok3_pos,
            "grok-4-mini (cheaper) should appear before grok-4 in pricing table"
        );
    }
    assert!(
        stdout.contains("cheapest"),
        "stdout should mention sorting by cheapest: {stdout}"
    );
}

/// `grokrs models list --type image` should hit `/v1/image-generation-models`.
#[tokio::test]
async fn models_list_type_image_uses_image_models_endpoint() {
    let server = MockServer::start().await;

    Mock::given(method("GET"))
        .and(path("/v1/image-generation-models"))
        .respond_with(ResponseTemplate::new(200).set_body_json(image_models_response()))
        .expect(1)
        .mount(&server)
        .await;

    let config = write_test_config(&server.uri());

    let output = Command::new(env!("CARGO_BIN_EXE_grokrs"))
        .args([
            "--config",
            config.path().to_str().unwrap(),
            "models",
            "list",
            "--type",
            "image",
        ])
        .env("GROKRS_MODELS_TEST_KEY", "test-key-models-image")
        .output()
        .expect("failed to run grokrs");

    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);

    assert!(output.status.success(), "expected exit 0, stderr: {stderr}");
    assert!(
        stdout.contains("grok-2-image"),
        "stdout should contain image model id: {stdout}"
    );
    assert!(
        stdout.contains("PER IMAGE"),
        "stdout should contain PER IMAGE column: {stdout}"
    );
}

/// `grokrs models list` with network denied should fail with a clear error.
#[tokio::test]
async fn models_list_network_denied_fails_with_policy_error() {
    let config = write_network_denied_config();

    let output = Command::new(env!("CARGO_BIN_EXE_grokrs"))
        .args([
            "--config",
            config.path().to_str().unwrap(),
            "models",
            "list",
        ])
        .output()
        .expect("failed to run grokrs");

    assert!(
        !output.status.success(),
        "should fail when network is denied"
    );
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("Network access is denied")
            || stderr.contains("network")
            || stderr.contains("allow_network"),
        "stderr should mention network policy denial: {stderr}"
    );
}

/// `grokrs models pricing --json` should output valid JSON pricing data.
#[tokio::test]
async fn models_pricing_json_outputs_valid_json() {
    let server = MockServer::start().await;

    Mock::given(method("GET"))
        .and(path("/v1/language-models"))
        .respond_with(ResponseTemplate::new(200).set_body_json(language_models_response()))
        .expect(1)
        .mount(&server)
        .await;

    let config = write_test_config(&server.uri());

    let output = Command::new(env!("CARGO_BIN_EXE_grokrs"))
        .args([
            "--config",
            config.path().to_str().unwrap(),
            "models",
            "pricing",
            "--json",
        ])
        .env("GROKRS_MODELS_TEST_KEY", "test-key-pricing-json")
        .output()
        .expect("failed to run grokrs");

    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);

    assert!(output.status.success(), "expected exit 0, stderr: {stderr}");

    let parsed: serde_json::Value = serde_json::from_str(stdout.trim()).unwrap_or_else(|e| {
        panic!("stdout is not valid JSON: {e}\nstdout: {stdout}");
    });

    let arr = parsed.as_array().expect("pricing JSON should be an array");
    assert_eq!(arr.len(), 2, "should have pricing for 2 models");
    // Each entry should have model id and pricing fields.
    for entry in arr {
        assert!(entry.get("id").is_some(), "entry should have 'id': {entry}");
        assert!(
            entry.get("prompt_text_token_price").is_some(),
            "entry should have pricing: {entry}"
        );
    }
}
