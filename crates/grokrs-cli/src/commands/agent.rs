//! CLI subcommand for running an agentic task with tool execution.
//!
//! `grokrs agent '<task>'` runs a single agentic task using the Responses API
//! function-calling loop. The agent constructs a `ToolRegistry` filtered by the
//! session's trust level, sends the task as the initial prompt with tool
//! definitions, and iterates until the model produces a text response or the
//! maximum iterations are reached.
//!
//! ## Headless / CI mode
//!
//! When `--headless` is passed the command operates without any TTY interaction:
//!
//! - No colored output, progress spinners, or approval prompts.
//! - Approval mode is forced to `"deny"` unless explicitly overridden via
//!   `--approval-mode`.
//! - `--output json` emits newline-delimited JSON events to **stdout** while
//!   diagnostic messages go to **stderr**.
//! - Process exit codes are stable API (see [`AgentExitCode`]):
//!   0 = success, 1 = agent error, 2 = policy denial, 3 = API error, 4 = timeout.
//! - `--timeout` defaults to 300 s in headless mode (no timeout in interactive).
//! - Task description can be piped via stdin when `--headless` is active.

use std::env;
use std::fmt::Write as _;
use std::io::{self, Read as _};
use std::sync::Arc;

use anyhow::{Context, Result, bail};
use clap::{Parser, ValueEnum};
use serde::Serialize;

use grokrs_api::client::GrokClient;
use grokrs_api::mcp;
use grokrs_api::tool_loop::{FunctionExecutor, ToolLoopConfig, ToolLoopError};
use grokrs_api::transport::policy_bridge::FnPolicyGate;
use grokrs_api::transport::policy_gate::{PolicyDecision, PolicyGate};
use grokrs_api::types::builtin_tools::BuiltinTool;
use grokrs_api::types::common::ContentBlock;
use grokrs_api::types::responses::{CreateResponseBuilder, OutputItem, ResponseInput};
use grokrs_cap::WorkspaceRoot;
use grokrs_core::AppConfig;
use grokrs_policy::{Decision, Effect, PolicyEngine};
use grokrs_store::Store;
use grokrs_tool::registry::default_registry;

use super::search::{self, SearchConfig};
use crate::agent::{McpToolAdapter, PolicyGatedExecutor};

// ---------------------------------------------------------------------------
// Exit codes (stable CLI API — do not renumber)
// ---------------------------------------------------------------------------

/// Process exit codes for the agent command.
///
/// These codes are part of the stable CLI API. Scripts and CI pipelines depend
/// on them, so they must not be changed without a major version bump.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum AgentExitCode {
    /// Task completed successfully.
    Success = 0,
    /// Agent-level error (e.g., max iterations exceeded, configuration error).
    AgentError = 1,
    /// A tool was blocked by policy (effect denied).
    PolicyDenial = 2,
    /// API / transport error (network, auth, rate limit).
    ApiError = 3,
    /// Execution exceeded the `--timeout` deadline.
    Timeout = 4,
}

impl AgentExitCode {
    /// Convert to the numeric code suitable for `std::process::exit`.
    #[must_use]
    pub fn code(self) -> i32 {
        self as i32
    }
}

/// Map a [`ToolLoopError`] to the appropriate [`AgentExitCode`].
#[must_use]
pub fn exit_code_for_tool_loop_error(err: &ToolLoopError) -> AgentExitCode {
    match err {
        ToolLoopError::MaxIterationsExceeded { .. }
        | ToolLoopError::InvalidConfiguration { .. } => AgentExitCode::AgentError,
        ToolLoopError::ExecutionFailed { error, .. } => {
            let msg = error.to_string();
            if msg.contains("policy denied") {
                AgentExitCode::PolicyDenial
            } else {
                AgentExitCode::AgentError
            }
        }
        ToolLoopError::Transport(_) => AgentExitCode::ApiError,
    }
}

/// Map a generic `anyhow::Error` (from pre-loop failures) to an exit code.
///
/// Public alias used by `main.rs` to map errors that escape `run()`.
#[must_use]
pub fn exit_code_for_anyhow_public(err: &anyhow::Error) -> AgentExitCode {
    exit_code_for_anyhow(err)
}

/// Map a generic `anyhow::Error` (from pre-loop failures) to an exit code.
fn exit_code_for_anyhow(err: &anyhow::Error) -> AgentExitCode {
    let msg = format!("{err:#}");
    if msg.contains("Network access is denied") || msg.contains("policy denied") {
        AgentExitCode::PolicyDenial
    } else if msg.contains("failed to construct API client") || msg.contains("transport") {
        AgentExitCode::ApiError
    } else {
        AgentExitCode::AgentError
    }
}

// ---------------------------------------------------------------------------
// Output format
// ---------------------------------------------------------------------------

/// Output format for agent results.
#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub enum OutputFormat {
    /// Human-readable text (default).
    Text,
    /// Newline-delimited JSON events on stdout.
    Json,
}

// ---------------------------------------------------------------------------
// Headless JSON events
// ---------------------------------------------------------------------------

/// A single newline-delimited JSON event emitted in `--output json` mode.
///
/// Each event is serialized as a single JSON object followed by a newline
/// character, suitable for consumption by `jq`, log aggregators, or other
/// structured-logging pipelines.
#[derive(Debug, Clone, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum HeadlessEvent {
    /// A tool was called by the model.
    ToolCall { name: String, arguments: String },
    /// A tool returned a result.
    ToolResult { name: String, output: String },
    /// The model produced a text message.
    Message { content: String },
    /// An error occurred.
    Error { message: String, exit_code: u8 },
    /// Token usage summary.
    Usage {
        input_tokens: u64,
        output_tokens: u64,
        total_tokens: u64,
    },
}

/// Run a single agentic task with tool execution.
#[derive(Debug, Parser)]
#[command(after_help = "\
Examples:
  grokrs agent 'explain src/main.rs'                  Read-only task (untrusted)
  grokrs agent 'refactor error handling' --trust interactive  Allow writes
  grokrs agent 'run tests and fix failures' --trust admin     Allow shell
  grokrs agent 'find info about Rust 2024' --search   Enable web search
  grokrs agent 'summarize changes' --dry-run           Preview tool calls
  grokrs agent 'optimize code' --max-iterations 5      Limit iterations
  echo 'task' | grokrs agent --headless                CI/CD pipeline mode
  grokrs agent --headless --output json 'task'         JSON event stream

See also: grokrs chat")]
pub struct AgentArgs {
    /// Task description for the agent.
    /// In --headless mode, if omitted, reads from stdin.
    pub task: Option<String>,

    /// Trust level for the session (untrusted, interactive, admin).
    /// When omitted, falls back to [agent].default_trust config, then "untrusted".
    #[arg(long)]
    pub trust: Option<String>,

    /// Maximum number of tool-calling iterations before aborting.
    /// When omitted, falls back to [agent].max_iterations config, then 10.
    #[arg(long)]
    pub max_iterations: Option<u32>,

    /// Model to use (overrides config default).
    #[arg(long)]
    pub model: Option<String>,

    /// System prompt to prepend to the task.
    #[arg(long)]
    pub system: Option<String>,

