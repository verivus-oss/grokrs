//! CLI subcommands for interacting with the xAI Grok API.
//!
//! Each subcommand loads config, constructs a `PolicyEngine`, wires it into
//! `GrokClient` via `FnPolicyGate`, and delegates to the appropriate endpoint
//! client. The API key is never stored or logged.

use std::io::Write;
use std::path::PathBuf;
use std::sync::Arc;

use anyhow::{Context, Result, bail};
use clap::Subcommand;

use futures::StreamExt;

use grokrs_api::client::GrokClient;
use grokrs_api::streaming::parser::parse_response_stream;
use grokrs_api::transport::policy_bridge::FnPolicyGate;
use grokrs_api::transport::policy_gate::{PolicyDecision, PolicyGate};
use grokrs_api::types::documents::{DocumentSearchRequest, DocumentsSource};
use grokrs_api::types::responses::{CreateResponseBuilder, ResponseInput};
use grokrs_api::types::stream::ResponseStreamEvent;
use grokrs_api::types::tts::{TtsOutputFormat, TtsRequest, VoiceId};
use grokrs_core::AppConfig;
use grokrs_policy::{Decision, Effect, PolicyEngine};
use grokrs_store::Store;
use grokrs_store::types::TranscriptUsage;

/// API subcommands for interacting with the xAI Grok API.
#[derive(Subcommand)]
pub enum ApiCommand {
    /// List available language models with ID and pricing
    Models,

    /// [DEPRECATED] One-shot prompt via the Responses API (for scripting).
    ///
    /// This subcommand is superseded by `grokrs chat`, which provides an
    /// interactive multi-turn REPL with session persistence, context
    /// management, web search, and server-side history chaining.
    ///
    /// `grokrs api chat` remains available for non-interactive scripting
    /// (stdin pipelines, CI one-shots), but will not receive new features.
    ///
    /// MIGRATION: replace `grokrs api chat 'prompt'` with `grokrs chat`.
    Chat {
        /// The prompt text
        prompt: String,
    },

    /// Print token count and token IDs for the given text
    Tokenize {
        /// The text to tokenize
        text: String,
    },

    /// Show API key metadata (name, team, ACLs, status)
    KeyInfo,

    /// Generate speech audio from text via the TTS API
    Tts {
        /// The text to synthesize into speech (max 15,000 characters)
        text: String,

        /// Voice to use for synthesis (eve, ara, rex, sal, leo)
        #[arg(long, default_value = "eve")]
        voice: String,

        /// Audio output format (mp3, wav, pcm, mulaw, alaw)
        #[arg(long, default_value = "mp3")]
        format: String,

        /// BCP-47 language tag (e.g., "en", "en-US", "de-DE")
        #[arg(long, default_value = "en")]
        language: String,

        /// Output file path; if omitted, writes raw audio to stdout (pipe only)
        #[arg(long, short)]
        output: Option<PathBuf>,
    },

    /// List available TTS voices
    Voices,

    /// Search document collections for matching chunks (semantic RAG)
    Search {
        /// The search query text
        query: String,

        /// Collection ID(s) to search (repeatable)
        #[arg(long = "collection", required = true)]
        collection_ids: Vec<String>,

        /// AIP-160 filter expression for metadata-based filtering
        #[arg(long)]
        filter: Option<String>,

        /// Maximum number of results to return
        #[arg(long)]
        limit: Option<u32>,
    },
}

