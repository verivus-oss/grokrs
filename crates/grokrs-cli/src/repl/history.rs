//! Conversation history tracking for the REPL.
//!
//! Maintains an in-memory record of conversation turns with cumulative
//! token usage. This is distinct from rustyline's readline history --
//! this tracks the actual conversation content and API usage.

use super::backend::TokenUsage;
use std::fmt;

/// A single conversation turn: one user message and the assistant response.
#[derive(Debug, Clone)]
pub struct ConversationTurn {
    /// The user's input message.
    pub user_input: String,
    /// The assistant's response text.
    pub assistant_response: String,
    /// Token usage for this turn.
    pub usage: TokenUsage,
}

/// In-memory conversation history.
#[derive(Debug, Default)]
pub struct ConversationHistory {
    /// All turns in chronological order.
    turns: Vec<ConversationTurn>,
    /// Cumulative input tokens across all turns.
    total_input_tokens: u64,
    /// Cumulative output tokens across all turns.
    total_output_tokens: u64,
    /// Optional response ID from the last turn (for stateful chaining).
    last_response_id: Option<String>,
}

impl ConversationHistory {
    /// Create an empty conversation history.
    pub fn new() -> Self {
        Self::default()
    }

    /// Record a completed conversation turn.
    pub fn push(&mut self, turn: ConversationTurn) {
        self.total_input_tokens += turn.usage.input_tokens;
        self.total_output_tokens += turn.usage.output_tokens;
        self.turns.push(turn);
    }

    /// Set the last response ID for stateful chaining.
    pub fn set_last_response_id(&mut self, id: Option<String>) {
        self.last_response_id = id;
    }

    /// Get the last response ID for stateful chaining.
    pub fn last_response_id(&self) -> Option<&str> {
        self.last_response_id.as_deref()
    }

    /// Number of completed turns.
    pub fn turn_count(&self) -> usize {
        self.turns.len()
    }

    /// Cumulative input tokens.
    pub fn total_input_tokens(&self) -> u64 {
        self.total_input_tokens
    }

    /// Cumulative output tokens.
    pub fn total_output_tokens(&self) -> u64 {
        self.total_output_tokens
    }

    /// Cumulative total tokens.
    pub fn total_tokens(&self) -> u64 {
        self.total_input_tokens + self.total_output_tokens
    }

    /// Immutable access to all turns.
    pub fn turns(&self) -> &[ConversationTurn] {
        &self.turns
    }

    /// Clear all conversation state.
    pub fn clear(&mut self) {
        self.turns.clear();
        self.total_input_tokens = 0;
        self.total_output_tokens = 0;
        self.last_response_id = None;
    }
}

impl fmt::Display for ConversationHistory {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "turns={}, input_tokens={}, output_tokens={}, total_tokens={}",
            self.turn_count(),
            self.total_input_tokens,
            self.total_output_tokens,
            self.total_tokens()
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_turn(input: &str, response: &str, input_tok: u64, output_tok: u64) -> ConversationTurn {
        ConversationTurn {
            user_input: input.to_owned(),
            assistant_response: response.to_owned(),
            usage: TokenUsage {
                input_tokens: input_tok,
                output_tokens: output_tok,
                cached_tokens: None,
            },
        }
    }

    #[test]
    fn empty_history() {
        let h = ConversationHistory::new();
        assert_eq!(h.turn_count(), 0);
        assert_eq!(h.total_tokens(), 0);
        assert!(h.last_response_id().is_none());
    }

    #[test]
    fn push_accumulates_usage() {
        let mut h = ConversationHistory::new();
        h.push(make_turn("hi", "hello", 10, 5));
        h.push(make_turn("how?", "fine", 20, 15));

        assert_eq!(h.turn_count(), 2);
        assert_eq!(h.total_input_tokens(), 30);
        assert_eq!(h.total_output_tokens(), 20);
        assert_eq!(h.total_tokens(), 50);
    }

    #[test]
    fn clear_resets_everything() {
        let mut h = ConversationHistory::new();
        h.push(make_turn("a", "b", 5, 3));
        h.set_last_response_id(Some("resp-123".to_owned()));

        h.clear();
        assert_eq!(h.turn_count(), 0);
        assert_eq!(h.total_tokens(), 0);
        assert!(h.last_response_id().is_none());
    }

    #[test]
    fn response_id_tracking() {
        let mut h = ConversationHistory::new();
        assert!(h.last_response_id().is_none());

        h.set_last_response_id(Some("resp-abc".to_owned()));
        assert_eq!(h.last_response_id(), Some("resp-abc"));

        h.set_last_response_id(None);
        assert!(h.last_response_id().is_none());
    }

    #[test]
    fn display_format() {
        let mut h = ConversationHistory::new();
        h.push(make_turn("x", "y", 100, 50));
        let s = h.to_string();
        assert!(s.contains("turns=1"));
        assert!(s.contains("input_tokens=100"));
        assert!(s.contains("output_tokens=50"));
        assert!(s.contains("total_tokens=150"));
    }

    #[test]
    fn turns_accessor() {
        let mut h = ConversationHistory::new();
        h.push(make_turn("q1", "a1", 1, 1));
        h.push(make_turn("q2", "a2", 2, 2));

        let turns = h.turns();
        assert_eq!(turns.len(), 2);
        assert_eq!(turns[0].user_input, "q1");
        assert_eq!(turns[1].user_input, "q2");
    }
}