    /// Show what tools would be called without executing (dry-run mode).
    #[arg(long)]
    pub dry_run: bool,

    /// Enable web search (adds `web_search` built-in tool alongside function tools).
    #[arg(long)]
    pub search: bool,

    /// Enable X (Twitter) search (adds `x_search` built-in tool alongside function tools).
    #[arg(long)]
    pub x_search: bool,

    /// Include citation URLs in the response when search is enabled.
    #[arg(long)]
    pub citations: bool,

    /// Run in headless mode (no TTY interaction, no color, no spinners).
    /// Suitable for CI/CD pipelines, GitHub Actions, cron jobs.
    /// Sets approval_mode to 'deny' unless --approval-mode is specified.
    #[arg(long)]
    pub headless: bool,

    /// Output format: 'text' (default) or 'json' (newline-delimited JSON events).
    /// JSON output goes to stdout; diagnostics go to stderr.
    #[arg(long, value_enum, default_value_t = OutputFormat::Text)]
    pub output: OutputFormat,

    /// Maximum execution time in seconds. In headless mode, defaults to 300s.
    /// In interactive mode, defaults to no timeout.
    #[arg(long)]
    pub timeout: Option<u64>,

    /// Override the approval mode for this run (allow, deny, interactive).
    /// In headless mode, defaults to 'deny' unless this flag is set.
    #[arg(long)]
    pub approval_mode: Option<String>,

    /// Connect to an ad-hoc MCP server for this session.
    /// Discovers tools from the server and registers them alongside built-in tools.
    /// Can be specified multiple times for multiple servers.
    /// Example: --mcp-server http://localhost:8080/mcp
    #[arg(long = "mcp-server", value_name = "URL")]
    pub mcp_servers: Vec<String>,

    /// Enable server-side prompt caching by sending this key as
    /// `prompt_cache_key` in the initial Responses API request.
    ///
    /// Use a stable key that identifies the fixed portion of your prompt
    /// (e.g. the system instructions or tool definitions). The server caches
    /// the KV state of the matched prefix and reuses it on subsequent requests
    /// that send the same key, reducing latency and effective input-token cost.
    ///
    /// When a cache hit occurs, the usage summary shows cached tokens:
    /// `input=500 (200 cached), output=100`.
    ///
    /// Omit this flag when the prompt changes every invocation or when caching
    /// is not desired.
    #[arg(long, value_name = "KEY")]
    pub cache_key: Option<String>,
}

/// Parse a trust level string into a trust rank (u8).
///
/// Returns an error if the string is not one of the recognized levels.
fn parse_trust_rank(trust: &str) -> Result<u8> {
    match trust {
        "untrusted" => Ok(0),
        "interactive" => Ok(1),
        "admin" => Ok(2),
        other => bail!(
            "unknown trust level: '{other}'\n\
             Valid values: untrusted, interactive, admin"
        ),
    }
}

/// Build a policy gate from config (same pattern as api.rs).
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

/// Check network access policy (same pattern as api.rs).
fn check_network_allowed(config: &AppConfig) -> Result<()> {
    if !config.policy.allow_network {
        bail!(
            "Network access is denied by policy.\n\
             \n\
             The agent command requires network access to communicate with the xAI API.\n\
             To enable, set `allow_network = true` in your config file:\n\
             \n\
             [policy]\n\
             allow_network = true\n\
             \n\
             Config file location: use --config <path> or the default configs/grokrs.example.toml"
        );
    }
    Ok(())
}

/// Best-effort store opening.
fn open_store_best_effort(config: &AppConfig) -> Option<Store> {
    let workspace_root = env::current_dir().ok()?;
    let store_path = config
        .store
        .as_ref()
        .map_or(".grokrs/state.db", |s| s.path.as_str());
    Store::open_with_path(&workspace_root, store_path).ok()
}

/// Build the system prompt, including cross-session memories if available.
///
/// The prompt is constructed by:
/// 1. Starting with the user-provided system prompt (if any).
/// 2. Appending the top-N most relevant memories from the store (by access
///    count and recency), formatted as a memory context section.
///
/// Returns `None` if there is no user system prompt and no memories.
fn build_system_prompt(
    user_system: Option<&str>,
    store: &Option<Store>,
    memory_limit: i64,
) -> Option<String> {
    // Retrieve top memories from the store (best-effort).
    let memories_section = store.as_ref().and_then(|s| {
        // Evict over-limit memories before reading.
        let mem = s.memories();
        let _ = mem.evict(memory_limit);

        // Retrieve top-N for inclusion in the prompt (cap at 20 for prompt size).
        let top_n = std::cmp::min(memory_limit, 20);
        let memories = mem.top_n(top_n).ok()?;
        if memories.is_empty() {
            return None;
        }

        let count = memories.len();
        let mut section = format!(
            "\n\n## Cross-Session Memory ({count} memor{})\n\
                 The following memories were saved from previous sessions:\n",
            if count == 1 { "y" } else { "ies" }
        );
        for m in &memories {
            write!(section, "\n- [{}] {}: {}", m.category, m.key, m.value).expect("String write is infallible");
        }
        section.push_str(
            "\n\nYou can use the `remember`, `recall`, and `forget` tools to manage memories.",
        );

        Some(section)
    });

    match (user_system, memories_section) {
        (Some(prompt), Some(mem)) => Some(format!("{prompt}{mem}")),
        (Some(prompt), None) => Some(prompt.to_string()),
        (None, Some(mem)) => Some(mem),
        (None, None) => None,
    }
}

/// A wrapper around `PolicyGatedExecutor` that logs tool calls to stderr
/// and supports dry-run mode and headless JSON event output.
struct LoggingExecutor {
    inner: PolicyGatedExecutor,
    dry_run: bool,
    output_format: OutputFormat,
}

impl LoggingExecutor {
    /// Emit a [`HeadlessEvent`] as a JSON line to stdout.
    fn emit_event(&self, event: &HeadlessEvent) {
        if self.output_format == OutputFormat::Json
            && let Ok(json) = serde_json::to_string(event)
        {
            println!("{json}");
        }
    }
}

impl FunctionExecutor for LoggingExecutor {
    fn execute(
        &self,
        name: &str,
        arguments: &str,
    ) -> Result<String, Box<dyn std::error::Error + Send + Sync>> {
        // Emit tool_call event in JSON mode.
        self.emit_event(&HeadlessEvent::ToolCall {
            name: name.to_owned(),
            arguments: arguments.to_owned(),
        });

        // Display tool call summary on stderr (text mode) or as diagnostic (JSON mode).
        let summary = summarize_tool_call(name, arguments);
        eprintln!("[tool:{name}] {summary}");

        if self.dry_run {
            let dry_result = format!(
                "{{\"dry_run\": true, \"tool\": \"{name}\", \"message\": \"dry-run: tool not executed\"}}"
            );
            self.emit_event(&HeadlessEvent::ToolResult {
                name: name.to_owned(),
                output: dry_result.clone(),
            });
            return Ok(dry_result);
        }

        let result = self.inner.execute(name, arguments);

        match &result {
            Ok(output) => {
                self.emit_event(&HeadlessEvent::ToolResult {
                    name: name.to_owned(),
                    output: output.clone(),
                });
            }
            Err(err) => {
                self.emit_event(&HeadlessEvent::Error {
                    message: err.to_string(),
                    exit_code: if err.to_string().contains("policy denied") {
                        AgentExitCode::PolicyDenial as u8
                    } else {
                        AgentExitCode::AgentError as u8
                    },
                });
            }
        }

        result
    }
}