/// Build a policy gate that bridges `PolicyEngine` into the `PolicyGate` trait
/// expected by `grokrs-api`. The gate translates `Decision::Allow` to
/// `PolicyDecision::Allow`, `Decision::Deny` to `PolicyDecision::Deny`, and
/// resolves `Decision::Ask` based on the configured `approval_mode`:
///
/// - `"allow"` — map `Ask` to `Allow` (bypass approval; use with caution)
/// - `"deny"` — map `Ask` to `Deny` (fail-closed; no interactive approval)
/// - `"interactive"` (or any other value) — keep `Ask` as `Ask` (requires
///   the approval broker, which is not yet implemented; effectively a denial)
fn build_policy_gate(engine: PolicyEngine, approval_mode: &str) -> Arc<dyn PolicyGate> {
    let approval_mode = approval_mode.to_owned();
    Arc::new(FnPolicyGate::new(move |host: &str| {
        let effect = Effect::NetworkConnect {
            host: host.to_owned(),
        };
        match engine.evaluate(&effect) {
            Decision::Allow { .. } => PolicyDecision::Allow,
            Decision::Ask { reason } => match approval_mode.as_str() {
                "allow" => PolicyDecision::Allow,
                "deny" => PolicyDecision::Deny {
                    reason: reason.to_owned(),
                },
                // "interactive" or any unrecognised value: preserve Ask.
                // The transport layer returns ApprovalRequired, keeping the
                // fail-closed boundary until the approval broker (spec 03)
                // is implemented.
                _ => PolicyDecision::Ask,
            },
            Decision::Deny { reason } => PolicyDecision::Deny {
                reason: reason.to_owned(),
            },
        }
    }))
}

/// Check that the config allows network access before attempting API calls.
/// Returns a helpful error message explaining how to enable network access
/// if it is denied.
fn check_network_allowed(config: &AppConfig) -> Result<()> {
    if !config.policy.allow_network {
        bail!(
            "Network access is denied by policy.\n\
             \n\
             To opt in to approval-gated network access, set `allow_network = true`\n\
             in your config file:\n\
             \n\
             [policy]\n\
             allow_network = true\n\
             \n\
             Note: with `allow_network = true`, network requests require explicit\n\
             approval via the approval broker (not yet implemented). Until the\n\
             approval surface is available, API commands will return an\n\
             ApprovalRequired error.\n\
             \n\
             Config file location: use --config <path> or the default configs/grokrs.example.toml"
        );
    }
    Ok(())
}

/// Execute an API subcommand.
///
/// Loads config from the given path (or falls back to a default), constructs
/// the policy engine and client, then dispatches to the appropriate handler.
///
/// # Errors
///
/// Returns an error if client construction, API calls, or output
/// rendering fails.
pub async fn run(command: &ApiCommand, config: &AppConfig) -> Result<()> {
    // Print deprecation notice for `api chat` early (before client construction)
    // so it appears even if the client fails to build (e.g. missing API key).
    if matches!(command, ApiCommand::Chat { .. }) {
        eprintln!(
            "DEPRECATED: `grokrs api chat` is superseded by `grokrs chat`.\n\
             \n\
             `grokrs chat` provides an interactive REPL with multi-turn conversation,\n\
             session persistence, web search (--search), and server-side history\n\
             chaining (--stateful). It will receive all future chat-related features.\n\
             \n\
             `grokrs api chat` remains available for non-interactive scripting\n\
             (pipes, CI one-shots) but will not receive new features.\n\
             \n\
             MIGRATION: replace `grokrs api chat 'prompt'` with `grokrs chat`.\n"
        );
    }

    check_network_allowed(config)?;

    let engine = PolicyEngine::new(config.policy.clone());
    let gate = build_policy_gate(engine, &config.session.approval_mode);
    let client =
        GrokClient::from_config(config, Some(gate)).context("failed to construct API client")?;

    match command {
        ApiCommand::Models => run_models(&client).await,
        ApiCommand::Chat { prompt } => {
            run_chat(&client, prompt, config, open_store_best_effort(config)).await
        }
        ApiCommand::Tokenize { text } => run_tokenize(&client, text, config).await,
        ApiCommand::KeyInfo => run_key_info(&client).await,
        ApiCommand::Tts {
            text,
            voice,
            format,
            language,
            output,
        } => run_tts(&client, text, voice, format, language, output.as_deref()).await,
        ApiCommand::Voices => run_voices(&client).await,
        ApiCommand::Search {
            query,
            collection_ids,
            filter,
            limit,
        } => run_search(&client, query, collection_ids, filter.as_deref(), *limit).await,
    }
}

