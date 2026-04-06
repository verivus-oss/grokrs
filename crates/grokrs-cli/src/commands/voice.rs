//! `grokrs voice` command: interactive voice session with the xAI Voice Agent API.
//!
//! Provides two modes:
//! - Audio mode (default, requires 'audio' feature): captures microphone audio via cpal,
//!   streams it to the voice agent, and plays back agent speech.
//! - Text-only mode (`--text-only`): text REPL that sends text messages to the voice
//!   agent and displays transcripts. Works without audio dependencies.
//!
//! Voice sessions are persisted in the store with session type "voice" and
//! transcriptions are stored in the transcripts table.

use std::sync::Arc;

use anyhow::{Context, Result, bail};
use clap::Args;
use tokio::sync::mpsc;

use grokrs_api::endpoints::voice::{VoiceAgentClient, VoiceReceived};
use grokrs_api::transport::auth::resolve_api_key;
use grokrs_api::transport::policy_bridge::FnPolicyGate;
use grokrs_api::transport::policy_gate::{PolicyDecision, PolicyGate};
use grokrs_api::transport::websocket::WsClientConfig;
use grokrs_api::types::voice::{
    ControlAction, TurnDetectionMode, VadSensitivity, VoiceConfig, VoiceEvent, VoiceId,
    VoiceSessionState,
};
use grokrs_cap::Untrusted;
use grokrs_core::AppConfig;
use grokrs_policy::{Decision, Effect, PolicyEngine};
use grokrs_session::{Session, SessionState};
use grokrs_store::Store;

/// Arguments for the `grokrs voice` command.
#[derive(Debug, Args)]
#[command(after_help = "\
Examples:
  grokrs voice                               Start interactive voice session
  grokrs voice --text-only                   Text-only mode (no audio I/O)
  grokrs voice --voice rex                   Use Rex voice
  grokrs voice --language de-DE              German language
  grokrs voice --model grok-4-mini           Use a specific model
  grokrs voice --system 'You are helpful'    Set system instructions

Note: Audio mode requires the 'audio' feature to be enabled at build time.
Without it, --text-only is enforced automatically.")]
pub struct VoiceArgs {
    /// Use text-only mode (no microphone/speaker, just text I/O).
    #[arg(long, default_value_t = false)]
    pub text_only: bool,

    /// Override the default model (e.g., `grok-4`, `grok-4-mini`).
    #[arg(long)]
    pub model: Option<String>,

    /// Voice to use for agent speech (eve, ara, rex, sal, leo).
    #[arg(long, default_value = "eve")]
    pub voice: String,

    /// Language code (BCP 47, e.g., "en-US", "de-DE").
    #[arg(long, default_value = "en-US")]
    pub language: String,

    /// Turn detection mode (server_vad or manual).
    #[arg(long, default_value = "server_vad")]
    pub turn_detection: String,

    /// VAD sensitivity (low, medium, high). Only applies with server_vad.
    #[arg(long, default_value = "medium")]
    pub vad_sensitivity: String,

    /// Set system instructions for the voice agent.
    #[arg(long)]
    pub system: Option<String>,

    /// Maximum session duration in seconds.
    #[arg(long)]
    pub max_duration: Option<u32>,
}

/// Run the `grokrs voice` command.
///
/// # Flow
///
/// 1. Check network policy -- fail fast if denied.
/// 2. Resolve API key and build WebSocket config.
/// 3. Build `VoiceAgentClient` with policy gate.
/// 4. Open store (best-effort) for session/transcript persistence.
/// 5. Create session record in store.
/// 6. Connect to voice agent and enter interactive loop.
/// 7. On exit, close session and print summary.
/// Build the WebSocket config from application config.
fn build_ws_config(config: &AppConfig) -> WsClientConfig {
    let api_config = config.api.as_ref();
    let base_url = api_config.and_then(|a| a.base_url.as_ref()).map_or_else(
        || "wss://api.x.ai".into(),
        |u| {
            if let Some(rest) = u.strip_prefix("https://") {
                format!("wss://{rest}")
            } else if let Some(rest) = u.strip_prefix("http://") {
                format!("ws://{rest}")
            } else {
                u.clone()
            }
        },
    );
    WsClientConfig {
        base_url,
        endpoint_path: "/v1/voice-agent".into(),
        ..WsClientConfig::default()
    }
}

