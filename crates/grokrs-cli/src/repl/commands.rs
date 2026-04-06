//! Slash command parsing for the REPL.
//!
//! Slash commands are single-line directives prefixed with `/` that control
//! the REPL itself rather than sending messages to the backend. Command names
//! are case-insensitive per the U12 spec.

/// A parsed slash command.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SlashCommand {
    /// Exit the REPL (`/exit` or `/quit`).
    Exit,
    /// Clear conversation history (`/clear`).
    Clear,
    /// Switch model for subsequent turns (`/model <name>`).
    Model(String),
    /// Set system instructions (`/system <prompt>`).
    System(String),
    /// Display available slash commands (`/help`).
    Help,
    /// Show conversation turn count and cumulative token usage (`/history`).
    History,
    /// Unrecognized slash command.
    Unknown(String),
}

/// Attempt to parse a line as a slash command.
///
/// Returns `Some(SlashCommand)` if the line starts with `/`, otherwise `None`
/// (meaning the line is a regular user message).
pub fn parse(input: &str) -> Option<SlashCommand> {
    let trimmed = input.trim();
    if !trimmed.starts_with('/') {
        return None;
    }

    // Split into command name and the rest of the line.
    let (cmd, rest) = match trimmed.find(char::is_whitespace) {
        Some(pos) => (&trimmed[..pos], trimmed[pos..].trim()),
        None => (trimmed, ""),
    };

    // Case-insensitive matching: lowercase the command portion only.
    let cmd_lower = cmd.to_ascii_lowercase();

    let slash_cmd = match cmd_lower.as_str() {
        "/exit" | "/quit" => SlashCommand::Exit,
        "/clear" => SlashCommand::Clear,
        "/model" => {
            if rest.is_empty() {
                // No model name provided -- treat as unknown so the user gets feedback.
                SlashCommand::Unknown(format!("{cmd} (missing model name; usage: /model <name>)"))
            } else {
                // Take the first whitespace-delimited token as the model name.
                let model_name = rest.split_whitespace().next().unwrap_or(rest);
                SlashCommand::Model(model_name.to_owned())
            }
        }
        "/system" => {
            if rest.is_empty() {
                SlashCommand::Unknown(format!("{cmd} (missing prompt; usage: /system <prompt>)"))
            } else {
                SlashCommand::System(rest.to_owned())
            }
        }
        "/help" => SlashCommand::Help,
        "/history" => SlashCommand::History,
        _ => SlashCommand::Unknown(cmd.to_owned()),
    };

    Some(slash_cmd)
}

/// Format the `/help` output listing all available slash commands.
#[must_use]
pub fn help_text() -> &'static str {
    "\
Available commands:
  /help              Show this help message
  /exit, /quit       Exit the REPL
  /clear             Clear conversation history
  /model <name>      Switch model for subsequent turns
  /system <prompt>   Set system instructions
  /history           Show turn count and cumulative token usage"
}

#[cfg(test)]
mod tests {
    use super::*;

    // -----------------------------------------------------------------------
    // Basic slash command parsing
    // -----------------------------------------------------------------------

    #[test]
    fn parse_exit() {
        assert_eq!(parse("/exit"), Some(SlashCommand::Exit));
    }

    #[test]
    fn parse_quit() {
        assert_eq!(parse("/quit"), Some(SlashCommand::Exit));
    }

    #[test]
    fn parse_clear() {
        assert_eq!(parse("/clear"), Some(SlashCommand::Clear));
    }

    #[test]
    fn parse_help() {
        assert_eq!(parse("/help"), Some(SlashCommand::Help));
    }

    #[test]
    fn parse_history() {
        assert_eq!(parse("/history"), Some(SlashCommand::History));
    }

    #[test]
    fn parse_model_with_name() {
        assert_eq!(
            parse("/model grok-4-mini"),
            Some(SlashCommand::Model("grok-4-mini".to_owned()))
        );
    }

    #[test]
    fn parse_model_without_name_is_unknown() {
        match parse("/model") {
            Some(SlashCommand::Unknown(msg)) => {
                assert!(msg.contains("missing model name"), "got: {msg}");
            }
            other => panic!("expected Unknown, got {other:?}"),
        }
    }

    #[test]
    fn parse_system_with_prompt() {
        assert_eq!(
            parse("/system You are a Rust expert"),
            Some(SlashCommand::System("You are a Rust expert".to_owned()))
        );
    }

    #[test]
    fn parse_system_without_prompt_is_unknown() {
        match parse("/system") {
            Some(SlashCommand::Unknown(msg)) => {
                assert!(msg.contains("missing prompt"), "got: {msg}");
            }
            other => panic!("expected Unknown, got {other:?}"),
        }
    }

    // -----------------------------------------------------------------------
    // Case insensitivity
    // -----------------------------------------------------------------------

    #[test]
    fn parse_case_insensitive() {
        assert_eq!(parse("/EXIT"), Some(SlashCommand::Exit));
        assert_eq!(parse("/Quit"), Some(SlashCommand::Exit));
        assert_eq!(parse("/CLEAR"), Some(SlashCommand::Clear));
        assert_eq!(parse("/Help"), Some(SlashCommand::Help));
        assert_eq!(parse("/HISTORY"), Some(SlashCommand::History));
        assert_eq!(
            parse("/MODEL grok-4"),
            Some(SlashCommand::Model("grok-4".to_owned()))
        );
        assert_eq!(
            parse("/SYSTEM be brief"),
            Some(SlashCommand::System("be brief".to_owned()))
        );
    }

    // -----------------------------------------------------------------------
    // Unknown commands
    // -----------------------------------------------------------------------

    #[test]
    fn parse_unknown_command() {
        assert_eq!(
            parse("/foobar"),
            Some(SlashCommand::Unknown("/foobar".to_owned()))
        );
    }

    // -----------------------------------------------------------------------
    // Non-commands (regular user input)
    // -----------------------------------------------------------------------

    #[test]
    fn non_slash_input_returns_none() {
        assert_eq!(parse("hello world"), None);
    }

    #[test]
    fn empty_input_returns_none() {
        assert_eq!(parse(""), None);
    }

    #[test]
    fn whitespace_only_returns_none() {
        assert_eq!(parse("   "), None);
    }

    // -----------------------------------------------------------------------
    // Leading/trailing whitespace
    // -----------------------------------------------------------------------

    #[test]
    fn parse_with_leading_whitespace() {
        assert_eq!(parse("  /exit"), Some(SlashCommand::Exit));
    }

    #[test]
    fn parse_with_trailing_whitespace() {
        assert_eq!(parse("/exit  "), Some(SlashCommand::Exit));
    }

    // -----------------------------------------------------------------------
    // Model name takes only first token
    // -----------------------------------------------------------------------

    #[test]
    fn model_takes_first_token_only() {
        assert_eq!(
            parse("/model grok-4-mini extra stuff"),
            Some(SlashCommand::Model("grok-4-mini".to_owned()))
        );
    }

    // -----------------------------------------------------------------------
    // Help text smoke test
    // -----------------------------------------------------------------------

    #[test]
    fn help_text_contains_all_commands() {
        let text = help_text();
        assert!(text.contains("/help"));
        assert!(text.contains("/exit"));
        assert!(text.contains("/quit"));
        assert!(text.contains("/clear"));
        assert!(text.contains("/model"));
        assert!(text.contains("/system"));
        assert!(text.contains("/history"));
    }
}
