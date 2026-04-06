use grokrs_cli::commands;
use grokrs_cli::telemetry;

use anyhow::{Context, Result};
use clap::{Parser, Subcommand};
use grokrs_cap::{Untrusted, WorkspacePath};
use grokrs_core::{AppConfig, check_deprecated_model, resolve_profile};
use grokrs_policy::{Decision, Effect, PolicyEngine};
use grokrs_session::{Session, SessionState};
use std::env;
use std::path::{Path, PathBuf};

use commands::agent::AgentArgs;
use commands::api::ApiCommand;
use commands::chat::ChatArgs;
use commands::collections::CollectionsCommand;
use commands::generate::GenerateCommand;
use commands::models::ModelsCommand;
use commands::sessions::SessionsCommand;
use commands::store::StoreCommand;
use commands::voice::VoiceArgs;

/// Default config path used when `--config` is not specified.
const DEFAULT_CONFIG_PATH: &str = "configs/grokrs.example.toml";

#[derive(Parser)]
#[command(name = "grokrs")]
#[command(about = "Safe Rust-only scaffold for a Grok-oriented development CLI")]
#[command(after_help = "\
Examples:
  grokrs doctor                              Run diagnostics with R2 feature status
  grokrs chat                                Interactive chat REPL
  grokrs chat --search --stateful            Chat with web search, server-side history
  grokrs chat --cache-key my-prompt          Chat with prompt caching
  grokrs agent 'explain src/main.rs'         Run an agentic task
  grokrs agent --headless 'task'             CI/CD mode (exit codes 0-4)
  grokrs agent --headless --output json 'task'   JSON event stream
  grokrs agent --mcp-server http://localhost:3000 'task'  With MCP tools
  grokrs voice                               Interactive voice session
  grokrs voice --text-only                   Voice without audio I/O
  grokrs generate image 'a cat' -o cat.png   Generate an image
  grokrs models list                         List available models
  grokrs store cost --group-by model         Cost breakdown by model
  grokrs --profile dev chat                  Use dev config profile

See `grokrs <command> --help` for details on each command.")]
struct Cli {
    /// Path to the TOML configuration file
    #[arg(long, global = true, default_value = DEFAULT_CONFIG_PATH)]
    config: PathBuf,

    /// Configuration profile name (e.g., dev, staging, prod).
    /// Loads configs/grokrs.NAME.toml and merges it on top of the base config.
    /// Overrides GROKRS_PROFILE env var.
    #[arg(long, global = true)]
    profile: Option<String>,

    /// OpenTelemetry OTLP exporter endpoint (e.g., http://localhost:4317).
    /// Requires the 'otel' feature. Overrides GROKRS_OTEL_ENDPOINT env var.
    #[arg(long, global = true)]
    otel_endpoint: Option<String>,

    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Run diagnostics and verify the environment
    Doctor,
    /// Display the parsed configuration
    ShowConfig {
        /// Path to the config file (overrides --config)
        path: Option<PathBuf>,
    },
    /// Evaluate a policy decision for an effect
    Eval {
        #[command(subcommand)]
        effect: EvalCommand,
    },
    /// Interactive chat REPL with Grok API streaming
    Chat(ChatArgs),
    /// Run a single agentic task with tool execution
    Agent(AgentArgs),
    /// Generate images or videos using Grok models
    Generate {
        #[command(subcommand)]
        command: GenerateCommand,
    },
    /// Discover and inspect available models
    Models {
        #[command(subcommand)]
        command: ModelsCommand,
    },
    /// Interact with the xAI Grok API
    Api {
        #[command(subcommand)]
        command: ApiCommand,
    },
    /// Manage collections via the xAI Management API
    Collections {
        #[command(subcommand)]
        command: CollectionsCommand,
    },
    /// Browse and manage stored sessions
    Sessions {
        #[command(subcommand)]
        command: SessionsCommand,
    },
    /// Inspect store state and usage
    Store {
        #[command(subcommand)]
        command: StoreCommand,
    },
    /// Interactive voice session with the xAI Voice Agent API
    Voice(VoiceArgs),
}

#[derive(Subcommand)]
enum EvalCommand {
    Read { path: String },
    Write { path: String },
    Network { host: String },
    Spawn { program: String },
}

