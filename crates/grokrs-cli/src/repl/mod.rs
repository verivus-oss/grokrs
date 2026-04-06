//! Interactive REPL (Read-Eval-Print Loop) for grokrs.
//!
//! Provides readline-based input with persistent history, slash command
//! parsing, and a pluggable [`ChatBackend`] trait for API communication.
//! The REPL core has no dependency on `grokrs-api` -- it only knows about
//! the backend trait, making it fully testable with mock implementations.

pub mod backend;
pub mod commands;
pub mod grok_backend;
pub mod history;

use backend::{BackendError, ChatBackend};
use commands::SlashCommand;
use history::{ConversationHistory, ConversationTurn};
use rustyline::DefaultEditor;
use rustyline::error::ReadlineError;
use std::path::PathBuf;

/// The default REPL prompt.
pub const PROMPT: &str = "grokrs> ";

/// Default history file name, stored under the `.grokrs/` workspace directory.
const HISTORY_FILENAME: &str = "chat_history.txt";

/// Configuration for the REPL loop.
#[derive(Debug, Clone)]
pub struct ReplConfig {
    /// Directory where `.grokrs/` state lives (typically the workspace root).
    pub state_dir: PathBuf,
}

impl ReplConfig {
    /// Resolve the history file path: `<state_dir>/.grokrs/chat_history.txt`.
    pub fn history_path(&self) -> PathBuf {
        self.state_dir.join(".grokrs").join(HISTORY_FILENAME)
    }
}

/// Outcome of processing a single line of REPL input.
#[derive(Debug, PartialEq, Eq)]
pub enum LineOutcome {
    /// Continue the REPL loop.
    Continue,
    /// Exit the REPL.
    Exit,
}

/// Process a single line of input against the REPL state.
///
/// This is the core dispatch function, extracted so it can be unit-tested
/// without an actual terminal. Returns [`LineOutcome`] to signal whether
/// the REPL should continue or exit.
pub async fn process_line<B: ChatBackend>(
    line: &str,
    backend: &mut B,
    conversation: &mut ConversationHistory,
    output: &mut dyn std::io::Write,
) -> LineOutcome {
    let trimmed = line.trim();

    // Ignore empty input.
    if trimmed.is_empty() {
        return LineOutcome::Continue;
    }

    // Check for slash commands.
    if let Some(cmd) = commands::parse(trimmed) {
        return handle_slash_command(cmd, backend, conversation, output);
    }

    // Regular user message -- send to backend.
    match backend.send_message(trimmed).await {
        Ok(response) => {
            // Print the response text.
            let _ = writeln!(output, "{}", response.text);

            // Record the turn.
            let turn = ConversationTurn {
                user_input: trimmed.to_owned(),
                assistant_response: response.text.clone(),
                usage: response.usage,
            };
            conversation.push(turn);

            // Track response ID for stateful chaining.
            conversation.set_last_response_id(response.previous_response_id);
        }
        Err(BackendError::Cancelled) => {
            let _ = writeln!(output, "[request cancelled]");
        }
        Err(err) => {
            let _ = writeln!(output, "error: {err}");
        }
    }

    LineOutcome::Continue
}

/// Handle a parsed slash command.
fn handle_slash_command<B: ChatBackend>(
    cmd: SlashCommand,
    backend: &mut B,
    conversation: &mut ConversationHistory,
    output: &mut dyn std::io::Write,
) -> LineOutcome {
    match cmd {
        SlashCommand::Exit => LineOutcome::Exit,

        SlashCommand::Clear => {
            backend.clear();
            conversation.clear();
            let _ = writeln!(output, "Conversation cleared.");
            LineOutcome::Continue
        }

        SlashCommand::Model(name) => {
            backend.set_model(&name);
            let _ = writeln!(output, "Model switched to: {name}");
            LineOutcome::Continue
        }

        SlashCommand::System(instructions) => {
            backend.set_system(&instructions);
            let _ = writeln!(output, "System instructions updated.");
            LineOutcome::Continue
        }

        SlashCommand::Help => {
            let _ = writeln!(output, "{}", commands::help_text());
            LineOutcome::Continue
        }

        SlashCommand::History => {
            let _ = writeln!(output, "{conversation}");
            LineOutcome::Continue
        }

        SlashCommand::Unknown(name) => {
            let _ = writeln!(
                output,
                "Unknown command: {name}. Type /help for available commands."
            );
            LineOutcome::Continue
        }
    }
}