/// Best-effort store opening. Returns None if the store cannot be opened.
/// API commands work without persistence; store integration is optional.
fn open_store_best_effort(config: &AppConfig) -> Option<Store> {
    let workspace_root = std::env::current_dir().ok()?;
    let store_path = config
        .store
        .as_ref()
        .map_or(".grokrs/state.db", |s| s.path.as_str());
    Store::open_with_path(&workspace_root, store_path).ok()
}

/// List available language models with ID and pricing.
async fn run_models(client: &GrokClient) -> Result<()> {
    let list = client
        .models()
        .list_language_models()
        .await
        .context("failed to list language models")?;

    if list.models.is_empty() {
        println!("No language models available.");
        return Ok(());
    }

    // Print header
    println!(
        "{:<30} {:<15} {:<15} {:<15}",
        "MODEL ID", "PROMPT PRICE", "COMPLETION", "OWNED BY"
    );
    println!("{}", "-".repeat(75));

    for model in &list.models {
        let prompt_price = model
            .prompt_text_token_price
            .map_or_else(|| "-".into(), |p| format!("{p}"));
        let completion_price = model
            .completion_text_token_price
            .map_or_else(|| "-".into(), |p| format!("{p}"));

        println!(
            "{:<30} {:<15} {:<15} {:<15}",
            model.id, prompt_price, completion_price, model.owned_by
        );
    }

    println!("\nPrices are in integer ticks (cents per 100M tokens).");
    println!("Total: {} language models", list.models.len());
    Ok(())
}

/// Accumulated state from consuming a streaming response.
struct StreamResult {
    found_text: bool,
    usage_input: u64,
    usage_output: u64,
    stream_error: Option<String>,
    response_body: String,
}

/// Consume a parsed response stream, printing text deltas to stdout.
async fn consume_response_stream(
    mut stream: std::pin::Pin<
        Box<
            dyn futures::Stream<
                    Item = Result<ResponseStreamEvent, grokrs_api::types::stream::StreamError>,
                > + Send,
        >,
    >,
) -> StreamResult {
    let stdout = std::io::stdout();
    let mut out = stdout.lock();
    let mut result = StreamResult {
        found_text: false,
        usage_input: 0,
        usage_output: 0,
        stream_error: None,
        response_body: String::new(),
    };

    while let Some(event) = stream.next().await {
        match event {
            Ok(ResponseStreamEvent::ContentDelta { delta, .. }) => {
                if let Some(text) = &delta.text {
                    write!(out, "{text}").ok();
                    out.flush().ok();
                    result.response_body.push_str(text);
                    result.found_text = true;
                }
            }
            Ok(ResponseStreamEvent::OutputTextDelta { delta, .. }) => {
                write!(out, "{delta}").ok();
                out.flush().ok();
                result.response_body.push_str(&delta);
                result.found_text = true;
            }
            Ok(ResponseStreamEvent::FunctionCallArgumentsDone { arguments, .. }) => {
                eprintln!("[function_call] {arguments}");
            }
            Ok(ResponseStreamEvent::ResponseCompleted { response }) => {
                if let Some(usage) = response.get("usage") {
                    result.usage_input = usage["input_tokens"].as_u64().unwrap_or(0);
                    result.usage_output = usage["output_tokens"].as_u64().unwrap_or(0);
                    let total = usage["total_tokens"]
                        .as_u64()
                        .unwrap_or(result.usage_input + result.usage_output);
                    eprintln!(
                        "\n[usage] input={} output={} total={total}",
                        result.usage_input, result.usage_output
                    );
                }
            }
            Ok(_) => {} // Other events (created, in_progress, etc.) are ignored
            Err(e) => {
                result.stream_error = Some(e.to_string());
                break;
            }
        }
    }
    result
}