/// Load the application config, applying profile overlay if one is active.
///
/// Profile resolution order: `--profile` flag > `GROKRS_PROFILE` env var > none.
///
/// After loading, emits a deprecation warning to stderr if the configured
/// default model name belongs to a known-deprecated Grok family (grok-2,
/// grok-3). The warning is informational and does not prevent operation.
fn load_config(cli: &Cli) -> Result<AppConfig> {
    let profile = resolve_profile(cli.profile.as_deref());
    let config = match profile {
        Some(ref name) => AppConfig::load_with_profile(&cli.config, Some(name.as_str()))
            .with_context(|| {
                format!(
                    "failed to load config from {} with profile '{}'",
                    cli.config.display(),
                    name
                )
            })?,
        None => AppConfig::load(&cli.config)
            .with_context(|| format!("failed to load config from {}", cli.config.display()))?,
    };
    check_deprecated_model(&config.model.default_model);
    Ok(config)
}

fn main() -> Result<()> {
    let cli = Cli::parse();

    // Initialize OpenTelemetry if the `otel` feature is enabled and an endpoint
    // is configured. The guard flushes pending spans on drop.
    let _telemetry_guard = telemetry::init(cli.otel_endpoint.as_deref());

    match cli.command {
        Command::Doctor => doctor(&cli),
        Command::ShowConfig { ref path } => {
            let config_path = path.clone().unwrap_or_else(|| cli.config.clone());
            show_config(&cli, &config_path)
        }
        Command::Eval { ref effect } => evaluate_effect(effect, &cli),
        Command::Chat(ref args) => {
            let config = load_config(&cli)?;
            let rt = tokio::runtime::Builder::new_multi_thread()
                .enable_all()
                .build()
                .context("failed to build tokio runtime")?;
            commands::chat::run(args, &config, rt.handle())
        }
        Command::Agent(ref args) => {
            let config = load_config(&cli)?;
            let rt = tokio::runtime::Builder::new_multi_thread()
                .enable_all()
                .build()
                .context("failed to build tokio runtime")?;
            match rt.block_on(commands::agent::run(args, &config)) {
                Ok(result) => {
                    let code = result.exit_code.code();
                    if code != 0 {
                        std::process::exit(code);
                    }
                    Ok(())
                }
                Err(e) => {
                    if args.headless {
                        let code = commands::agent::exit_code_for_anyhow_public(&e).code();
                        eprintln!("[agent] fatal: {e:#}");
                        std::process::exit(code);
                    }
                    Err(e)
                }
            }
        }
        Command::Generate { ref command } => {
            let config = load_config(&cli)?;
            let rt = tokio::runtime::Builder::new_multi_thread()
                .enable_all()
                .build()
                .context("failed to build tokio runtime")?;
            rt.block_on(commands::generate::run(command, &config))
        }
        Command::Models { ref command } => {
            let config = load_config(&cli)?;
            let rt = tokio::runtime::Builder::new_multi_thread()
                .enable_all()
                .build()
                .context("failed to build tokio runtime")?;
            rt.block_on(commands::models::run(command, &config))
        }
        Command::Api { ref command } => {
            let config = load_config(&cli)?;
            let rt = tokio::runtime::Builder::new_multi_thread()
                .enable_all()
                .build()
                .context("failed to build tokio runtime")?;
            rt.block_on(commands::api::run(command, &config))
        }
        Command::Collections { ref command } => {
            let config = load_config(&cli)?;
            let rt = tokio::runtime::Builder::new_multi_thread()
                .enable_all()
                .build()
                .context("failed to build tokio runtime")?;
            rt.block_on(commands::collections::run(command, &config))
        }
        Command::Sessions { ref command } => {
            let config = load_config(&cli)?;
            let workspace_root =
                env::current_dir().context("failed to resolve current directory")?;
            commands::sessions::run(command, &config, &workspace_root)
        }
        Command::Store { ref command } => {
            let config = load_config(&cli)?;
            let workspace_root =
                env::current_dir().context("failed to resolve current directory")?;
            commands::store::run(command, &config, &workspace_root)
        }
        Command::Voice(ref args) => {
            let config = load_config(&cli)?;
            let rt = tokio::runtime::Builder::new_multi_thread()
                .enable_all()
                .build()
                .context("failed to build tokio runtime")?;
            rt.block_on(commands::voice::run(args, &config))
        }
    }
}