/// UTF-8 safe truncation: find the nearest char boundary at or before `max_bytes`.
fn truncate_utf8(s: &str, max_bytes: usize) -> &str {
    if s.len() <= max_bytes {
        return s;
    }
    let mut i = max_bytes;
    while i > 0 && !s.is_char_boundary(i) {
        i -= 1;
    }
    &s[..i]
}

/// Summarize a tool call for display.
fn summarize_tool_call(_name: &str, arguments: &str) -> String {
    // Try to extract the "path" field for filesystem tools.
    if let Ok(val) = serde_json::from_str::<serde_json::Value>(arguments) {
        if let Some(path) = val.get("path").and_then(|v| v.as_str()) {
            return path.to_string();
        }
        if let Some(cmd) = val.get("command").and_then(|v| v.as_str()) {
            // Truncate long commands (UTF-8 safe).
            if cmd.len() > 80 {
                return format!("{}...", truncate_utf8(cmd, 77));
            }
            return cmd.to_string();
        }
    }
    // Fallback: truncated arguments (UTF-8 safe).
    if arguments.len() > 80 {
        format!("{}...", truncate_utf8(arguments, 77))
    } else {
        arguments.to_string()
    }
}

/// Extract text content from the final response.
fn extract_text_output(output: &[OutputItem]) -> String {
    let mut text = String::new();
    for item in output {
        if let OutputItem::Message { content, .. } = item {
            for block in content {
                match block {
                    ContentBlock::Text { text: t } | ContentBlock::InputText { text: t } => {
                        if !text.is_empty() {
                            text.push('\n');
                        }
                        text.push_str(t);
                    }
                    _ => {}
                }
            }
        }
    }
    text
}

/// Resolve the task description from CLI args or stdin (headless mode).
///
/// In headless mode, if no positional `task` argument is provided, the task
/// is read from stdin (suitable for piping). In interactive mode the task
/// argument is required.
fn resolve_task(args: &AgentArgs) -> Result<String> {
    if let Some(ref task) = args.task {
        return Ok(task.clone());
    }

    if args.headless {
        let mut buf = String::new();
        io::stdin()
            .read_to_string(&mut buf)
            .context("failed to read task from stdin")?;
        let task = buf.trim().to_string();
        if task.is_empty() {
            bail!("no task provided: pass a task argument or pipe via stdin in headless mode");
        }
        Ok(task)
    } else {
        bail!(
            "task argument is required in interactive mode.\n\
             Usage: grokrs agent '<task description>'\n\
             In headless mode you can pipe via stdin: echo 'task' | grokrs agent --headless"
        );
    }
}

/// Result of [`run`] that carries the exit code for headless mode.
///
/// Callers (main.rs) should inspect this to decide the process exit code.
pub struct AgentResult {
    /// The exit code to return to the OS.
    pub exit_code: AgentExitCode,
}

/// Emit a headless JSON event to stdout.
fn emit_event(output_format: OutputFormat, event: &HeadlessEvent) {
    if output_format == OutputFormat::Json
        && let Ok(json) = serde_json::to_string(event)
    {
        println!("{json}");
    }
}

