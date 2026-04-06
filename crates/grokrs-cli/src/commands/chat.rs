//! Top-level `grokrs chat` command: interactive REPL with Grok API streaming
//! and optional SQLite session persistence.
//!
//! Launches the interactive REPL ([`run_repl`]) with a [`GrokChatBackend`]
//! connected to the xAI Grok Responses API. Supports model override, system
//! instructions, stateful conversation chaining, session persistence, and
//! session resume via `--resume <id>`.
//!
//! This is the primary user-facing interactive command -- distinct from
//! `grokrs api chat` which is a one-shot prompt/response.

use std::fmt::Write as _;
use std::sync::Arc;

use anyhow::{Context, Result, bail};
use clap::Args;

use grokrs_api::client::GrokClient;
use grokrs_api::transport::policy_bridge::FnPolicyGate;
use grokrs_api::transport::policy_gate::{PolicyDecision, PolicyGate};
use grokrs_cap::Untrusted;
use grokrs_core::AppConfig;
use grokrs_policy::{Decision, Effect, PolicyEngine};
use grokrs_session::{Session, SessionState};
use grokrs_store::Store;

use crate::repl::backend::{ChatBackend, TokenUsage};
use crate::repl::grok_backend::{GrokBackendConfig, GrokChatBackend};
use crate::repl::history::{ConversationHistory, ConversationTurn};
use crate::repl::{self, ReplConfig};

use super::search::{self, SearchConfig};

/// Arguments for the `grokrs chat` command.
#[derive(Debug, Args)]
#[command(after_help = "\
Examples:
  grokrs chat                                     Basic interactive chat
  grokrs chat --model grok-4-mini                 Use a specific model
  grokrs chat --system 'You are a Rust expert'    Set system instructions
  grokrs chat --stateful                          Enable server-side conversation chaining
  grokrs chat --resume abc123                     Resume a previous session
  grokrs chat --search                            Enable web search
  grokrs chat --search --x-search --citations     Web + X search with citations
  grokrs chat --search --search-from-date 2025-01-01  Search with date range
  grokrs chat --cache-key my-system-prompt        Enable prompt caching for the system prompt

See also: grokrs agent, grokrs api chat")]
pub struct ChatArgs {
    /// Override the default model (e.g., `grok-4`, `grok-4-mini`).
    #[arg(long)]
    pub model: Option<String>,

    /// Set initial system instructions for the conversation.
    #[arg(long)]
    pub system: Option<String>,

    /// Enable server-side conversation chaining (sets `store=true` on API
    /// requests and chains via `previous_response_id`).
    #[arg(long, default_value_t = false)]
    pub stateful: bool,

    /// Resume a previous session by ID or prefix. Loads conversation history
    /// from the store and continues where you left off.
    #[arg(long)]
    pub resume: Option<String>,

    /// Enable web search (adds `web_search` to the request tools array).
    #[arg(long)]
    pub search: bool,

    /// Enable X (Twitter) search (adds `x_search` to the request tools array).
    #[arg(long)]
    pub x_search: bool,

    /// Explicitly disable search, overriding `--search` and `--x-search`.
    #[arg(long)]
    pub no_search: bool,

    /// Earliest date for search results (ISO 8601: YYYY-MM-DD).
    #[arg(long)]
    pub search_from_date: Option<String>,

    /// Latest date for search results (ISO 8601: YYYY-MM-DD).
    #[arg(long)]
    pub search_to_date: Option<String>,

    /// Maximum number of search results to return.
    #[arg(long)]
    pub search_max_results: Option<u32>,

    /// Include citation URLs in the response when search is enabled.
    #[arg(long)]
    pub citations: bool,

    /// Enable server-side prompt caching by sending this key as
    /// `prompt_cache_key` in every Responses API request.
    ///
    /// Use a stable key that identifies the fixed portion of your prompt
    /// (e.g. the system instructions). The server caches the KV state of
    /// the matched prefix and reuses it on subsequent requests that send
    /// the same key, reducing latency and effective input-token cost.
    ///
    /// When a cache hit occurs the per-turn usage line shows the number of
    /// cached tokens: `input=500 (200 cached), output=100, total=600`.
    ///
    /// Omit this flag when the prompt changes every turn or when caching
    /// is not desired.
    #[arg(long, value_name = "KEY")]
    pub cache_key: Option<String>,
}