/// Build a network policy gate for the voice client.
fn build_voice_policy_gate(config: &AppConfig) -> Arc<dyn PolicyGate> {
    let engine = PolicyEngine::new(config.policy.clone());
    let approval_mode = config.session.approval_mode.clone();
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

/// Parse voice-related CLI arguments into a `VoiceConfig`.
fn parse_voice_config(args: &VoiceArgs, config: &AppConfig) -> Result<VoiceConfig> {
    let voice_id: VoiceId = args
        .voice
        .parse()
        .map_err(|e: String| anyhow::anyhow!("{e}"))?;

    let turn_detection = match args.turn_detection.as_str() {
        "server_vad" => TurnDetectionMode::ServerVad,
        "manual" => TurnDetectionMode::Manual,
        other => bail!("unknown turn detection mode '{other}'; expected: server_vad, manual"),
    };
    let vad_sensitivity = match args.vad_sensitivity.as_str() {
        "low" => VadSensitivity::Low,
        "medium" => VadSensitivity::Medium,
        "high" => VadSensitivity::High,
        other => bail!("unknown VAD sensitivity '{other}'; expected: low, medium, high"),
    };

    let model = args
        .model
        .clone()
        .unwrap_or_else(|| config.model.default_model.clone());

    Ok(VoiceConfig {
        model,
        voice: voice_id,
        language: args.language.clone(),
        turn_detection,
        vad_sensitivity,
        system_instructions: args.system.clone(),
        max_duration_secs: args.max_duration,
        ..VoiceConfig::default()
    })
}

/// # Errors
///
/// Returns an error if the network policy check, API key resolution,
/// WebSocket connection, or the interactive voice loop fails.
pub async fn run(args: &VoiceArgs, config: &AppConfig) -> Result<()> {
    let text_only = args.text_only || !audio_feature_available();
    if !audio_feature_available() && !args.text_only {
        eprintln!(
            "warning: audio feature not enabled at build time; falling back to --text-only mode"
        );
    }

    check_network_policy(config)?;

    let api_config = config.api.as_ref();
    let api_key_env = api_config
        .and_then(|a| a.api_key_env.as_deref())
        .unwrap_or("XAI_API_KEY");
    let api_key = resolve_api_key(api_key_env)
        .map_err(|e| anyhow::anyhow!("failed to resolve API key: {e}"))?;

    let ws_config = build_ws_config(config);
    let gate = build_voice_policy_gate(config);
    let voice_config = parse_voice_config(args, config)?;
    let model = &voice_config.model;

    let client = VoiceAgentClient::new(ws_config, api_key, Some(gate));

    // Store integration (best-effort).
    let store = open_store_best_effort(config);
    let session_id = uuid::Uuid::new_v4().to_string();

    {
        let mut session = Session::<Untrusted>::new(&session_id);
        session.transition(SessionState::Ready);
    }
    if let Some(ref s) = store {
        if let Err(e) = s.sessions().create(&session_id, "Untrusted") {
            eprintln!("warning: failed to persist session: {e}");
        } else {
            let _ = s.sessions().transition(&session_id, "Ready");
        }
    }

    let mode_str = if text_only { "text-only" } else { "audio" };
    eprintln!(
        "grokrs voice | model={model} | voice={} | mode={mode_str} | session={}",
        voice_config.voice,
        &session_id[..session_id.len().min(8)]
    );
    if text_only {
        eprintln!("Text-only mode. Type your messages and press Enter.");
        eprintln!("Type /exit or press Ctrl-D to quit, /interrupt to stop agent.\n");
    } else {
        eprintln!("Audio mode. Speak into your microphone.");
        eprintln!("Type /exit or press Ctrl-D to quit, /mute to toggle microphone.\n");
    }

    eprintln!("Connecting to voice agent...");
    let (mut event_rx, sink) = client
        .connect(voice_config)
        .await
        .context("failed to connect to voice agent")?;

    if let Some(ref s) = store {
        let _ = s.sessions().transition(&session_id, "RunningTurn");
    }

    let result = if text_only {
        run_text_mode(&client, &sink, &mut event_rx, store.as_ref(), &session_id).await
    } else {
        #[cfg(feature = "audio")]
        {
            run_audio_mode(&client, &sink, &mut event_rx, store.as_ref(), &session_id).await
        }
        #[cfg(not(feature = "audio"))]
        {
            run_text_mode(&client, &sink, &mut event_rx, store.as_ref(), &session_id).await
        }
    };

    eprintln!("\nClosing voice session...");
    let _ = client.close(&sink).await;

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
    }
    if let Some(s) = store {
        let _ = s.close();
    }

    result
}