fn doctor(cli: &Cli) -> Result<()> {
    let cwd = env::current_dir().context("failed to resolve current directory")?;
    let mut session = Session::<Untrusted>::new("bootstrap");
    session.transition(SessionState::Ready);
    println!("grokrs doctor");
    println!("workspace={}", cwd.display());
    println!("session_id={}", session.id());
    println!("session_state={:?}", session.state());
    println!("safety=typed_trust,rooted_paths,deny_by_default");

    let profile = resolve_profile(cli.profile.as_deref());
    if let Some(ref name) = profile {
        println!("profile={name}");
    }

    // Store status (best-effort: does not fail doctor if config is missing).
    match load_config(cli) {
        Ok(config) => {
            commands::store::doctor_report(&config, &cwd);
            doctor_features(&config, cli.otel_endpoint.as_deref(), &cwd);
        }
        Err(_) => println!("store=unconfigured (config not loaded)"),
    }

    Ok(())
}

/// Check core feature readiness (chat, agent, search, generate, models, sessions).
fn doctor_core_features(config: &AppConfig, network_ready: bool, network_decision: &Decision) {
    let engine = PolicyEngine::new(config.policy.clone());

    if network_ready {
        println!("chat=ready");
    } else {
        println!(
            "chat=blocked: network access denied. Set allow_network=true in [policy] and approval_mode=allow in [session]"
        );
    }

    let registry = grokrs_tool::registry::default_registry();
    let (untrusted_count, interactive_count, admin_count) = (
        registry.available_tools(0).len(),
        registry.available_tools(1).len(),
        registry.available_tools(2).len(),
    );
    if network_ready {
        println!(
            "agent=ready (tools: untrusted={untrusted_count}, interactive={interactive_count}, admin={admin_count})"
        );
    } else {
        println!(
            "agent=blocked: network access denied (tools: untrusted={untrusted_count}, interactive={interactive_count}, admin={admin_count})"
        );
    }

    let fs_write_decision = engine.evaluate(&Effect::FsWrite(
        grokrs_cap::WorkspacePath::new("test.txt").expect("static path is valid"),
    ));
    let spawn_decision = engine.evaluate(&Effect::ProcessSpawn {
        program: "cargo".to_owned(),
    });
    println!(
        "policy_fs_write={}",
        format_decision_short(&fs_write_decision)
    );
    println!(
        "policy_process_spawn={}",
        format_decision_short(&spawn_decision)
    );
    println!("policy_network={}", format_decision_short(network_decision));

    let network_features = [
        (
            "search",
            "ready (web_search, x_search via --search/--x-search flags)",
            "blocked: requires network access",
        ),
        (
            "generate",
            "ready (image, video via grokrs generate)",
            "blocked: requires network access",
        ),
        (
            "models",
            "ready (list, info, pricing via grokrs models)",
            "blocked: requires network access",
        ),
    ];
    for (name, ready_msg, blocked_msg) in &network_features {
        println!(
            "{name}={}",
            if network_ready {
                ready_msg
            } else {
                blocked_msg
            }
        );
    }

    let store_ok = config.store.as_ref().is_none_or(|s| !s.path.is_empty());
    if store_ok {
        println!("sessions=ready (list, show, transcript, clean via grokrs sessions)");
    } else {
        println!("sessions=blocked: store path is empty");
    }
}