/// Run the `grokrs chat` command.
///
/// # Flow
///
/// 1. Check network policy -- fail fast with a helpful error if denied.
/// 2. Build the Grok API client with a policy gate.
/// 3. If `--resume`, load the previous session. Otherwise, create a new one.
/// 4. Open the store (best-effort) for session/transcript persistence.
/// 5. Construct the `GrokChatBackend` and launch the REPL with per-turn logging.
/// 6. On exit, transition session to `Closed` and print usage summary.
pub fn run(args: &ChatArgs, config: &AppConfig, rt: &tokio::runtime::Handle) -> Result<()> {
    // --- Network policy check ---
    check_network_policy(config)?;

    // --- Build API client ---
    let engine = PolicyEngine::new(config.policy.clone());
    let gate = build_policy_gate(engine, &config.session.approval_mode);
    let client =
        GrokClient::from_config(config, Some(gate)).context("failed to construct API client")?;

    // --- Resolve search config ---
    let search_config = resolve_search_config(args)?;

    // --- Build backend (CLI args override [chat] config defaults) ---
    let chat_config = config.chat.as_ref();
    let model = args
        .model
        .clone()
        .or_else(|| chat_config.and_then(|c| c.default_model.clone()))
        .unwrap_or_else(|| config.model.default_model.clone());

    let stateful = args.stateful || chat_config.is_some_and(|c| c.stateful);

    let backend_config = GrokBackendConfig {
        model,
        stateful,
        search: search_config,
        cache_key: args.cache_key.clone(),
    };

    let mut backend = GrokChatBackend::new(Arc::new(client), backend_config);

    // Apply system instructions: CLI arg overrides config default.
    let system = args
        .system
        .as_ref()
        .or_else(|| chat_config.and_then(|c| c.system_prompt.as_ref()));
    if let Some(system) = system {
        backend.set_system(system);
    }

    // --- Store integration (best-effort) ---
    let store = open_store_best_effort(config);

    // --- Session lifecycle ---
    let session_id;
    let initial_conversation;

    if let Some(ref prefix) = args.resume {
        // Resume an existing session.
        let (resolved_id, history, prev_response_id) = resolve_resume_session(&store, prefix)?;
        session_id = resolved_id;
        initial_conversation = history;

        // Reopen if closed.
        if let Some(ref s) = store {
            let _ = s.sessions().transition(&session_id, "Ready");
        }

        // Set the previous_response_id on the backend for stateful chaining.
        if let Some(ref resp_id) = prev_response_id {
            backend.set_previous_response_id(resp_id);
        }

        eprintln!("Resumed session: {session_id}");
        eprintln!(
            "  {} previous turn(s), input_tokens={}, output_tokens={}",
            initial_conversation.turn_count(),
            initial_conversation.total_input_tokens(),
            initial_conversation.total_output_tokens(),
        );
    } else {
        // Create a new session.
        session_id = uuid::Uuid::new_v4().to_string();
        initial_conversation = ConversationHistory::new();

        let mut session = Session::<Untrusted>::new(&session_id);
        session.transition(SessionState::Ready);

        if let Some(ref s) = store {
            if let Err(e) = s.sessions().create(&session_id, "Untrusted") {
                eprintln!("warning: failed to persist session: {e}");
            } else {
                let _ = s.sessions().transition(&session_id, "Ready");
            }
        }
    }

    // --- REPL config ---
    let workspace_root =
        std::env::current_dir().context("failed to resolve current working directory")?;
    let repl_config = ReplConfig {
        state_dir: workspace_root,
    };

    // --- Enter REPL ---
    if let Some(ref key) = args.cache_key {
        eprintln!(
            "grokrs chat | model={} | stateful={} | session={} | cache_key={key:?}",
            backend.model(),
            args.stateful,
            &session_id,
        );
    } else {
        eprintln!(
            "grokrs chat | model={} | stateful={} | session={}",
            backend.model(),
            args.stateful,
            &session_id,
        );
    }
    eprintln!("Type /help for available commands, /exit or Ctrl-D to quit.\n");

    let result = run_repl_with_store(
        backend,
        &repl_config,
        rt,
        store.as_ref(),
        &session_id,
        initial_conversation,
    );

    // --- Cleanup ---
    if let Some(ref s) = store {
        match &result {
            Ok(()) => {
                let _ = s.sessions().transition(&session_id, "Closed");
            }
            Err(e) => {
                let _ = s
                    .sessions()
                    .transition(&session_id, &format!("Failed: {e}"));
            }
        }

        // Print usage summary from store.
        if let Ok(summary) = s.usage().session_totals(&session_id)
            && summary.request_count > 0
        {
            eprintln!(
                "[session {}] requests={} input_tokens={} output_tokens={} reasoning_tokens={}",
                &session_id[..session_id.len().min(8)],
                summary.request_count,
                summary.total_input_tokens,
                summary.total_output_tokens,
                summary.total_reasoning_tokens,
            );
        }

        // Print cache key in use, if any.
        if let Some(ref key) = args.cache_key {
            eprintln!("[cache] prompt_cache_key={key:?} (see turn usage for cached_tokens)");
        }
    }

    // Close store (best-effort).
    if let Some(s) = store {
        let _ = s.close();
    }

    result
}