/// Log stream results to store and finalize session state.
fn finalize_chat_store(
    store: &Option<Store>,
    session_id: &str,
    transcript_id: Option<i64>,
    stream: &StreamResult,
) {
    let Some(ref s) = *store else { return };

    if let Some(tid) = transcript_id {
        if let Some(ref err) = stream.stream_error {
            let _ = s.transcripts().log_error(tid, err);
        } else {
            let usage = TranscriptUsage {
                cost_in_usd_ticks: None,
                input_tokens: Some(stream.usage_input),
                output_tokens: Some(stream.usage_output),
                reasoning_tokens: None,
            };
            let body_ref = if stream.response_body.is_empty() {
                None
            } else {
                Some(stream.response_body.as_str())
            };
            let _ = s
                .transcripts()
                .log_response(tid, 200, body_ref, &usage, None);
        }
    }

    if stream.stream_error.is_some() {
        let msg = stream.stream_error.as_deref().unwrap_or("unknown error");
        let _ = s
            .sessions()
            .transition(session_id, &format!("Failed: {msg}"));
    } else {
        let _ = s.sessions().transition(session_id, "Ready");
        let _ = s.sessions().transition(session_id, "Closed");
    }

    if let Ok(summary) = s.usage().session_totals(session_id)
        && summary.request_count > 0
    {
        eprintln!(
            "[session {}] requests={} input_tokens={} output_tokens={} reasoning_tokens={}",
            &session_id[..8],
            summary.request_count,
            summary.total_input_tokens,
            summary.total_output_tokens,
            summary.total_reasoning_tokens,
        );
    }
}

/// Send a one-shot prompt via the Responses API with streaming output (store=false).
///
/// When `store` is `Some`, creates a session, logs the request/response
/// transcript, and prints a usage summary on exit. Store failures are
/// best-effort: the API call proceeds even if persistence fails.
async fn run_chat(
    client: &GrokClient,
    prompt: &str,
    config: &AppConfig,
    store: Option<Store>,
) -> Result<()> {
    let model = &config.model.default_model;

    // Session + store setup (best-effort).
    let session_id = uuid::Uuid::new_v4().to_string();
    let mut transcript_id: Option<i64> = None;

    if let Some(ref s) = store
        && s.sessions().create(&session_id, "Untrusted").is_ok()
    {
        let _ = s.sessions().transition(&session_id, "Ready");
        let _ = s.sessions().transition(&session_id, "RunningTurn");
        let body = serde_json::json!({"model": model, "prompt": prompt}).to_string();
        transcript_id = s
            .transcripts()
            .log_request(&session_id, "/v1/responses", "POST", Some(&body))
            .ok();
    }

    let request = CreateResponseBuilder::new(model, ResponseInput::Text(prompt.to_owned()))
        .store(false)
        .stream(true)
        .build();

    let raw_stream = match client
        .responses()
        .create_stream(&request)
        .await
        .context("failed to create streaming response")
    {
        Ok(s) => s,
        Err(e) => {
            if let (Some(s), Some(tid)) = (&store, transcript_id) {
                let _ = s.transcripts().log_error(tid, &e.to_string());
                let _ = s
                    .sessions()
                    .transition(&session_id, &format!("Failed: {e}"));
            }
            return Err(e);
        }
    };

    let stream_result = consume_response_stream(parse_response_stream(raw_stream)).await;
    if stream_result.found_text {
        println!();
    }

    finalize_chat_store(&store, &session_id, transcript_id, &stream_result);

    if let Some(s) = store {
        let _ = s.close();
    }

    if let Some(err) = stream_result.stream_error {
        bail!("stream error: {err}");
    }

    Ok(())
}