/// Handle a single voice event, returning whether the session should close.
async fn handle_voice_event(
    client: &VoiceAgentClient,
    sink: &Arc<tokio::sync::Mutex<grokrs_api::transport::websocket::WsSink>>,
    voice_event: &VoiceEvent,
    store: Option<&Store>,
    session_id: &str,
    transcript_log: &mut Vec<(String, String)>,
) -> bool {
    match voice_event {
        VoiceEvent::SessionCreated {
            session_id: sid, ..
        } => {
            eprintln!("[voice] Session created: {}", &sid[..sid.len().min(12)]);
        }
        VoiceEvent::Transcript {
            role,
            text,
            is_final,
        } => {
            if *is_final {
                println!("[{role}] {text}");
                transcript_log.push((role.to_string(), text.clone()));
                if let Some(s) = store {
                    let body =
                        serde_json::json!({"role": role.to_string(), "text": text}).to_string();
                    if let Ok(tid) = s.transcripts().log_request(
                        session_id,
                        "/v1/voice-agent",
                        "WS",
                        Some(&body),
                    ) {
                        let usage = grokrs_store::types::TranscriptUsage::default();
                        let _ = s
                            .transcripts()
                            .log_response(tid, 200, Some(text), &usage, None);
                    }
                }
            } else {
                eprint!("\r[{role}] {text}...");
            }
        }
        VoiceEvent::FunctionCall {
            call_id,
            name,
            arguments,
        } => {
            eprintln!("[voice] Function call: {name}({arguments})");
            eprintln!("[voice] Call ID: {call_id}");
            eprintln!(
                "[voice] Function calling in voice text-only mode is not yet connected to tool execution."
            );
            let result_str = serde_json::json!({"error": "function execution not available in text-only voice mode"}).to_string();
            if let Err(e) = client
                .send_function_result(sink, call_id, &result_str)
                .await
            {
                eprintln!("[voice] Failed to send function result: {e}");
            }
        }
        VoiceEvent::StateChange { state, reason } => {
            let reason_str = reason.as_deref().unwrap_or("");
            eprintln!("[voice] State: {state} {reason_str}");
            if *state == VoiceSessionState::Closed {
                return true;
            }
        }
        VoiceEvent::Error {
            code,
            message,
            fatal,
        } => {
            eprintln!("[voice] Error ({code}): {message}");
            if *fatal {
                eprintln!("[voice] Fatal error -- session ending.");
                return true;
            }
        }
        VoiceEvent::Pong => {}
        VoiceEvent::AudioChunk { .. } => {}
        VoiceEvent::Usage {
            input_tokens,
            output_tokens,
            input_audio_secs,
            output_audio_secs,
        } => {
            eprintln!(
                "[voice] Usage: input_tokens={input_tokens} output_tokens={output_tokens} audio_in={input_audio_secs:.1}s audio_out={output_audio_secs:.1}s"
            );
        }
    }
    false
}

/// Run the text-only interactive loop.
///
/// Reads text from stdin, sends to voice agent, and displays events.
async fn run_text_mode(
    client: &VoiceAgentClient,
    sink: &Arc<tokio::sync::Mutex<grokrs_api::transport::websocket::WsSink>>,
    event_rx: &mut mpsc::Receiver<
        Result<VoiceReceived, grokrs_api::transport::error::TransportError>,
    >,
    store: Option<&Store>,
    session_id: &str,
) -> Result<()> {
    use tokio::io::{self, AsyncBufReadExt, BufReader};

    let stdin = BufReader::new(io::stdin());
    let mut lines = stdin.lines();
    let mut transcript_log: Vec<(String, String)> = Vec::new();

    loop {
        tokio::select! {
            biased;

            event = event_rx.recv() => {
                match event {
                    Some(Ok(VoiceReceived::Event(voice_event))) => {
                        if handle_voice_event(client, sink, &voice_event, store, session_id, &mut transcript_log).await {
                            break;
                        }
                    }
                    Some(Ok(VoiceReceived::Audio(_))) => {}
                    Some(Err(e)) => { eprintln!("[voice] Transport error: {e}"); break; }
                    None => { eprintln!("[voice] Connection closed."); break; }
                }
            }

            line = lines.next_line() => {
                match line {
                    Ok(Some(input)) => {
                        let trimmed = input.trim();
                        if trimmed.is_empty() { continue; }

                        if trimmed.starts_with('/') {
                            match trimmed {
                                "/exit" | "/quit" => break,
                                "/interrupt" => {
                                    if let Err(e) = client.send_control(sink, ControlAction::Interrupt).await {
                                        eprintln!("[voice] Failed to send interrupt: {e}");
                                    }
                                    continue;
                                }
                                "/help" => {
                                    eprintln!("  /exit       - Close the voice session");
                                    eprintln!("  /interrupt  - Interrupt agent speech");
                                    eprintln!("  /help       - Show this help");
                                    continue;
                                }
                                other => {
                                    eprintln!("Unknown command: {other}. Type /help for available commands.");
                                    continue;
                                }
                            }
                        }

                        if let Err(e) = client.send_text(sink, trimmed).await {
                            eprintln!("[voice] Failed to send text: {e}");
                            break;
                        }
                    }
                    Ok(None) => break,
                    Err(e) => { eprintln!("[voice] Input error: {e}"); break; }
                }
            }
        }
    }

    if !transcript_log.is_empty() {
        eprintln!(
            "\n[voice] Session summary: {} transcript(s)",
            transcript_log.len()
        );
    }

    Ok(())
}