/// Run the REPL loop with per-turn store logging.
///
/// This replaces the generic `run_repl` when store integration is active. Each
/// conversation turn is wrapped with `log_request` before the API call and
/// `log_response` (or `log_error`) after. Session state transitions are tracked:
/// `Ready -> RunningTurn` before each API call, back to `Ready` after.
///
/// On readline errors, transitions to `Failed`. On clean exit (/exit or Ctrl-D),
/// the caller transitions to `Closed`.
fn run_repl_with_store<B: ChatBackend>(
    mut backend: B,
    config: &ReplConfig,
    rt: &tokio::runtime::Handle,
    store: Option<&Store>,
    session_id: &str,
    initial_conversation: ConversationHistory,
) -> Result<()> {
    use rustyline::DefaultEditor;
    use rustyline::error::ReadlineError;

    let mut editor = DefaultEditor::new()?;

    // Ensure the .grokrs directory exists for history persistence.
    let history_path = config.history_path();
    if let Some(parent) = history_path.parent() {
        std::fs::create_dir_all(parent).ok();
    }

    // Load existing readline history (best-effort).
    if history_path.exists()
        && let Err(e) = editor.load_history(&history_path)
    {
        eprintln!(
            "warning: could not load history from {}: {e}",
            history_path.display()
        );
    }

    let mut conversation = initial_conversation;
    let mut stdout = std::io::stdout();
    let turn_count_before = conversation.turn_count();

    loop {
        match editor.readline(repl::PROMPT) {
            Ok(line) => {
                let trimmed = line.trim();
                if trimmed.is_empty() {
                    continue;
                }

                let _ = editor.add_history_entry(trimmed);

                // Handle slash commands directly (no store logging needed).
                if trimmed.starts_with('/') {
                    let outcome = rt.block_on(repl::process_line(
                        &line,
                        &mut backend,
                        &mut conversation,
                        &mut stdout,
                    ));
                    if outcome == repl::LineOutcome::Exit {
                        break;
                    }
                    continue;
                }

                // Regular user message -- wrap with store logging.
                let turn_count_pre = conversation.turn_count();

                // Log request before sending (best-effort).
                let transcript_id = if let Some(s) = store {
                    let _ = s.sessions().transition(session_id, "RunningTurn");
                    let body = serde_json::json!({"message": trimmed}).to_string();
                    s.transcripts()
                        .log_request(session_id, "/v1/responses", "POST", Some(&body))
                        .ok()
                } else {
                    None
                };

                // Send to backend via process_line.
                let outcome = rt.block_on(repl::process_line(
                    &line,
                    &mut backend,
                    &mut conversation,
                    &mut stdout,
                ));

                // Log response after receiving (best-effort).
                if let Some(tid) = transcript_id
                    && let Some(s) = store
                {
                    let new_turn_added = conversation.turn_count() > turn_count_pre;
                    if new_turn_added {
                        if let Some(last_turn) = conversation.turns().last() {
                            let usage = grokrs_store::types::TranscriptUsage {
                                cost_in_usd_ticks: None,
                                input_tokens: Some(last_turn.usage.input_tokens),
                                output_tokens: Some(last_turn.usage.output_tokens),
                                reasoning_tokens: None,
                            };
                            let resp_body = if last_turn.assistant_response.is_empty() {
                                None
                            } else {
                                Some(last_turn.assistant_response.as_str())
                            };
                            let resp_id = conversation.last_response_id();
                            let _ = s
                                .transcripts()
                                .log_response(tid, 200, resp_body, &usage, resp_id);
                        }
                    } else {
                        // No turn recorded -- error occurred.
                        let _ = s.transcripts().log_error(tid, "no response recorded");
                    }
                    let _ = s.sessions().transition(session_id, "Ready");
                }

                if outcome == repl::LineOutcome::Exit {
                    break;
                }
            }

            // Ctrl-C: cancel current input, show new prompt.
            Err(ReadlineError::Interrupted) => {
                continue;
            }

            // Ctrl-D (EOF): exit cleanly.
            Err(ReadlineError::Eof) => {
                break;
            }

            // Other readline errors.
            Err(err) => {
                eprintln!("readline error: {err}");
                // Transition to Failed on unrecoverable error.
                if let Some(s) = store {
                    let _ = s
                        .sessions()
                        .transition(session_id, &format!("Failed: readline error: {err}"));
                }
                break;
            }
        }
    }

    // Print conversation summary on exit.
    if conversation.turn_count() > turn_count_before {
        eprintln!("Session summary: {conversation}");
    }

    // Save readline history (best-effort).
    if let Err(e) = editor.save_history(&history_path) {
        eprintln!(
            "warning: could not save history to {}: {e}",
            history_path.display()
        );
    }

    Ok(())
}