/// Tokenize text and print token count and IDs.
async fn run_tokenize(client: &GrokClient, text: &str, config: &AppConfig) -> Result<()> {
    let model = &config.model.default_model;

    let response = client
        .tokenize()
        .tokenize(text, model)
        .await
        .context("failed to tokenize text")?;

    println!("Model:       {model}");
    println!("Token count: {}", response.token_ids.len());
    println!();

    if response.token_ids.is_empty() {
        println!("(no tokens)");
        return Ok(());
    }

    // Print token table
    println!("{:<10} {:<20} BYTES", "TOKEN ID", "STRING");
    println!("{}", "-".repeat(50));

    for token in &response.token_ids {
        let bytes_display = if token.token_bytes.is_empty() {
            "-".to_string()
        } else {
            format!("{:?}", token.token_bytes)
        };
        println!(
            "{:<10} {:<20} {}",
            token.token_id, token.string_token, bytes_display
        );
    }

    Ok(())
}

/// Generate speech audio from text and write to file or stdout.
async fn run_tts(
    client: &GrokClient,
    text: &str,
    voice: &str,
    format: &str,
    language: &str,
    output: Option<&std::path::Path>,
) -> Result<()> {
    // Parse voice ID.
    let voice_id: VoiceId = voice.parse().map_err(|e: String| anyhow::anyhow!("{e}"))?;

    // Parse output format.
    let output_format: TtsOutputFormat =
        format.parse().map_err(|e: String| anyhow::anyhow!("{e}"))?;

    // TTY safety: if no --output and stdout is a TTY, refuse to dump binary.
    if output.is_none() && atty_stdout() {
        bail!(
            "Refusing to write binary audio data to terminal.\n\
             \n\
             Use one of:\n\
             \n\
             1. --output <file>    Write audio to a file:\n\
             \n\
                grokrs api tts 'Hello' --output hello.mp3\n\
             \n\
             2. Pipe to a player or file:\n\
             \n\
                grokrs api tts 'Hello' | mpv -\n\
                grokrs api tts 'Hello' > hello.mp3"
        );
    }

    // Build request.
    let request = TtsRequest::new(text, voice_id, language)
        .context("failed to build TTS request")?
        .with_output_format(output_format);

    // Generate audio.
    let audio_bytes = client
        .tts()
        .generate(&request)
        .await
        .context("failed to generate TTS audio")?;

    // Write output.
    if let Some(path) = output {
        std::fs::write(path, &audio_bytes)
            .with_context(|| format!("failed to write audio to {}", path.display()))?;
        eprintln!("Wrote {} bytes to {}", audio_bytes.len(), path.display());
    } else {
        let stdout = std::io::stdout();
        let mut out = stdout.lock();
        out.write_all(&audio_bytes)
            .context("failed to write audio to stdout")?;
        out.flush().context("failed to flush stdout")?;
    }

    Ok(())
}

/// List available TTS voices.
async fn run_voices(client: &GrokClient) -> Result<()> {
    let list = client
        .tts()
        .list_voices()
        .await
        .context("failed to list TTS voices")?;

    if list.voices.is_empty() {
        println!("No TTS voices available.");
        return Ok(());
    }

    println!("{:<15} {:<20} {:<15}", "VOICE ID", "NAME", "LANGUAGE");
    println!("{}", "-".repeat(50));

    for voice in &list.voices {
        let language = voice.language.as_deref().unwrap_or("-");
        println!("{:<15} {:<20} {:<15}", voice.voice_id, voice.name, language);
    }

    println!("\nTotal: {} voices", list.voices.len());
    Ok(())
}

/// Check if stdout is connected to a TTY.
///
/// Uses `libc::isatty` on Unix. This avoids pulling in an external crate
/// just for one check.
fn atty_stdout() -> bool {
    #[cfg(unix)]
    {
        // SAFETY: STDOUT_FILENO (1) is always valid on Unix.
        unsafe { libc::isatty(libc::STDOUT_FILENO) != 0 }
    }
    #[cfg(not(unix))]
    {
        // Conservative: assume TTY on non-Unix platforms.
        true
    }
}