/// Print approval mode status with security warning if set to "allow".
fn doctor_approval_mode(config: &AppConfig) {
    if config.session.approval_mode == "allow" {
        println!(
            "approval_mode=allow  *** SECURITY WARNING ***\n\
             \n\
             approval_mode='allow' bypasses the interactive approval gate: every\n\
             Ask decision (network, shell, filesystem write) is automatically\n\
             approved without human review. This is intended for local development\n\
             only and MUST NOT be used in shared, automated, or production\n\
             environments where tool misuse or prompt-injection could cause harm.\n\
             \n\
             Risks when approval_mode='allow' is active:\n\
             - Network: any model-requested host is contacted without confirmation\n\
             - Shell:   model-requested commands execute without review\n\
             - Writes:  workspace files are modified without review\n\
             \n\
             To harden: set approval_mode='interactive' or approval_mode='deny'\n\
             in the [session] section of your config."
        );
    } else {
        println!("approval_mode={}", config.session.approval_mode);
    }

    if let Some(ref agent) = config.agent {
        println!(
            "agent_config: max_iterations={} default_trust={} enable_search={}",
            agent.max_iterations, agent.default_trust, agent.enable_search
        );
    }
    if let Some(ref chat) = config.chat {
        let model = chat.default_model.as_deref().unwrap_or("(inherit)");
        println!(
            "chat_config: default_model={} stateful={} max_conversation_tokens={}",
            model, chat.stateful, chat.max_conversation_tokens
        );
    }
}

/// Check R2 features: voice, otel, MCP, git, model freshness, memory/store.
fn doctor_r2_features(
    config: &AppConfig,
    otel_endpoint: Option<&str>,
    workspace_root: &std::path::Path,
) {
    // Voice agent / audio.
    if cfg!(feature = "audio") {
        println!("[ok] voice_agent=enabled (audio feature compiled in)");
    } else {
        println!(
            "[--] voice_agent=disabled (audio feature not compiled in; rebuild with --features audio)"
        );
    }

    // OpenTelemetry.
    if cfg!(feature = "otel") {
        let effective_endpoint = otel_endpoint
            .map(ToOwned::to_owned)
            .or_else(|| std::env::var("GROKRS_OTEL_ENDPOINT").ok());
        match effective_endpoint {
            Some(ref ep) => println!("[ok] otel=enabled endpoint={ep}"),
            None => println!(
                "[warn] otel=enabled but no endpoint configured (set --otel-endpoint or GROKRS_OTEL_ENDPOINT)"
            ),
        }
    } else {
        println!("[--] otel=disabled (otel feature not compiled in; rebuild with --features otel)");
    }

    // MCP servers.
    match config.mcp.as_ref() {
        None => println!("[--] mcp=not configured (add [mcp] section to enable)"),
        Some(mcp) if mcp.servers.is_empty() => {
            println!("[--] mcp=configured but no servers defined")
        }
        Some(mcp) => {
            println!("[ok] mcp={} server(s) configured", mcp.servers.len());
            let mut names: Vec<&str> = mcp.servers.keys().map(String::as_str).collect();
            names.sort_unstable();
            for name in names {
                let srv = &mcp.servers[name];
                let label = srv.label.as_deref().unwrap_or(name);
                println!(
                    "     mcp.{name}: url={} label={label} trust_rank={} timeout={}s",
                    srv.url, srv.trust_rank, srv.timeout_secs
                );
            }
        }
    }

    // Git repository.
    let mut git_root = None;
    let mut dir = workspace_root.to_owned();
    loop {
        if dir.join(".git").exists() {
            git_root = Some(dir.clone());
            break;
        }
        match dir.parent() {
            Some(p) => dir = p.to_owned(),
            None => break,
        }
    }
    match git_root {
        Some(ref root) => println!("[ok] git=repo detected at {}", root.display()),
        None => println!("[warn] git=not a git repository (workspace not under version control)"),
    }

    doctor_model_freshness(config);
    doctor_memory_store(config, workspace_root);
}

/// Check whether configured model names look deprecated.
fn doctor_model_freshness(config: &AppConfig) {
    const DEPRECATED_PATTERNS: &[&str] = &[
        "grok-1",
        "grok-beta",
        "-preview",
        "-legacy",
        "-old",
        "-deprecated",
    ];
    let default_model = &config.model.default_model;
    if DEPRECATED_PATTERNS
        .iter()
        .any(|pat| default_model.contains(pat))
    {
        println!(
            "[warn] model_freshness=WARN default_model={default_model} appears deprecated; update [model].default_model"
        );
    } else {
        println!("[ok] model_freshness=ok default_model={default_model}");
    }
    if let Some(ref chat) = config.chat
        && let Some(ref chat_model) = chat.default_model
        && DEPRECATED_PATTERNS
            .iter()
            .any(|pat| chat_model.contains(pat))
    {
        println!(
            "[warn] model_freshness=WARN chat.default_model={chat_model} appears deprecated; update [chat].default_model"
        );
    }
}