/// Resolve a session for --resume, supporting prefix match.
/// Returns (session_id, reconstructed_conversation, last_response_id).
fn resolve_resume_session(
    store: &Option<Store>,
    prefix: &str,
) -> Result<(String, ConversationHistory, Option<String>)> {
    let store = store.as_ref().context(
        "--resume requires a store database. \
         Ensure [store] is configured or a .grokrs/state.db exists.",
    )?;

    // Try exact match first.
    let session = if let Some(record) = store
        .sessions()
        .get(prefix)
        .context("failed to look up session")?
    {
        record
    } else {
        // Try prefix match.
        let matches = store
            .sessions()
            .find_by_prefix(prefix)
            .context("failed to search sessions by prefix")?;

        match matches.len() {
            0 => bail!("No session found matching '{prefix}'."),
            1 => matches.into_iter().next().unwrap(),
            n => {
                let mut msg =
                    format!("Ambiguous session ID prefix '{prefix}' matches {n} sessions:\n");
                for s in &matches {
                    write!(
                        msg,
                        "  {}  state={}  updated={}\n",
                        &s.id[..s.id.len().min(12)],
                        s.state,
                        s.updated_at,
                    )
                    .unwrap();
                }
                msg.push_str("\nPlease provide a longer prefix to uniquely identify the session.");
                bail!("{msg}");
            }
        }
    };

    let session_id = session.id.clone();

    // Reconstruct conversation history from transcripts.
    let transcripts = store
        .transcripts()
        .list_by_session(&session_id)
        .context("failed to load transcripts for session")?;

    let mut conversation = ConversationHistory::new();

    for t in &transcripts {
        // Extract user message from request body JSON.
        let user_input = t
            .request_body
            .as_deref()
            .and_then(|body| {
                serde_json::from_str::<serde_json::Value>(body)
                    .ok()
                    .and_then(|v| {
                        v.get("message")
                            .or_else(|| v.get("prompt"))
                            .and_then(|m| m.as_str().map(ToOwned::to_owned))
                    })
            })
            .unwrap_or_default();

        let assistant_response = t.response_body.clone().unwrap_or_default();

        let usage = TokenUsage {
            input_tokens: t.input_tokens.unwrap_or(0),
            output_tokens: t.output_tokens.unwrap_or(0),
            // Cached token counts are not persisted in the transcript store;
            // they are only reported during the live session turn.
            cached_tokens: None,
        };

        if !user_input.is_empty() || !assistant_response.is_empty() {
            conversation.push(ConversationTurn {
                user_input,
                assistant_response,
                usage,
            });
        }
    }

    // Get the last response_id for stateful chaining.
    let last_response_id = store
        .transcripts()
        .get_last_response_id(&session_id)
        .context("failed to get last response ID")?;

    // Set last_response_id on conversation for consistency.
    conversation.set_last_response_id(last_response_id.clone());

    Ok((session_id, conversation, last_response_id))
}