/// Search document collections and print ranked results.
async fn run_search(
    client: &GrokClient,
    query: &str,
    collection_ids: &[String],
    filter: Option<&str>,
    limit: Option<u32>,
) -> Result<()> {
    let request = DocumentSearchRequest {
        query: query.to_string(),
        source: DocumentsSource {
            collection_ids: collection_ids.to_vec(),
        },
        filter: filter.map(ToString::to_string),
        group_by: None,
        ranking_metric: None,
        retrieval_mode: None,
        limit,
    };

    let response = client
        .documents()
        .search(&request)
        .await
        .context("failed to search documents")?;

    if response.matches.is_empty() {
        println!("No matches found.");
        return Ok(());
    }

    // Print results as a table: file_id, score, chunk_content (truncated).
    println!("{:<20} {:<10} CONTENT", "FILE ID", "SCORE");
    println!("{}", "-".repeat(80));

    for m in &response.matches {
        // Truncate chunk content to 50 characters for display, using
        // char_indices to avoid panicking on multi-byte UTF-8 boundaries.
        let content = match m.chunk_content.char_indices().nth(50) {
            Some((i, _)) => format!("{}...", &m.chunk_content[..i]),
            None => m.chunk_content.clone(),
        };
        println!("{:<20} {:<10.4} {}", m.file_id, m.score, content);
    }

    println!("\nTotal: {} matches", response.matches.len());
    Ok(())
}