/// Execute the `grokrs agent` command.
///
/// Returns an [`AgentResult`] with the exit code. In interactive mode the
/// exit code is always `Success` on `Ok(...)` (errors propagate via `?`).
/// In headless mode the exit code distinguishes error categories.
pub async fn run(args: &AgentArgs, config: &AppConfig) -> Result<AgentResult> {
    // Resolve task (positional arg or stdin in headless mode).
    let task = match resolve_task(args) {
        Ok(t) => t,
        Err(e) => {
            emit_event(
                args.output,
                &HeadlessEvent::Error {
                    message: format!("{e:#}"),
                    exit_code: AgentExitCode::AgentError as u8,
                },
            );
            return Err(e);
        }
    };

    // Resolve approval mode: headless defaults to "deny" unless overridden.
    let approval_mode = if let Some(ref mode) = args.approval_mode {
        mode.clone()
    } else if args.headless {
        "deny".to_owned()
    } else {
        config.session.approval_mode.clone()
    };

    // Resolve timeout: headless defaults to 300s, interactive has none.
    let timeout_secs: Option<u64> = args
        .timeout
        .or(if args.headless { Some(300) } else { None });

    if let Err(e) = check_network_allowed(config) {
        emit_event(
            args.output,
            &HeadlessEvent::Error {
                message: format!("{e:#}"),
                exit_code: AgentExitCode::PolicyDenial as u8,
            },
        );
        if args.headless {
            return Ok(AgentResult {
                exit_code: AgentExitCode::PolicyDenial,
            });
        }
        return Err(e);
    }

    // CLI args override [agent] config defaults. Option-based: None = not provided.
    let agent_config = config.agent.as_ref();
    let trust_str = args
        .trust
        .as_deref()
        .or_else(|| agent_config.map(|c| c.default_trust.as_str()))
        .unwrap_or("untrusted");
    let trust_rank = parse_trust_rank(trust_str)?;
    let model = args.model.as_deref().unwrap_or(&config.model.default_model);

    // Build tool registry and filter by trust rank.
    let mut registry = default_registry();

    // Connect to MCP servers (config-based + CLI ad-hoc).
    let mcp_clients = connect_mcp_servers(args, config, &approval_mode, &mut registry).await;
    if !mcp_clients.is_empty() {
        let mcp_tool_names: Vec<&str> = registry
            .available_tools(trust_rank)
            .iter()
            .filter(|t| t.name().starts_with("mcp_"))
            .map(|t| t.name())
            .collect();
        if !mcp_tool_names.is_empty() {
            eprintln!("[agent] MCP tools: {}", mcp_tool_names.join(", "));
        }
    }

    let tool_defs = registry.tool_definitions(trust_rank);

    if tool_defs.is_empty() {
        let msg = format!("no tools available at trust level '{trust_str}'");
        emit_event(
            args.output,
            &HeadlessEvent::Error {
                message: msg.clone(),
                exit_code: AgentExitCode::AgentError as u8,
            },
        );
        bail!("{msg}");
    }

    let max_iterations = args
        .max_iterations
        .or_else(|| agent_config.map(|c| c.max_iterations))
        .unwrap_or(10);

    eprintln!(
        "[agent] trust={} model={} max_iterations={} tools={} headless={} approval_mode={}",
        trust_str,
        model,
        max_iterations,
        tool_defs.len(),
        args.headless,
        approval_mode,
    );

    if args.dry_run {
        eprintln!("[agent] dry-run mode: tools will not be executed");
    }

    if let Some(t) = timeout_secs {
        eprintln!("[agent] timeout: {t}s");
    }

    // Show available tools.
    let tool_names: Vec<&str> = registry
        .available_tools(trust_rank)
        .iter()
        .map(|t| t.name())
        .collect();
    eprintln!("[agent] available tools: {}", tool_names.join(", "));

    // Build API client.
    let engine = PolicyEngine::new(config.policy.clone());
    let gate = build_policy_gate(engine, &approval_mode);
    let client =
        GrokClient::from_config(config, Some(gate)).context("failed to construct API client")?;

    // Build workspace root.
    let workspace_root =
        WorkspaceRoot::new(&env::current_dir().context("failed to resolve current directory")?)
            .context("failed to construct workspace root")?;

    // Build a second registry for the executor (includes MCP tools).
    let mut executor_registry = default_registry();
    // Re-connect MCP tools for the executor's registry (shared clients).
    register_mcp_tools_from_clients(&mcp_clients, &mut executor_registry);

    // Build the executor using the existing PolicyGatedExecutor.
    let inner_executor = PolicyGatedExecutor::new(
        executor_registry,
        PolicyEngine::new(config.policy.clone()),
        workspace_root,
        approval_mode.clone(),
        trust_rank,
    );

    let executor = LoggingExecutor {
        inner: inner_executor,
        dry_run: args.dry_run,
        output_format: args.output,
    };

    // Resolve search config: CLI flags override [agent].enable_search.
    let enable_search_from_config = agent_config.is_some_and(|c| c.enable_search);
    let search_config = SearchConfig {
        web_search: args.search || enable_search_from_config,
        x_search: args.x_search,
        citations: args.citations,
        ..Default::default()
    };

    // Merge function tool definitions with built-in search tools.
    let mut all_tools = tool_defs;
    if !search_config.is_empty() {
        all_tools.extend(search_config.tool_values());
        let search_tool_names: Vec<&str> = search_config
            .builtin_tools()
            .iter()
            .map(BuiltinTool::type_name)
            .collect();
        eprintln!("[agent] search tools: {}", search_tool_names.join(", "));
    }

    // Store integration (best-effort) — opened early so memories can be
    // included in the system prompt.
    let store = open_store_best_effort(config);

    // Build the system prompt, including cross-session memories if available.
    let memory_limit = agent_config.map_or(50, |c| c.memory_limit);
    let system_prompt = build_system_prompt(args.system.as_deref(), &store, memory_limit);

    // Build the initial request.
    let mut builder = CreateResponseBuilder::new(model, ResponseInput::Text(task))
        .store(false)
        .tools(all_tools);

    if let Some(ref instructions) = system_prompt {
        builder = builder.instructions(instructions.clone());
    }

    // Search parameters (citations).
    if let Some(params) = search_config.search_parameters() {
        builder = builder.search_parameters(params.to_value());
    }

    // Prompt cache key: enables server-side prompt caching for the initial
    // request (system prompt + tool definitions). Subsequent tool-loop turns
    // reuse the same cached KV state automatically because the server matches
    // on the key.
    if let Some(ref key) = args.cache_key {
        builder = builder.prompt_cache_key(key.clone());
        eprintln!("[agent] prompt_cache_key={key:?}");
    }

    let request = builder.build();
    let session_id = uuid::Uuid::new_v4().to_string();

    if let Some(ref s) = store
        && s.sessions().create(&session_id, "Untrusted").is_ok()
    {
        let _ = s.sessions().transition(&session_id, "Ready");
        let _ = s.sessions().transition(&session_id, "RunningTurn");
    }

    // Run the tool loop, optionally wrapped in a timeout.
    let loop_config = ToolLoopConfig { max_iterations };

    let responses_client = client.responses();
    let tool_loop_future =
        grokrs_api::tool_loop::run_tool_loop(&responses_client, request, &executor, loop_config);

    let result = if let Some(secs) = timeout_secs {
        match tokio::time::timeout(std::time::Duration::from_secs(secs), tool_loop_future).await {
            Ok(inner) => inner,
            Err(_elapsed) => {
                let msg = format!("agent timed out after {secs}s");
                eprintln!("[agent] {msg}");
                emit_event(
                    args.output,
                    &HeadlessEvent::Error {
                        message: msg.clone(),
                        exit_code: AgentExitCode::Timeout as u8,
                    },
                );

                // Store: transition to Failed.
                if let Some(ref s) = store {
                    let _ = s
                        .sessions()
                        .transition(&session_id, &format!("Failed: {msg}"));
                }
                if let Some(s) = store {
                    let _ = s.close();
                }

                if args.headless {
                    return Ok(AgentResult {
                        exit_code: AgentExitCode::Timeout,
                    });
                }
                bail!("{msg}");
            }
        }
    } else {
        tool_loop_future.await
    };

    let output_format = args.output;

    match result {
        Ok(response) => {
            // Extract and print text output.
            let text = extract_text_output(&response.output);

            if output_format == OutputFormat::Json {
                // Emit message event.
                if !text.is_empty() {
                    emit_event(
                        output_format,
                        &HeadlessEvent::Message {
                            content: text.clone(),
                        },
                    );
                }
            } else if !text.is_empty() {
                println!("{text}");
            }

            // Extract and display citations from search results.
            let citations = search::extract_citations_from_output(&response.output);
            if !citations.is_empty() && output_format == OutputFormat::Text {
                eprint!("{}", search::format_citations(&citations));
            }

            // Print usage summary.
            if let Some(usage) = &response.usage {
                let total = usage
                    .total_tokens
                    .unwrap_or(usage.input_tokens + usage.output_tokens);

                emit_event(
                    output_format,
                    &HeadlessEvent::Usage {
                        input_tokens: usage.input_tokens,
                        output_tokens: usage.output_tokens,
                        total_tokens: total,
                    },
                );

                // Extract cached_tokens from prompt_tokens_details or
                // input_tokens_details, whichever the server returned.
                let cached_tokens = usage
                    .prompt_tokens_details
                    .as_ref()
                    .and_then(|d| d.cached_tokens);

                match cached_tokens {
                    Some(cached) if cached > 0 => {
                        eprintln!(
                            "[usage] input={} ({cached} cached) output={} total={}",
                            usage.input_tokens, usage.output_tokens, total,
                        );
                    }
                    _ => {
                        eprintln!(
                            "[usage] input={} output={} total={}",
                            usage.input_tokens, usage.output_tokens, total,
                        );
                    }
                }
            }

            // Store: transition to Closed.
            if let Some(ref s) = store {
                let _ = s.sessions().transition(&session_id, "Ready");
                let _ = s.sessions().transition(&session_id, "Closed");
            }

            // Close store (best-effort).
            if let Some(s) = store {
                let _ = s.close();
            }

            Ok(AgentResult {
                exit_code: AgentExitCode::Success,
            })
        }
        Err(e) => {
            let exit_code = exit_code_for_tool_loop_error(&e);

            emit_event(
                output_format,
                &HeadlessEvent::Error {
                    message: e.to_string(),
                    exit_code: exit_code as u8,
                },
            );

            // Store: transition to Failed.
            if let Some(ref s) = store {
                let _ = s
                    .sessions()
                    .transition(&session_id, &format!("Failed: {e}"));
            }
            if let Some(s) = store {
                let _ = s.close();
            }

            if args.headless {
                eprintln!("[agent] failed: {e}");
                return Ok(AgentResult { exit_code });
            }

            bail!("agent failed: {e}");
        }
    }
}