/// Resolve a [`SearchConfig`] from CLI flags with validation.
///
/// `--no-search` overrides `--search` and `--x-search`. Date flags are
/// validated as ISO 8601 (YYYY-MM-DD).
fn resolve_search_config(args: &ChatArgs) -> Result<SearchConfig> {
    // --no-search kills everything.
    if args.no_search {
        return Ok(SearchConfig::default());
    }

    // Validate date flags.
    if let Some(ref date) = args.search_from_date {
        search::validate_date(date).map_err(|e| anyhow::anyhow!("--search-from-date: {e}"))?;
    }
    if let Some(ref date) = args.search_to_date {
        search::validate_date(date).map_err(|e| anyhow::anyhow!("--search-to-date: {e}"))?;
    }

    Ok(SearchConfig {
        web_search: args.search,
        x_search: args.x_search,
        from_date: args.search_from_date.clone(),
        to_date: args.search_to_date.clone(),
        max_results: args.search_max_results,
        citations: args.citations,
    })
}

/// Check that the config allows network access before entering the REPL.
/// Returns a helpful error explaining how to enable network access if denied.
fn check_network_policy(config: &AppConfig) -> Result<()> {
    if !config.policy.allow_network {
        bail!(
            "Network access is denied by policy.\n\
             \n\
             The `grokrs chat` command requires network access to reach the xAI API.\n\
             To opt in, set `allow_network = true` in your config file:\n\
             \n\
             [policy]\n\
             allow_network = true\n\
             \n\
             Then set `approval_mode` in [session] to control how network requests\n\
             are approved:\n\
             \n\
             [session]\n\
             approval_mode = \"allow\"   # bypass approval (development only)\n\
             \n\
             Config file location: use --config <path> or the default configs/grokrs.example.toml"
        );
    }

    // With allow_network=true, the engine returns Ask for network effects.
    // Check that the approval_mode won't block us.
    let engine = PolicyEngine::new(config.policy.clone());
    let decision = engine.evaluate(&Effect::NetworkConnect {
        host: "api.x.ai".to_owned(),
    });
    match decision {
        Decision::Allow { .. } => Ok(()),
        Decision::Ask { .. } => {
            match config.session.approval_mode.as_str() {
                "allow" => Ok(()), // bypass
                "deny" => bail!(
                    "Network access requires approval but approval_mode is 'deny'.\n\
                     Set approval_mode = 'allow' in [session] config to bypass."
                ),
                _ => bail!(
                    "Network access requires approval but the approval broker is not yet implemented.\n\
                     Set approval_mode = 'allow' in [session] config to bypass approval,\n\
                     or wait for the approval broker (spec 03)."
                ),
            }
        }
        Decision::Deny { reason } => bail!("Network access denied by policy: {reason}"),
    }
}