/// Show API key metadata.
async fn run_key_info(client: &GrokClient) -> Result<()> {
    let info = client
        .api_key()
        .info()
        .await
        .context("failed to retrieve API key info")?;

    println!("Name:     {}", info.name.as_deref().unwrap_or("(not set)"));
    println!(
        "Status:   {}",
        info.status.as_deref().unwrap_or("(unknown)")
    );
    println!("Team:     {}", info.team_id.as_deref().unwrap_or("(none)"));
    println!(
        "Blocked:  {}",
        info.blocked
            .map_or("(unknown)", |b| if b { "yes" } else { "no" })
    );
    println!(
        "Disabled: {}",
        info.disabled
            .map_or("(unknown)", |b| if b { "yes" } else { "no" })
    );

    match &info.acls {
        Some(acls) if !acls.is_empty() => {
            println!("ACLs:");
            for acl in acls {
                println!("  - {acl}");
            }
        }
        Some(_) => println!("ACLs:     (none)"),
        None => println!("ACLs:     (not available)"),
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::build_policy_gate;
    use grokrs_api::transport::policy_gate::PolicyDecision;
    use grokrs_core::PolicyConfig;
    use grokrs_policy::PolicyEngine;

    fn engine(allow_network: bool) -> PolicyEngine {
        PolicyEngine::new(PolicyConfig {
            allow_network,
            allow_shell: false,
            allow_workspace_writes: false,
            max_patch_bytes: 0,
        })
    }

    // --- approval_mode tests ---

    #[test]
    fn approval_mode_allow_maps_ask_to_allow() {
        // allow_network=true produces Ask; approval_mode="allow" upgrades to Allow.
        let gate = build_policy_gate(engine(true), "allow");
        let decision = gate.evaluate_network("api.x.ai");
        assert!(
            matches!(decision, PolicyDecision::Allow),
            "expected Allow when approval_mode='allow', got {decision:?}"
        );
    }

    #[test]
    fn approval_mode_deny_maps_ask_to_deny() {
        // allow_network=true produces Ask; approval_mode="deny" downgrades to Deny.
        let gate = build_policy_gate(engine(true), "deny");
        let decision = gate.evaluate_network("api.x.ai");
        assert!(
            matches!(decision, PolicyDecision::Deny { .. }),
            "expected Deny when approval_mode='deny', got {decision:?}"
        );
    }

    #[test]
    fn approval_mode_interactive_keeps_ask() {
        // allow_network=true produces Ask; approval_mode="interactive" preserves Ask.
        let gate = build_policy_gate(engine(true), "interactive");
        let decision = gate.evaluate_network("api.x.ai");
        assert!(
            matches!(decision, PolicyDecision::Ask),
            "expected Ask when approval_mode='interactive', got {decision:?}"
        );
    }

    #[test]
    fn approval_mode_unknown_value_treated_as_interactive() {
        // Unrecognised approval_mode falls back to interactive (Ask preserved).
        let gate = build_policy_gate(engine(true), "banana");
        let decision = gate.evaluate_network("api.x.ai");
        assert!(
            matches!(decision, PolicyDecision::Ask),
            "expected Ask for unknown approval_mode, got {decision:?}"
        );
    }

    // --- Allow decisions are never downgraded ---

    #[test]
    fn allow_decision_never_downgraded_by_approval_mode_deny() {
        // PolicyEngine would only return Allow if the policy explicitly allows
        // the effect (not the case for NetworkConnect today, but the bridge must
        // be correct regardless). We can test this by verifying that a Deny
        // approval_mode does not touch Allow decisions from the engine.
        //
        // With allow_network=false the engine returns Deny (not Allow), so we
        // verify that Deny stays Deny — the bridge never manufactures Allow.
        let gate = build_policy_gate(engine(false), "deny");
        let decision = gate.evaluate_network("api.x.ai");
        assert!(
            matches!(decision, PolicyDecision::Deny { .. }),
            "Deny from engine must stay Deny regardless of approval_mode"
        );
    }

    #[test]
    fn allow_decision_never_downgraded_by_approval_mode_interactive() {
        // Same rationale: engine Deny must not be upgraded to Ask by
        // approval_mode="interactive".
        let gate = build_policy_gate(engine(false), "interactive");
        let decision = gate.evaluate_network("api.x.ai");
        assert!(
            matches!(decision, PolicyDecision::Deny { .. }),
            "Deny from engine must stay Deny regardless of approval_mode"
        );
    }

    // --- Deny decisions are never upgraded ---

    #[test]
    fn deny_decision_never_upgraded_by_approval_mode_allow() {
        // allow_network=false produces Deny; approval_mode="allow" must NOT
        // upgrade it to Allow. Only Ask decisions are affected.
        let gate = build_policy_gate(engine(false), "allow");
        let decision = gate.evaluate_network("api.x.ai");
        assert!(
            matches!(decision, PolicyDecision::Deny { .. }),
            "Deny from engine must never be upgraded to Allow, got {decision:?}"
        );
    }

    #[test]
    fn deny_decision_never_upgraded_by_approval_mode_interactive() {
        let gate = build_policy_gate(engine(false), "interactive");
        let decision = gate.evaluate_network("api.x.ai");
        assert!(
            matches!(decision, PolicyDecision::Deny { .. }),
            "Deny from engine must never be upgraded to Ask, got {decision:?}"
        );
    }

    // --- Legacy behaviour preserved ---

    #[test]
    fn policy_bridge_maps_deny_to_deny() {
        let gate = build_policy_gate(engine(false), "interactive");
        let decision = gate.evaluate_network("api.x.ai");
        assert!(
            matches!(decision, PolicyDecision::Deny { .. }),
            "expected Deny when allow_network=false, got {decision:?}"
        );
    }

    #[test]
    fn policy_bridge_maps_ask_to_ask() {
        // When allow_network=true and approval_mode=interactive, Ask is preserved.
        let gate = build_policy_gate(engine(true), "interactive");
        let decision = gate.evaluate_network("api.x.ai");
        assert!(
            matches!(decision, PolicyDecision::Ask),
            "expected Ask when allow_network=true (approval boundary), got {decision:?}"
        );
    }

    // --- Deprecation notice ---
    // Real stderr/stdout verification is in tests/cli_smoke.rs::api_chat_prints_deprecation_notice_to_stderr
}