/// Check agent memory count and store file size.
fn doctor_memory_store(config: &AppConfig, workspace_root: &std::path::Path) {
    let store_path = config
        .store
        .as_ref()
        .map_or(".grokrs/state.db", |s| s.path.as_str());
    let db_full_path = workspace_root.join(store_path);
    if db_full_path.exists() {
        let size_kb = std::fs::metadata(&db_full_path)
            .map(|m| m.len())
            .unwrap_or(0)
            / 1024;
        match grokrs_store::Store::open_with_path(workspace_root, store_path) {
            Ok(store) => {
                let memory_count = store.memories().count().unwrap_or(0);
                let memory_limit = config.agent.as_ref().map_or(50, |a| a.memory_limit);
                store.close().ok();
                println!(
                    "[ok] memory={memory_count}/{memory_limit} entries store_size={size_kb}KB ({})",
                    db_full_path.display()
                );
            }
            Err(e) => println!(
                "[warn] memory=store open error: {e} path={}",
                db_full_path.display()
            ),
        }
    } else {
        println!("[--] memory=store not created yet (run an agent task to initialise)");
    }
}

/// Report on competitive feature readiness based on configuration.
///
/// Each feature reports "ready" or "blocked: <reason>" with actionable
/// fix instructions. No API calls are made — readiness is config-based only.
fn doctor_features(
    config: &AppConfig,
    otel_endpoint: Option<&str>,
    workspace_root: &std::path::Path,
) {
    let engine = PolicyEngine::new(config.policy.clone());

    println!();
    println!("--- feature status ---");

    let network_decision = engine.evaluate(&Effect::NetworkConnect {
        host: "api.x.ai".to_owned(),
    });
    let network_ready = match &network_decision {
        Decision::Allow { .. } => true,
        Decision::Ask { .. } => config.session.approval_mode == "allow",
        Decision::Deny { .. } => false,
    };

    doctor_core_features(config, network_ready, &network_decision);
    doctor_approval_mode(config);
    doctor_r2_features(config, otel_endpoint, workspace_root);
}

/// Format a policy decision as a short status string.
fn format_decision_short(decision: &Decision) -> &'static str {
    match decision {
        Decision::Allow { .. } => "allow",
        Decision::Ask { .. } => "ask",
        Decision::Deny { .. } => "deny",
    }
}

fn show_config(cli: &Cli, path: &Path) -> Result<()> {
    let profile = resolve_profile(cli.profile.as_deref());
    let config = match profile {
        Some(ref name) => {
            AppConfig::load_with_profile(path, Some(name.as_str())).with_context(|| {
                format!(
                    "failed to load config from {} with profile '{}'",
                    path.display(),
                    name
                )
            })?
        }
        None => AppConfig::load(path)
            .with_context(|| format!("failed to load config from {}", path.display()))?,
    };
    if let Some(ref name) = profile {
        println!("profile={name}");
    }
    println!("{}", config.summary());
    Ok(())
}

fn evaluate_effect(effect: &EvalCommand, cli: &Cli) -> Result<()> {
    let config = load_config(cli)?;
    let engine = PolicyEngine::new(config.policy.clone());

    let effect = match effect {
        EvalCommand::Read { path } => Effect::FsRead(WorkspacePath::new(path.clone())?),
        EvalCommand::Write { path } => Effect::FsWrite(WorkspacePath::new(path.clone())?),
        EvalCommand::Network { host } => Effect::NetworkConnect { host: host.clone() },
        EvalCommand::Spawn { program } => Effect::ProcessSpawn {
            program: program.clone(),
        },
    };

    let decision = engine.evaluate(&effect);
    print_decision(&decision);
    Ok(())
}

fn print_decision(decision: &Decision) {
    match decision {
        Decision::Allow { reason } => println!("allow: {reason}"),
        Decision::Ask { reason } => println!("ask: {reason}"),
        Decision::Deny { reason } => println!("deny: {reason}"),
    }
}