/// Build a policy gate for the Grok client.
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
                _ => PolicyDecision::Ask,
            },
            Decision::Deny { reason } => PolicyDecision::Deny {
                reason: reason.to_owned(),
            },
        }
    }))
}

/// Best-effort store opening. Returns None if the store cannot be opened.
fn open_store_best_effort(config: &AppConfig) -> Option<Store> {
    let workspace_root = std::env::current_dir().ok()?;
    let store_path = config
        .store
        .as_ref()
        .map_or(".grokrs/state.db", |s| s.path.as_str());
    Store::open_with_path(&workspace_root, store_path).ok()
}

#[cfg(test)]
mod tests {
    use super::*;
    use grokrs_core::{ModelConfig, PolicyConfig, SessionConfig, WorkspaceConfig};

    fn make_config(allow_network: bool, approval_mode: &str) -> AppConfig {
        AppConfig {
            workspace: WorkspaceConfig {
                name: "test".into(),
                root: ".".into(),
            },
            model: ModelConfig {
                provider: "xai".into(),
                default_model: "grok-4".into(),
            },
            policy: PolicyConfig {
                allow_network,
                allow_shell: false,
                allow_workspace_writes: true,
                max_patch_bytes: 1024,
            },
            session: SessionConfig {
                approval_mode: approval_mode.into(),
                transcript_dir: ".grokrs/sessions".into(),
            },
            api: None,
            management_api: None,
            store: None,
            agent: None,
            chat: None,
            mcp: None,
        }
    }

    #[test]
    fn check_network_denied_returns_error() {
        let config = make_config(false, "allow");
        let result = check_network_policy(&config);
        assert!(result.is_err());
        let msg = result.unwrap_err().to_string();
        assert!(msg.contains("Network access is denied by policy"));
        assert!(msg.contains("allow_network = true"));
    }

    #[test]
    fn check_network_allowed_with_allow_mode_succeeds() {
        let config = make_config(true, "allow");
        let result = check_network_policy(&config);
        assert!(result.is_ok(), "expected Ok, got: {result:?}");
    }

    #[test]
    fn check_network_allowed_with_deny_mode_fails() {
        let config = make_config(true, "deny");
        let result = check_network_policy(&config);
        assert!(result.is_err());
        let msg = result.unwrap_err().to_string();
        assert!(msg.contains("approval_mode is 'deny'"));
    }

    #[test]
    fn check_network_allowed_with_interactive_mode_fails() {
        let config = make_config(true, "interactive");
        let result = check_network_policy(&config);
        assert!(result.is_err());
        let msg = result.unwrap_err().to_string();
        assert!(msg.contains("approval broker is not yet implemented"));
    }

    #[test]
    fn default_chat_args() {
        let args = ChatArgs {
            model: None,
            system: None,
            stateful: false,
            resume: None,
            search: false,
            x_search: false,
            no_search: false,
            search_from_date: None,
            search_to_date: None,
            search_max_results: None,
            citations: false,
            cache_key: None,
        };
        assert!(args.model.is_none());
        assert!(args.system.is_none());
        assert!(!args.stateful);
        assert!(args.resume.is_none());
        assert!(!args.search);
        assert!(!args.x_search);
        assert!(!args.no_search);
        assert!(args.cache_key.is_none());
    }

    #[test]
    fn resolve_resume_no_store_returns_error() {
        let result = resolve_resume_session(&None, "abc");
        assert!(result.is_err());
        let err = result.unwrap_err().to_string();
        assert!(err.contains("requires a store"), "got: {err}");
    }

    #[test]
    fn resolve_resume_nonexistent_session() {
        let tmp = tempfile::tempdir().unwrap();
        let store = Store::open(tmp.path()).unwrap();
        let result = resolve_resume_session(&Some(store), "nonexistent");
        assert!(result.is_err());
        let err = result.unwrap_err().to_string();
        assert!(err.contains("No session found"), "got: {err}");
    }