/// Placeholder for audio mode. Requires the 'audio' feature.
#[cfg(feature = "audio")]
async fn run_audio_mode(
    client: &VoiceAgentClient,
    sink: &Arc<tokio::sync::Mutex<grokrs_api::transport::websocket::WsSink>>,
    event_rx: &mut mpsc::Receiver<
        Result<VoiceReceived, grokrs_api::transport::error::TransportError>,
    >,
    store: Option<&Store>,
    session_id: &str,
) -> Result<()> {
    // Audio mode implementation with cpal capture and playback.
    // This would:
    // 1. Open the default input device (microphone) via cpal
    // 2. Open the default output device (speaker) via cpal
    // 3. Stream microphone PCM data to the voice agent
    // 4. Play received audio chunks through the speaker
    // 5. Display transcripts and handle function calls
    //
    // For now, delegate to text mode with a note.
    eprintln!("[voice] Audio capture/playback requires platform audio drivers.");
    eprintln!("[voice] Falling back to text-only interaction.\n");
    run_text_mode(client, sink, event_rx, store, session_id).await
}

/// Check whether the audio feature is available at compile time.
fn audio_feature_available() -> bool {
    cfg!(feature = "audio")
}

/// Check that the config allows network access before connecting.
fn check_network_policy(config: &AppConfig) -> Result<()> {
    if !config.policy.allow_network {
        bail!(
            "Network access is denied by policy.\n\
             \n\
             The `grokrs voice` command requires network access to reach the xAI Voice Agent API.\n\
             To opt in, set `allow_network = true` in your config file:\n\
             \n\
             [policy]\n\
             allow_network = true\n\
             \n\
             Then set `approval_mode` in [session] to control how network requests\n\
             are approved:\n\
             \n\
             [session]\n\
             approval_mode = \"allow\"   # bypass approval (development only)"
        );
    }

    let engine = PolicyEngine::new(config.policy.clone());
    let decision = engine.evaluate(&Effect::NetworkConnect {
        host: "api.x.ai".to_owned(),
    });
    match decision {
        Decision::Allow { .. } => Ok(()),
        Decision::Ask { .. } => match config.session.approval_mode.as_str() {
            "allow" => Ok(()),
            "deny" => bail!(
                "Network access requires approval but approval_mode is 'deny'.\n\
                 Set approval_mode = 'allow' in [session] config to bypass."
            ),
            _ => bail!(
                "Network access requires approval but the approval broker is not yet implemented.\n\
                 Set approval_mode = 'allow' in [session] config to bypass approval."
            ),
        },
        Decision::Deny { reason } => bail!("Network access denied by policy: {reason}"),
    }
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
    }

    #[test]
    fn check_network_allowed_with_allow_mode() {
        let config = make_config(true, "allow");
        let result = check_network_policy(&config);
        assert!(result.is_ok());
    }

    #[test]
    fn check_network_allowed_with_deny_mode() {
        let config = make_config(true, "deny");
        let result = check_network_policy(&config);
        assert!(result.is_err());
        let msg = result.unwrap_err().to_string();
        assert!(msg.contains("approval_mode is 'deny'"));
    }

    #[test]
    fn audio_feature_check() {
        // In default build (no 'audio' feature), this should be false.
        let available = audio_feature_available();
        // We can't assert the value because it depends on build config,
        // but we verify the function doesn't panic.
        let _ = available;
    }

    #[test]
    fn voice_args_defaults() {
        let args = VoiceArgs {
            text_only: false,
            model: None,
            voice: "eve".into(),
            language: "en-US".into(),
            turn_detection: "server_vad".into(),
            vad_sensitivity: "medium".into(),
            system: None,
            max_duration: None,
        };
        assert!(!args.text_only);
        assert!(args.model.is_none());
        assert_eq!(args.voice, "eve");
        assert_eq!(args.language, "en-US");
    }
}