// ---------------------------------------------------------------------------
// MCP server connection and tool registration
// ---------------------------------------------------------------------------

/// Metadata about a connected MCP server and its discovered tools.
struct ConnectedMcpServer {
    client: Arc<tokio::sync::RwLock<mcp::McpClient>>,
    server_host: String,
    label: Option<String>,
    /// Discovered tool definitions from this server (already filtered by allowlist).
    tools: Vec<mcp::McpToolDefinition>,
    /// Trust rank for tools from this server.
    trust_rank: u8,
}

/// Connect to MCP servers from config + CLI flags, discover tools, and register
/// them in the provided tool registry.
///
/// Returns the list of connected servers (for re-registering tools in the
/// executor's registry).
async fn connect_mcp_servers(
    args: &AgentArgs,
    config: &AppConfig,
    approval_mode: &str,
    registry: &mut grokrs_tool::registry::ToolRegistry,
) -> Vec<ConnectedMcpServer> {
    let mut connected = Vec::new();
    let policy_engine = PolicyEngine::new(config.policy.clone());

    // Config-based servers.
    if let Some(ref mcp_config) = config.mcp {
        for (key, server_config) in &mcp_config.servers {
            let label = server_config.label.as_deref().unwrap_or(key);
            match connect_single_mcp_server(
                &server_config.url,
                Some(label),
                server_config.trust_rank,
                server_config.allowed_tools.clone(),
                server_config.timeout_secs,
                &policy_engine,
                approval_mode,
            )
            .await
            {
                Ok(server) => {
                    register_mcp_tools_for_server(&server, registry);
                    eprintln!(
                        "[mcp] connected to '{}' ({}) — {} tools",
                        label,
                        server_config.url,
                        server.tools.len()
                    );
                    connected.push(server);
                }
                Err(e) => {
                    eprintln!(
                        "[mcp] failed to connect to '{}' ({}): {e}",
                        label, server_config.url
                    );
                }
            }
        }
    }

    // CLI ad-hoc servers (--mcp-server).
    for url in &args.mcp_servers {
        match connect_single_mcp_server(url, None, 1, None, 30, &policy_engine, approval_mode).await
        {
            Ok(server) => {
                let label_display = server.label.as_deref().unwrap_or(&server.server_host);
                eprintln!(
                    "[mcp] connected to ad-hoc server '{}' — {} tools",
                    label_display,
                    server.tools.len()
                );
                register_mcp_tools_for_server(&server, registry);
                connected.push(server);
            }
            Err(e) => {
                eprintln!("[mcp] failed to connect to ad-hoc server '{url}': {e}");
            }
        }
    }

    connected
}

/// Connect to a single MCP server, perform the initialize handshake, and
/// discover tools.
async fn connect_single_mcp_server(
    url: &str,
    label: Option<&str>,
    trust_rank: u8,
    allowed_tools: Option<Vec<String>>,
    timeout_secs: u64,
    policy_engine: &PolicyEngine,
    approval_mode: &str,
) -> Result<ConnectedMcpServer> {
    // Check network policy for the MCP server host.
    let parsed_url =
        url::Url::parse(url).with_context(|| format!("invalid MCP server URL: {url}"))?;
    let host = parsed_url.host_str().unwrap_or("unknown").to_owned();

    let effect = Effect::NetworkConnect { host: host.clone() };
    let decision = policy_engine.evaluate(&effect);
    match decision {
        Decision::Allow { .. } => { /* proceed */ }
        Decision::Ask { reason } => match approval_mode {
            "allow" => { /* proceed */ }
            "deny" => bail!("network access to MCP server '{host}' denied: {reason}"),
            _ => bail!("network access to MCP server '{host}' requires approval: {reason}"),
        },
        Decision::Deny { reason } => {
            bail!("network access to MCP server '{host}' denied by policy: {reason}");
        }
    }

    let transport_config = mcp::McpTransportConfig::new(url)
        .with_timeout(std::time::Duration::from_secs(timeout_secs));

    let mut client = mcp::McpClient::with_config(transport_config)
        .with_context(|| format!("failed to create MCP client for '{url}'"))?;

    if let Some(l) = label {
        client = client.with_label(l);
    }

    // Perform the initialize handshake.
    let init_result = client
        .connect()
        .await
        .with_context(|| format!("MCP initialize handshake failed for '{url}'"))?;

    let server_name = init_result.server_info.name.clone();
    let effective_label = label
        .map(ToOwned::to_owned)
        .or_else(|| Some(server_name.clone()));

    // Discover tools.
    let all_tools = client
        .list_tools()
        .await
        .with_context(|| format!("MCP tools/list failed for '{url}'"))?;

    // Filter by allowlist if specified.
    let tools = match &allowed_tools {
        Some(allowlist) => all_tools
            .into_iter()
            .filter(|t| allowlist.contains(&t.name))
            .collect(),
        None => all_tools,
    };

    let client_arc = Arc::new(tokio::sync::RwLock::new(client));

    Ok(ConnectedMcpServer {
        client: client_arc,
        server_host: host,
        label: effective_label,
        tools,
        trust_rank,
    })
}

/// Register MCP tools from a connected server into a tool registry.
fn register_mcp_tools_for_server(
    server: &ConnectedMcpServer,
    registry: &mut grokrs_tool::registry::ToolRegistry,
) {
    for tool_def in &server.tools {
        let adapter = McpToolAdapter::new(
            tool_def.clone(),
            server.client.clone(),
            server.server_host.clone(),
            server.label.as_deref(),
            server.trust_rank,
        );
        registry.register(Box::new(adapter));
    }
}