    #[test]
    fn resolve_resume_exact_match() {
        let tmp = tempfile::tempdir().unwrap();
        let store = Store::open(tmp.path()).unwrap();
        store
            .sessions()
            .create("resume-test-session-1", "Untrusted")
            .unwrap();

        let (id, history, _) =
            resolve_resume_session(&Some(store), "resume-test-session-1").unwrap();
        assert_eq!(id, "resume-test-session-1");
        assert_eq!(history.turn_count(), 0);
    }

    #[test]
    fn resolve_resume_prefix_match() {
        let tmp = tempfile::tempdir().unwrap();
        let store = Store::open(tmp.path()).unwrap();
        store
            .sessions()
            .create("resume-unique-session-abc", "Untrusted")
            .unwrap();

        let (id, _, _) = resolve_resume_session(&Some(store), "resume-unique").unwrap();
        assert_eq!(id, "resume-unique-session-abc");
    }

    #[test]
    fn resolve_resume_ambiguous_prefix() {
        let tmp = tempfile::tempdir().unwrap();
        let store = Store::open(tmp.path()).unwrap();
        store
            .sessions()
            .create("resume-ambig-001", "Untrusted")
            .unwrap();
        store
            .sessions()
            .create("resume-ambig-002", "Untrusted")
            .unwrap();

        let result = resolve_resume_session(&Some(store), "resume-ambig");
        assert!(result.is_err());
        let err = result.unwrap_err().to_string();
        assert!(err.contains("Ambiguous"), "got: {err}");
    }

    #[test]
    fn resolve_resume_reconstructs_conversation() {
        let tmp = tempfile::tempdir().unwrap();
        let store = Store::open(tmp.path()).unwrap();
        store
            .sessions()
            .create("resume-tx-test", "Untrusted")
            .unwrap();

        // Log a request/response pair.
        let tid = store
            .transcripts()
            .log_request(
                "resume-tx-test",
                "/v1/responses",
                "POST",
                Some("{\"message\":\"hello\"}"),
            )
            .unwrap();
        let usage = grokrs_store::types::TranscriptUsage {
            cost_in_usd_ticks: None,
            input_tokens: Some(10),
            output_tokens: Some(20),
            reasoning_tokens: None,
        };
        store
            .transcripts()
            .log_response(tid, 200, Some("Hi there!"), &usage, Some("resp_abc"))
            .unwrap();

        let (id, history, last_resp_id) =
            resolve_resume_session(&Some(store), "resume-tx-test").unwrap();
        assert_eq!(id, "resume-tx-test");
        assert_eq!(history.turn_count(), 1);
        assert_eq!(history.total_input_tokens(), 10);
        assert_eq!(history.total_output_tokens(), 20);
        assert_eq!(last_resp_id.as_deref(), Some("resp_abc"));
        assert_eq!(history.last_response_id(), Some("resp_abc"));

        // Verify reconstructed turn content.
        let turns = history.turns();
        assert_eq!(turns[0].user_input, "hello");
        assert_eq!(turns[0].assistant_response, "Hi there!");
    }

    #[test]
    fn resolve_resume_multiple_turns() {
        let tmp = tempfile::tempdir().unwrap();
        let store = Store::open(tmp.path()).unwrap();
        store
            .sessions()
            .create("resume-multi", "Untrusted")
            .unwrap();

        // First turn.
        let tid1 = store
            .transcripts()
            .log_request(
                "resume-multi",
                "/v1/responses",
                "POST",
                Some("{\"message\":\"first\"}"),
            )
            .unwrap();
        let usage1 = grokrs_store::types::TranscriptUsage {
            cost_in_usd_ticks: None,
            input_tokens: Some(5),
            output_tokens: Some(10),
            reasoning_tokens: None,
        };
        store
            .transcripts()
            .log_response(tid1, 200, Some("response one"), &usage1, Some("resp_1"))
            .unwrap();

        // Second turn.
        let tid2 = store
            .transcripts()
            .log_request(
                "resume-multi",
                "/v1/responses",
                "POST",
                Some("{\"message\":\"second\"}"),
            )
            .unwrap();
        let usage2 = grokrs_store::types::TranscriptUsage {
            cost_in_usd_ticks: None,
            input_tokens: Some(15),
            output_tokens: Some(25),
            reasoning_tokens: None,
        };
        store
            .transcripts()
            .log_response(tid2, 200, Some("response two"), &usage2, Some("resp_2"))
            .unwrap();

        let (_, history, last_resp_id) =
            resolve_resume_session(&Some(store), "resume-multi").unwrap();
        assert_eq!(history.turn_count(), 2);
        assert_eq!(history.total_input_tokens(), 20); // 5 + 15
        assert_eq!(history.total_output_tokens(), 35); // 10 + 25
        assert_eq!(last_resp_id.as_deref(), Some("resp_2"));
    }