/// Run the interactive REPL loop.
///
/// This is the top-level entry point for the interactive session. It creates
/// a rustyline editor, loads history, enters the read-eval-print loop, and
/// saves history on exit.
///
/// # Arguments
///
/// * `backend` - The chat backend to send messages to.
/// * `config` - REPL configuration (state directory for history persistence).
/// * `rt` - A tokio runtime handle for executing async backend calls.
pub fn run_repl<B: ChatBackend>(
    mut backend: B,
    config: &ReplConfig,
    rt: &tokio::runtime::Handle,
) -> anyhow::Result<()> {
    let mut editor = DefaultEditor::new()?;

    // Ensure the .grokrs directory exists for history persistence.
    let history_path = config.history_path();
    if let Some(parent) = history_path.parent() {
        std::fs::create_dir_all(parent).ok();
    }

    // Load existing history (best-effort -- corruption is non-fatal).
    if history_path.exists()
        && let Err(e) = editor.load_history(&history_path)
    {
        eprintln!(
            "warning: could not load history from {}: {e}",
            history_path.display()
        );
    }

    let mut conversation = ConversationHistory::new();
    let mut stdout = std::io::stdout();

    loop {
        match editor.readline(PROMPT) {
            Ok(line) => {
                // Add non-empty lines to readline history.
                let trimmed = line.trim();
                if !trimmed.is_empty() {
                    let _ = editor.add_history_entry(trimmed);
                }

                let outcome = rt.block_on(process_line(
                    &line,
                    &mut backend,
                    &mut conversation,
                    &mut stdout,
                ));

                if outcome == LineOutcome::Exit {
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

            // Other readline errors (e.g. terminal issues).
            Err(err) => {
                eprintln!("readline error: {err}");
                break;
            }
        }
    }

    // Print usage summary on exit.
    if conversation.turn_count() > 0 {
        eprintln!("Session summary: {conversation}");
    }

    // Save history (best-effort).
    if let Err(e) = editor.save_history(&history_path) {
        eprintln!(
            "warning: could not save history to {}: {e}",
            history_path.display()
        );
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use backend::{ChatResponse, TokenUsage};
    use std::sync::{Arc, Mutex};

    // -----------------------------------------------------------------------
    // Mock backend for testing
    // -----------------------------------------------------------------------

    #[derive(Clone)]
    struct MockBackend {
        model_name: String,
        system_instructions: Option<String>,
        /// Messages sent to the backend (for assertions).
        sent_messages: Arc<Mutex<Vec<String>>>,
        /// Canned response text.
        response_text: String,
        /// Canned token usage.
        response_usage: TokenUsage,
        /// Whether clear() was called.
        cleared: Arc<Mutex<bool>>,
        /// If set, send_message returns this error.
        error: Option<String>,
    }

    impl MockBackend {
        fn new(response_text: &str) -> Self {
            Self {
                model_name: "mock-model".to_owned(),
                system_instructions: None,
                sent_messages: Arc::new(Mutex::new(Vec::new())),
                response_text: response_text.to_owned(),
                response_usage: TokenUsage {
                    input_tokens: 10,
                    output_tokens: 20,
                    cached_tokens: None,
                },
                cleared: Arc::new(Mutex::new(false)),
                error: None,
            }
        }

        fn with_error(mut self, msg: &str) -> Self {
            self.error = Some(msg.to_owned());
            self
        }

        fn sent_messages(&self) -> Vec<String> {
            self.sent_messages.lock().unwrap().clone()
        }

        fn was_cleared(&self) -> bool {
            *self.cleared.lock().unwrap()
        }
    }

    impl ChatBackend for MockBackend {
        async fn send_message(&mut self, message: &str) -> Result<ChatResponse, BackendError> {
            self.sent_messages.lock().unwrap().push(message.to_owned());

            if let Some(ref err_msg) = self.error {
                return Err(BackendError::Transport(err_msg.clone()));
            }

            Ok(ChatResponse {
                text: self.response_text.clone(),
                usage: self.response_usage.clone(),
                previous_response_id: None,
                citations: Vec::new(),
            })
        }

        fn model(&self) -> &str {
            &self.model_name
        }

        fn set_model(&mut self, model: &str) {
            self.model_name = model.to_owned();
        }

        fn set_system(&mut self, instructions: &str) {
            self.system_instructions = Some(instructions.to_owned());
        }

        fn clear(&mut self) {
            *self.cleared.lock().unwrap() = true;
            self.system_instructions = None;
        }
    }

    // Helper to run async process_line in tests.
    fn run_process_line(
        line: &str,
        backend: &mut MockBackend,
        conversation: &mut ConversationHistory,
    ) -> (LineOutcome, String) {
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        let mut output = Vec::new();
        let outcome = rt.block_on(process_line(line, backend, conversation, &mut output));
        let output_str = String::from_utf8(output).unwrap();
        (outcome, output_str)
    }

    // -----------------------------------------------------------------------
    // Empty input
    // -----------------------------------------------------------------------

    #[test]
    fn empty_input_ignored() {
        let mut backend = MockBackend::new("response");
        let mut conv = ConversationHistory::new();
        let (outcome, output) = run_process_line("", &mut backend, &mut conv);
        assert_eq!(outcome, LineOutcome::Continue);
        assert!(output.is_empty());
        assert!(backend.sent_messages().is_empty());
    }

    #[test]
    fn whitespace_only_ignored() {
        let mut backend = MockBackend::new("response");
        let mut conv = ConversationHistory::new();
        let (outcome, _) = run_process_line("   ", &mut backend, &mut conv);
        assert_eq!(outcome, LineOutcome::Continue);
        assert!(backend.sent_messages().is_empty());
    }

    // -----------------------------------------------------------------------
    // Slash commands through process_line
    // -----------------------------------------------------------------------

    #[test]
    fn exit_command_returns_exit() {
        let mut backend = MockBackend::new("response");
        let mut conv = ConversationHistory::new();
        let (outcome, _) = run_process_line("/exit", &mut backend, &mut conv);
        assert_eq!(outcome, LineOutcome::Exit);
    }

    #[test]
    fn quit_command_returns_exit() {
        let mut backend = MockBackend::new("response");
        let mut conv = ConversationHistory::new();
        let (outcome, _) = run_process_line("/quit", &mut backend, &mut conv);
        assert_eq!(outcome, LineOutcome::Exit);
    }

    #[test]
    fn clear_command_clears_backend_and_history() {
        let mut backend = MockBackend::new("response");
        let mut conv = ConversationHistory::new();

        // Add a turn first.
        let _ = run_process_line("hello", &mut backend, &mut conv);
        assert_eq!(conv.turn_count(), 1);

        // Now clear.
        let (outcome, output) = run_process_line("/clear", &mut backend, &mut conv);
        assert_eq!(outcome, LineOutcome::Continue);
        assert!(output.contains("Conversation cleared"));
        assert_eq!(conv.turn_count(), 0);
        assert!(backend.was_cleared());
    }

    #[test]
    fn model_command_switches_model() {
        let mut backend = MockBackend::new("response");
        let mut conv = ConversationHistory::new();
        let (outcome, output) = run_process_line("/model grok-4-mini", &mut backend, &mut conv);
        assert_eq!(outcome, LineOutcome::Continue);
        assert!(output.contains("grok-4-mini"));
        assert_eq!(backend.model(), "grok-4-mini");
    }

    #[test]
    fn system_command_sets_instructions() {
        let mut backend = MockBackend::new("response");
        let mut conv = ConversationHistory::new();
        let (outcome, output) =
            run_process_line("/system You are a Rust expert", &mut backend, &mut conv);
        assert_eq!(outcome, LineOutcome::Continue);
        assert!(output.contains("System instructions updated"));
        assert_eq!(
            backend.system_instructions,
            Some("You are a Rust expert".to_owned())
        );
    }

    #[test]
    fn help_command_shows_commands() {
        let mut backend = MockBackend::new("response");
        let mut conv = ConversationHistory::new();
        let (_, output) = run_process_line("/help", &mut backend, &mut conv);
        assert!(output.contains("/exit"));
        assert!(output.contains("/clear"));
        assert!(output.contains("/model"));
        assert!(output.contains("/system"));
        assert!(output.contains("/history"));
    }

    #[test]
    fn history_command_shows_stats() {
        let mut backend = MockBackend::new("response");
        let mut conv = ConversationHistory::new();

        // Send a message first to have something in history.
        let _ = run_process_line("hello", &mut backend, &mut conv);

        let (_, output) = run_process_line("/history", &mut backend, &mut conv);
        assert!(output.contains("turns=1"));
        assert!(output.contains("input_tokens=10"));
        assert!(output.contains("output_tokens=20"));
    }

    #[test]
    fn unknown_command_reported() {
        let mut backend = MockBackend::new("response");
        let mut conv = ConversationHistory::new();
        let (outcome, output) = run_process_line("/foobar", &mut backend, &mut conv);
        assert_eq!(outcome, LineOutcome::Continue);
        assert!(output.contains("Unknown command"));
        assert!(output.contains("/foobar"));
    }

    // -----------------------------------------------------------------------
    // Regular messages
    // -----------------------------------------------------------------------

    #[test]
    fn regular_message_sent_to_backend() {
        let mut backend = MockBackend::new("Hello back!");
        let mut conv = ConversationHistory::new();
        let (outcome, output) = run_process_line("tell me a joke", &mut backend, &mut conv);
        assert_eq!(outcome, LineOutcome::Continue);
        assert!(output.contains("Hello back!"));
        assert_eq!(backend.sent_messages(), vec!["tell me a joke"]);
        assert_eq!(conv.turn_count(), 1);
    }

    #[test]
    fn multiple_turns_accumulate() {
        let mut backend = MockBackend::new("ok");
        let mut conv = ConversationHistory::new();

        let _ = run_process_line("first", &mut backend, &mut conv);
        let _ = run_process_line("second", &mut backend, &mut conv);
        let _ = run_process_line("third", &mut backend, &mut conv);

        assert_eq!(conv.turn_count(), 3);
        assert_eq!(conv.total_input_tokens(), 30); // 10 * 3
        assert_eq!(conv.total_output_tokens(), 60); // 20 * 3
    }

    // -----------------------------------------------------------------------
    // Error handling
    // -----------------------------------------------------------------------

    #[test]
    fn backend_error_displayed_and_continues() {
        let mut backend = MockBackend::new("ok").with_error("connection refused");
        let mut conv = ConversationHistory::new();
        let (outcome, output) = run_process_line("hello", &mut backend, &mut conv);
        assert_eq!(outcome, LineOutcome::Continue);
        assert!(output.contains("error:"));
        assert!(output.contains("connection refused"));
        // Error turns are not recorded in history.
        assert_eq!(conv.turn_count(), 0);
    }

    // -----------------------------------------------------------------------
    // ReplConfig
    // -----------------------------------------------------------------------

    #[test]
    fn repl_config_history_path() {
        let config = ReplConfig {
            state_dir: PathBuf::from("/workspace"),
        };
        assert_eq!(
            config.history_path(),
            PathBuf::from("/workspace/.grokrs/chat_history.txt")
        );
    }
}