/// Re-register MCP tools from already-connected servers into a fresh registry.
///
/// Used to populate the executor's registry with the same MCP tools (sharing
/// the same `McpClient` connections).
fn register_mcp_tools_from_clients(
    servers: &[ConnectedMcpServer],
    registry: &mut grokrs_tool::registry::ToolRegistry,
) {
    for server in servers {
        register_mcp_tools_for_server(server, registry);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // --- parse_trust_rank tests ---

    #[test]
    fn parse_trust_rank_untrusted() {
        assert_eq!(parse_trust_rank("untrusted").unwrap(), 0);
    }

    #[test]
    fn parse_trust_rank_interactive() {
        assert_eq!(parse_trust_rank("interactive").unwrap(), 1);
    }

    #[test]
    fn parse_trust_rank_admin() {
        assert_eq!(parse_trust_rank("admin").unwrap(), 2);
    }

    #[test]
    fn parse_trust_rank_unknown_is_error() {
        let err = parse_trust_rank("superuser").unwrap_err();
        let msg = format!("{err}");
        assert!(
            msg.contains("superuser"),
            "error should mention the value: {msg}"
        );
    }

    // --- summarize_tool_call tests ---

    #[test]
    fn summarize_read_file() {
        let summary = summarize_tool_call("read_file", r#"{"path": "src/main.rs"}"#);
        assert_eq!(summary, "src/main.rs");
    }

    #[test]
    fn summarize_write_file() {
        let summary = summarize_tool_call("write_file", r#"{"path": "out.txt", "content": "hi"}"#);
        assert_eq!(summary, "out.txt");
    }

    #[test]
    fn summarize_run_command() {
        let summary = summarize_tool_call("run_command", r#"{"command": "cargo test"}"#);
        assert_eq!(summary, "cargo test");
    }

    #[test]
    fn summarize_long_command_is_truncated() {
        let long_cmd = "a".repeat(100);
        let args = format!(r#"{{"command": "{long_cmd}"}}"#);
        let summary = summarize_tool_call("run_command", &args);
        assert!(
            summary.len() <= 83,
            "should be truncated: len={}",
            summary.len()
        );
        assert!(summary.ends_with("..."));
    }

    #[test]
    fn summarize_invalid_json_falls_back() {
        let summary = summarize_tool_call("unknown", "not json");
        assert_eq!(summary, "not json");
    }

    // --- extract_text_output tests ---

    #[test]
    fn extract_text_from_message() {
        let output = vec![OutputItem::Message {
            role: grokrs_api::types::common::Role::Assistant,
            content: vec![ContentBlock::Text {
                text: "Hello, world!".to_string(),
            }],
        }];
        let text = extract_text_output(&output);
        assert_eq!(text, "Hello, world!");
    }

    #[test]
    fn extract_text_skips_function_calls() {
        let output = vec![
            OutputItem::FunctionCall {
                id: "fc1".to_string(),
                call_id: "c1".to_string(),
                name: "read_file".to_string(),
                arguments: "{}".to_string(),
            },
            OutputItem::Message {
                role: grokrs_api::types::common::Role::Assistant,
                content: vec![ContentBlock::Text {
                    text: "Done!".to_string(),
                }],
            },
        ];
        let text = extract_text_output(&output);
        assert_eq!(text, "Done!");
    }

    #[test]
    fn extract_text_empty_output() {
        let text = extract_text_output(&[]);
        assert!(text.is_empty());
    }

    // --- tool registry trust rank filtering ---

    #[test]
    fn untrusted_gets_read_and_list_tools() {
        let registry = default_registry();
        let tools = registry.available_tools(0);
        let names: Vec<&str> = tools.iter().map(|t| t.name()).collect();
        assert!(names.contains(&"read_file"));
        assert!(names.contains(&"list_directory"));
        assert!(!names.contains(&"write_file"));
        assert!(!names.contains(&"run_command"));
    }

    #[test]
    fn interactive_adds_write_file() {
        let registry = default_registry();
        let tools = registry.available_tools(1);
        let names: Vec<&str> = tools.iter().map(|t| t.name()).collect();
        assert!(names.contains(&"read_file"));
        assert!(names.contains(&"list_directory"));
        assert!(names.contains(&"write_file"));
        assert!(!names.contains(&"run_command"));
    }

    #[test]
    fn admin_adds_run_command() {
        let registry = default_registry();
        let tools = registry.available_tools(2);
        let names: Vec<&str> = tools.iter().map(|t| t.name()).collect();
        assert!(names.contains(&"read_file"));
        assert!(names.contains(&"list_directory"));
        assert!(names.contains(&"write_file"));
        assert!(names.contains(&"run_command"));
    }

    // --- check_network_allowed tests ---

    #[test]
    fn check_network_denied_returns_error() {
        let config = test_config(false);
        let err = check_network_allowed(&config).unwrap_err();
        let msg = format!("{err}");
        assert!(msg.contains("Network access is denied"), "msg: {msg}");
    }

    #[test]
    fn check_network_allowed_returns_ok() {
        let config = test_config(true);
        assert!(check_network_allowed(&config).is_ok());
    }

    // --- AgentArgs parsing ---

    #[test]
    fn agent_args_defaults() {
        let args = AgentArgs::parse_from(["agent", "do something"]);
        assert_eq!(args.task.as_deref(), Some("do something"));
        assert!(args.trust.is_none());
        assert!(args.max_iterations.is_none());
        assert!(args.model.is_none());
        assert!(args.system.is_none());
        assert!(!args.dry_run);
        assert!(!args.search);
        assert!(!args.x_search);
        assert!(!args.citations);
        assert!(!args.headless);
        assert_eq!(args.output, OutputFormat::Text);
        assert!(args.timeout.is_none());
        assert!(args.approval_mode.is_none());
    }

    #[test]
    fn agent_args_with_flags() {
        let args = AgentArgs::parse_from([
            "agent",
            "refactor code",
            "--trust",
            "admin",
            "--max-iterations",
            "5",
            "--model",
            "grok-4-mini",
            "--system",
            "Be concise.",
            "--dry-run",
        ]);
        assert_eq!(args.task.as_deref(), Some("refactor code"));
        assert_eq!(args.trust.as_deref(), Some("admin"));
        assert_eq!(args.max_iterations, Some(5));
        assert_eq!(args.model.as_deref(), Some("grok-4-mini"));
        assert_eq!(args.system.as_deref(), Some("Be concise."));
        assert!(args.dry_run);
    }

    #[test]
    fn agent_args_with_search_flags() {
        let args = AgentArgs::parse_from([
            "agent",
            "find info",
            "--search",
            "--x-search",
            "--citations",
        ]);
        assert_eq!(args.task.as_deref(), Some("find info"));
        assert!(args.search);
        assert!(args.x_search);
        assert!(args.citations);
    }

    // --- Search integration ---

    #[test]
    fn search_config_merges_with_function_tools() {
        let registry = default_registry();
        let mut tool_defs = registry.tool_definitions(0); // untrusted
        let func_tool_count = tool_defs.len();
        assert!(
            func_tool_count >= 2,
            "should have at least read_file and list_directory"
        );

        let search_config = SearchConfig {
            web_search: true,
            x_search: true,
            ..Default::default()
        };
        tool_defs.extend(search_config.tool_values());

        // Function tools + 2 search tools.
        assert_eq!(tool_defs.len(), func_tool_count + 2);

        // Verify the last two are search tools.
        let last_two = &tool_defs[func_tool_count..];
        assert_eq!(last_two[0]["type"], "web_search");
        assert_eq!(last_two[1]["type"], "x_search");

        // Verify function tools are still present.
        assert!(
            tool_defs[0].get("name").is_some(),
            "first tool should be a function definition"
        );
    }

    #[test]
    fn search_config_empty_does_not_add_tools() {
        let registry = default_registry();
        let mut tool_defs = registry.tool_definitions(0);
        let original_count = tool_defs.len();

        let search_config = SearchConfig::default();
        tool_defs.extend(search_config.tool_values());

        assert_eq!(tool_defs.len(), original_count);
    }

    #[test]
    fn extract_text_skips_search_call_items() {
        let output = vec![
            OutputItem::WebSearchCall {
                id: "ws_1".to_string(),
                status: Some("completed".to_string()),
                search_results: Some(vec![
                    serde_json::json!({"url": "https://example.com", "title": "Example"}),
                ]),
            },
            OutputItem::Message {
                role: grokrs_api::types::common::Role::Assistant,
                content: vec![ContentBlock::Text {
                    text: "Based on my search...".to_string(),
                }],
            },
        ];
        let text = extract_text_output(&output);
        assert_eq!(text, "Based on my search...");
    }

    #[test]
    fn extract_citations_from_agent_output() {
        let output = vec![
            OutputItem::WebSearchCall {
                id: "ws_1".to_string(),
                status: Some("completed".to_string()),
                search_results: Some(vec![
                    serde_json::json!({"url": "https://example.com", "title": "Example"}),
                ]),
            },
            OutputItem::XSearchCall {
                id: "xs_1".to_string(),
                status: Some("completed".to_string()),
                search_results: Some(vec![
                    serde_json::json!({"url": "https://x.com/post", "title": "X Post"}),
                ]),
            },
        ];
        let citations = search::extract_citations_from_output(&output);
        assert_eq!(citations.len(), 2);
        assert_eq!(citations[0].url, "https://example.com");
        assert_eq!(citations[1].url, "https://x.com/post");

        let formatted = search::format_citations(&citations);
        assert!(formatted.contains("Sources:"));
        assert!(formatted.contains("[1] Example"));
        assert!(formatted.contains("[2] X Post"));
    }

    // --- Helper ---

    fn test_config(allow_network: bool) -> AppConfig {
        use grokrs_core::{ModelConfig, PolicyConfig, SessionConfig, WorkspaceConfig};
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
                allow_workspace_writes: false,
                max_patch_bytes: 0,
            },
            session: SessionConfig {
                approval_mode: "interactive".into(),
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

    // --- Option-based CLI override regression tests ---

    #[test]
    fn agent_args_trust_none_when_omitted() {
        let args = AgentArgs::parse_from(["agent", "task"]);
        assert!(args.trust.is_none(), "trust should be None when omitted");
    }

    #[test]
    fn agent_args_trust_explicit_untrusted() {
        let args = AgentArgs::parse_from(["agent", "task", "--trust", "untrusted"]);
        assert_eq!(args.trust.as_deref(), Some("untrusted"));
    }

    #[test]
    fn agent_args_max_iterations_none_when_omitted() {
        let args = AgentArgs::parse_from(["agent", "task"]);
        assert!(
            args.max_iterations.is_none(),
            "max_iterations should be None when omitted"
        );
    }

    #[test]
    fn agent_args_max_iterations_explicit_10() {
        let args = AgentArgs::parse_from(["agent", "task", "--max-iterations", "10"]);
        assert_eq!(args.max_iterations, Some(10));
    }

    // --- UTF-8 safe truncation ---

    #[test]
    fn truncate_utf8_ascii() {
        assert_eq!(truncate_utf8("hello", 10), "hello");
        assert_eq!(truncate_utf8("hello world", 5), "hello");
    }

    #[test]
    fn truncate_utf8_multibyte_boundary() {
        // 'é' is 2 bytes in UTF-8. "café" = 5 bytes.
        let s = "café";
        assert_eq!(s.len(), 5);
        // Truncating at 4 would split 'é' — should back up to 3.
        assert_eq!(truncate_utf8(s, 4), "caf");
        // Truncating at 5 returns the full string.
        assert_eq!(truncate_utf8(s, 5), "café");
        // Truncating at 3 stops before 'é'.
        assert_eq!(truncate_utf8(s, 3), "caf");
    }

    #[test]
    fn truncate_utf8_emoji() {
        // '🦀' is 4 bytes.
        let s = "hi🦀bye";
        assert_eq!(s.len(), 9); // 2 + 4 + 3
        // Truncating at 3 would split the emoji — should back up to 2.
        assert_eq!(truncate_utf8(s, 3), "hi");
        // Truncating at 6 includes the full emoji.
        assert_eq!(truncate_utf8(s, 6), "hi🦀");
    }

    #[test]
    fn summarize_multibyte_command_truncates_safely() {
        // 100 emoji characters = 400 bytes.
        let long_emoji_cmd = "🦀".repeat(100);
        let args = format!(r#"{{"command": "{long_emoji_cmd}"}}"#);
        let summary = summarize_tool_call("run_command", &args);
        // Should not panic and should end with "..."
        assert!(summary.ends_with("..."));
        // Should be valid UTF-8 (no panic on Display).
        let _ = summary.clone();
    }

    // =====================================================================
    // Headless mode tests
    // =====================================================================

    // --- AgentExitCode ---

    #[test]
    fn exit_code_values_are_stable() {
        assert_eq!(AgentExitCode::Success.code(), 0);
        assert_eq!(AgentExitCode::AgentError.code(), 1);
        assert_eq!(AgentExitCode::PolicyDenial.code(), 2);
        assert_eq!(AgentExitCode::ApiError.code(), 3);
        assert_eq!(AgentExitCode::Timeout.code(), 4);
    }

    // --- exit_code_for_tool_loop_error ---

    #[test]
    fn exit_code_max_iterations() {
        let err = ToolLoopError::MaxIterationsExceeded {
            iterations: 10,
            max: 10,
        };
        assert_eq!(
            exit_code_for_tool_loop_error(&err),
            AgentExitCode::AgentError
        );
    }

    #[test]
    fn exit_code_invalid_config() {
        let err = ToolLoopError::InvalidConfiguration {
            message: "bad config".to_owned(),
        };
        assert_eq!(
            exit_code_for_tool_loop_error(&err),
            AgentExitCode::AgentError
        );
    }

    #[test]
    fn exit_code_execution_failed_policy_denied() {
        let err = ToolLoopError::ExecutionFailed {
            name: "write_file".to_owned(),
            error: "policy denied tool 'write_file'".into(),
        };
        assert_eq!(
            exit_code_for_tool_loop_error(&err),
            AgentExitCode::PolicyDenial
        );
    }

    #[test]
    fn exit_code_execution_failed_generic() {
        let err = ToolLoopError::ExecutionFailed {
            name: "read_file".to_owned(),
            error: "file not found".into(),
        };
        assert_eq!(
            exit_code_for_tool_loop_error(&err),
            AgentExitCode::AgentError
        );
    }

    #[test]
    fn exit_code_transport() {
        use grokrs_api::transport::error::TransportError;
        let err =
            ToolLoopError::Transport(TransportError::Api(grokrs_api::types::error::ApiError {
                status_code: 429,
                message: "rate limit".to_owned(),
                error_type: Some("rate_limit_error".to_owned()),
                code: Some("rate_limit".to_owned()),
                request_id: None,
            }));
        assert_eq!(exit_code_for_tool_loop_error(&err), AgentExitCode::ApiError);
    }

    // --- exit_code_for_anyhow ---

    #[test]
    fn anyhow_network_denied_maps_to_policy() {
        let err = anyhow::anyhow!("Network access is denied by policy");
        assert_eq!(exit_code_for_anyhow(&err), AgentExitCode::PolicyDenial);
    }

    #[test]
    fn anyhow_api_client_maps_to_api_error() {
        let err = anyhow::anyhow!("failed to construct API client: missing key");
        assert_eq!(exit_code_for_anyhow(&err), AgentExitCode::ApiError);
    }

    #[test]
    fn anyhow_generic_maps_to_agent_error() {
        let err = anyhow::anyhow!("something else went wrong");
        assert_eq!(exit_code_for_anyhow(&err), AgentExitCode::AgentError);
    }

    // --- HeadlessEvent JSON serialization ---

    #[test]
    fn headless_event_tool_call_serialization() {
        let event = HeadlessEvent::ToolCall {
            name: "read_file".to_owned(),
            arguments: r#"{"path":"src/main.rs"}"#.to_owned(),
        };
        let json = serde_json::to_string(&event).unwrap();
        let parsed: serde_json::Value = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed["type"], "tool_call");
        assert_eq!(parsed["name"], "read_file");
        assert_eq!(parsed["arguments"], r#"{"path":"src/main.rs"}"#);
    }

    #[test]
    fn headless_event_tool_result_serialization() {
        let event = HeadlessEvent::ToolResult {
            name: "read_file".to_owned(),
            output: "file contents here".to_owned(),
        };
        let json = serde_json::to_string(&event).unwrap();
        let parsed: serde_json::Value = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed["type"], "tool_result");
        assert_eq!(parsed["name"], "read_file");
        assert_eq!(parsed["output"], "file contents here");
    }

    #[test]
    fn headless_event_message_serialization() {
        let event = HeadlessEvent::Message {
            content: "Task completed successfully.".to_owned(),
        };
        let json = serde_json::to_string(&event).unwrap();
        let parsed: serde_json::Value = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed["type"], "message");
        assert_eq!(parsed["content"], "Task completed successfully.");
    }

    #[test]
    fn headless_event_error_serialization() {
        let event = HeadlessEvent::Error {
            message: "policy denied tool 'write_file'".to_owned(),
            exit_code: AgentExitCode::PolicyDenial as u8,
        };
        let json = serde_json::to_string(&event).unwrap();
        let parsed: serde_json::Value = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed["type"], "error");
        assert_eq!(parsed["message"], "policy denied tool 'write_file'");
        assert_eq!(parsed["exit_code"], 2);
    }

    #[test]
    fn headless_event_usage_serialization() {
        let event = HeadlessEvent::Usage {
            input_tokens: 100,
            output_tokens: 50,
            total_tokens: 150,
        };
        let json = serde_json::to_string(&event).unwrap();
        let parsed: serde_json::Value = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed["type"], "usage");
        assert_eq!(parsed["input_tokens"], 100);
        assert_eq!(parsed["output_tokens"], 50);
        assert_eq!(parsed["total_tokens"], 150);
    }

    #[test]
    fn headless_events_are_single_line_json() {
        // All event variants must serialize to a single line (no embedded newlines).
        let events: Vec<HeadlessEvent> = vec![
            HeadlessEvent::ToolCall {
                name: "read_file".to_owned(),
                arguments: r#"{"path":"test.rs"}"#.to_owned(),
            },
            HeadlessEvent::ToolResult {
                name: "read_file".to_owned(),
                output: "fn main() {}".to_owned(),
            },
            HeadlessEvent::Message {
                content: "Done.".to_owned(),
            },
            HeadlessEvent::Error {
                message: "oops".to_owned(),
                exit_code: 1,
            },
            HeadlessEvent::Usage {
                input_tokens: 10,
                output_tokens: 5,
                total_tokens: 15,
            },
        ];
        for event in &events {
            let json = serde_json::to_string(event).unwrap();
            assert!(
                !json.contains('\n'),
                "JSON event should be single line: {json}"
            );
        }
    }

    // --- Headless CLI argument parsing ---

    #[test]
    fn agent_args_headless_flag() {
        let args = AgentArgs::parse_from(["agent", "--headless", "some task"]);
        assert!(args.headless);
        assert_eq!(args.task.as_deref(), Some("some task"));
        assert_eq!(args.output, OutputFormat::Text);
    }

    #[test]
    fn agent_args_headless_with_json_output() {
        let args = AgentArgs::parse_from(["agent", "--headless", "--output", "json", "task"]);
        assert!(args.headless);
        assert_eq!(args.output, OutputFormat::Json);
    }

    #[test]
    fn agent_args_timeout_flag() {
        let args = AgentArgs::parse_from(["agent", "--timeout", "60", "task"]);
        assert_eq!(args.timeout, Some(60));
    }

    #[test]
    fn agent_args_approval_mode_override() {
        let args =
            AgentArgs::parse_from(["agent", "--headless", "--approval-mode", "allow", "task"]);
        assert!(args.headless);
        assert_eq!(args.approval_mode.as_deref(), Some("allow"));
    }

    #[test]
    fn agent_args_headless_no_task_arg() {
        // In headless mode, task can be omitted (reads from stdin).
        let args = AgentArgs::parse_from(["agent", "--headless"]);
        assert!(args.headless);
        assert!(args.task.is_none());
    }

    // --- resolve_task tests ---

    #[test]
    fn resolve_task_from_arg() {
        let args = AgentArgs::parse_from(["agent", "do the thing"]);
        let task = resolve_task(&args).unwrap();
        assert_eq!(task, "do the thing");
    }

    #[test]
    fn resolve_task_missing_in_interactive_mode_is_error() {
        let args = AgentArgs::parse_from(["agent"]);
        let err = resolve_task(&args).unwrap_err();
        let msg = format!("{err}");
        assert!(
            msg.contains("task argument is required"),
            "unexpected error: {msg}"
        );
    }

    // --- Headless approval mode resolution ---

    #[test]
    fn headless_defaults_approval_mode_to_deny() {
        // Simulating the resolution logic from run().
        let approval_mode_override: Option<&str> = None;
        let headless = true;
        let config_approval = "interactive";

        let resolved = if let Some(mode) = approval_mode_override {
            mode.to_owned()
        } else if headless {
            "deny".to_owned()
        } else {
            config_approval.to_owned()
        };

        assert_eq!(resolved, "deny");
    }

    #[test]
    fn headless_approval_mode_can_be_overridden() {
        let approval_mode_override: Option<&str> = Some("allow");
        let headless = true;
        let config_approval = "interactive";

        let resolved = if let Some(mode) = approval_mode_override {
            mode.to_owned()
        } else if headless {
            "deny".to_owned()
        } else {
            config_approval.to_owned()
        };

        assert_eq!(resolved, "allow");
    }

    #[test]
    fn interactive_uses_config_approval_mode() {
        let approval_mode_override: Option<&str> = None;
        let headless = false;
        let config_approval = "interactive";

        let resolved = if let Some(mode) = approval_mode_override {
            mode.to_owned()
        } else if headless {
            "deny".to_owned()
        } else {
            config_approval.to_owned()
        };

        assert_eq!(resolved, "interactive");
    }

    // --- Timeout resolution ---

    #[test]
    fn headless_default_timeout_300s() {
        let explicit_timeout: Option<u64> = None;
        let headless = true;
        let resolved = explicit_timeout.or(if headless { Some(300) } else { None });
        assert_eq!(resolved, Some(300));
    }

    #[test]
    fn interactive_no_default_timeout() {
        let explicit_timeout: Option<u64> = None;
        let headless = false;
        let resolved = explicit_timeout.or(if headless { Some(300) } else { None });
        assert_eq!(resolved, None);
    }

    #[test]
    fn explicit_timeout_overrides_default() {
        let explicit_timeout: Option<u64> = Some(60);
        let headless = true;
        let resolved = explicit_timeout.or(if headless { Some(300) } else { None });
        assert_eq!(resolved, Some(60));
    }
}