    // -----------------------------------------------------------------------
    // resolve_search_config tests
    // -----------------------------------------------------------------------

    fn search_args(
        search: bool,
        x_search: bool,
        no_search: bool,
        from_date: Option<&str>,
        to_date: Option<&str>,
        max_results: Option<u32>,
        citations: bool,
    ) -> ChatArgs {
        ChatArgs {
            model: None,
            system: None,
            stateful: false,
            resume: None,
            search,
            x_search,
            no_search,
            search_from_date: from_date.map(ToOwned::to_owned),
            search_to_date: to_date.map(ToOwned::to_owned),
            search_max_results: max_results,
            citations,
            cache_key: None,
        }
    }

    #[test]
    fn resolve_search_config_no_flags() {
        let args = search_args(false, false, false, None, None, None, false);
        let config = resolve_search_config(&args).unwrap();
        assert!(config.is_empty());
    }

    #[test]
    fn resolve_search_config_web_search() {
        let args = search_args(true, false, false, None, None, None, false);
        let config = resolve_search_config(&args).unwrap();
        assert!(config.web_search);
        assert!(!config.x_search);
    }

    #[test]
    fn resolve_search_config_x_search() {
        let args = search_args(false, true, false, None, None, None, false);
        let config = resolve_search_config(&args).unwrap();
        assert!(!config.web_search);
        assert!(config.x_search);
    }

    #[test]
    fn resolve_search_config_both_search() {
        let args = search_args(true, true, false, None, None, None, false);
        let config = resolve_search_config(&args).unwrap();
        assert!(config.web_search);
        assert!(config.x_search);
    }

    #[test]
    fn resolve_search_config_no_search_overrides() {
        let args = search_args(true, true, true, None, None, None, false);
        let config = resolve_search_config(&args).unwrap();
        assert!(config.is_empty());
    }

    #[test]
    fn resolve_search_config_with_date_range() {
        let args = search_args(
            true,
            false,
            false,
            Some("2025-01-01"),
            Some("2025-12-31"),
            None,
            false,
        );
        let config = resolve_search_config(&args).unwrap();
        assert_eq!(config.from_date.as_deref(), Some("2025-01-01"));
        assert_eq!(config.to_date.as_deref(), Some("2025-12-31"));
    }

    #[test]
    fn resolve_search_config_invalid_from_date() {
        let args = search_args(true, false, false, Some("not-a-date"), None, None, false);
        let result = resolve_search_config(&args);
        assert!(result.is_err());
        let err = result.unwrap_err().to_string();
        assert!(err.contains("search-from-date"), "got: {err}");
    }

    #[test]
    fn resolve_search_config_invalid_to_date() {
        let args = search_args(true, false, false, None, Some("2025-13-01"), None, false);
        let result = resolve_search_config(&args);
        assert!(result.is_err());
        let err = result.unwrap_err().to_string();
        assert!(err.contains("search-to-date"), "got: {err}");
    }

    #[test]
    fn resolve_search_config_with_max_results_and_citations() {
        let args = search_args(true, false, false, None, None, Some(5), true);
        let config = resolve_search_config(&args).unwrap();
        assert_eq!(config.max_results, Some(5));
        assert!(config.citations);
    }
}
