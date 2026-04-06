//! ChatBackend trait and response types for the REPL.
//!
//! The backend trait decouples the REPL loop from any concrete API client,
//! making it testable with mock backends and swappable for different providers.

use std::fmt;

use crate::commands::search::Citation;

/// Token usage statistics for a single turn.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct TokenUsage {
    /// Number of tokens in the prompt/input.
    pub input_tokens: u64,
    /// Number of tokens in the completion/output.
    pub output_tokens: u64,
    /// Number of prompt tokens served from the prompt cache, if any.
    ///
    /// Present when `prompt_cache_key` was set on the request and the server
    /// returned a cache hit in `usage.prompt_tokens_details.cached_tokens`.
    /// `None` when no cache key was used or when the server did not report it.
    pub cached_tokens: Option<u64>,
}

impl TokenUsage {
    /// Total tokens consumed in this turn.
    pub fn total(&self) -> u64 {
        self.input_tokens + self.output_tokens
    }
}

impl fmt::Display for TokenUsage {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self.cached_tokens {
            Some(cached) if cached > 0 => write!(
                f,
                "input={} ({cached} cached), output={}, total={}",
                self.input_tokens,
                self.output_tokens,
                self.total()
            ),
            _ => write!(
                f,
                "input={}, output={}, total={}",
                self.input_tokens,
                self.output_tokens,
                self.total()
            ),
        }
    }
}

/// Response from the chat backend for a single turn.
#[derive(Debug, Clone)]
pub struct ChatResponse {
    /// The full response text.
    pub text: String,
    /// Token usage for this turn.
    pub usage: TokenUsage,
    /// Optional response ID for stateful chaining (server-side conversation).
    pub previous_response_id: Option<String>,
    /// Citations extracted from search results, if any.
    pub citations: Vec<Citation>,
}

/// Trait that decouples the REPL from the API client.
///
/// Implementations handle the actual network communication. The REPL only
/// knows about this trait, never about concrete API types.
///
/// The trait is object-safe via `Send` bound. Concrete backends (e.g.
/// `GrokChatBackend` from U13) implement this to wire the REPL to an API.
pub trait ChatBackend: Send {
    /// Send a user message and return the assistant response.
    ///
    /// Implementations may stream tokens internally (e.g. printing to stdout
    /// as they arrive) before returning the accumulated response.
    fn send_message(
        &mut self,
        message: &str,
    ) -> impl std::future::Future<Output = Result<ChatResponse, BackendError>> + Send;

    /// Get the current model name.
    fn model(&self) -> &str;

    /// Switch to a different model for subsequent turns.
    fn set_model(&mut self, model: &str);

    /// Set system instructions for subsequent turns.
    fn set_system(&mut self, instructions: &str);

    /// Clear conversation history and any server-side state references.
    fn clear(&mut self);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn token_usage_total_is_input_plus_output() {
        let u = TokenUsage {
            input_tokens: 100,
            output_tokens: 50,
            cached_tokens: None,
        };
        assert_eq!(u.total(), 150);
    }

    #[test]
    fn token_usage_display_no_cached_tokens() {
        let u = TokenUsage {
            input_tokens: 100,
            output_tokens: 50,
            cached_tokens: None,
        };
        let s = u.to_string();
        assert_eq!(s, "input=100, output=50, total=150");
        assert!(!s.contains("cached"));
    }

    #[test]
    fn token_usage_display_zero_cached_tokens_omits_cached() {
        // cached=0 should NOT show the "(0 cached)" annotation.
        let u = TokenUsage {
            input_tokens: 100,
            output_tokens: 50,
            cached_tokens: Some(0),
        };
        let s = u.to_string();
        assert_eq!(s, "input=100, output=50, total=150");
        assert!(!s.contains("cached"));
    }

    #[test]
    fn token_usage_display_with_cached_tokens() {
        let u = TokenUsage {
            input_tokens: 500,
            output_tokens: 100,
            cached_tokens: Some(200),
        };
        let s = u.to_string();
        assert_eq!(s, "input=500 (200 cached), output=100, total=600");
    }

    #[test]
    fn token_usage_default_has_none_cached() {
        let u = TokenUsage::default();
        assert_eq!(u.input_tokens, 0);
        assert_eq!(u.output_tokens, 0);
        assert!(u.cached_tokens.is_none());
    }

    #[test]
    fn token_usage_equality_considers_cached_tokens() {
        let a = TokenUsage {
            input_tokens: 10,
            output_tokens: 5,
            cached_tokens: Some(3),
        };
        let b = TokenUsage {
            input_tokens: 10,
            output_tokens: 5,
            cached_tokens: None,
        };
        assert_ne!(a, b);
        assert_eq!(a, a.clone());
    }
}

/// Errors from a chat backend.
#[derive(Debug, thiserror::Error)]
pub enum BackendError {
    /// Network or transport error.
    #[error("transport error: {0}")]
    Transport(String),

    /// API returned an error response.
    #[error("API error ({status}): {message}")]
    Api {
        /// HTTP status code or equivalent.
        status: u16,
        /// Error message from the API.
        message: String,
    },

    /// The request was cancelled (e.g. by Ctrl-C during streaming).
    #[error("request cancelled")]
    Cancelled,

    /// Any other error.
    #[error("{0}")]
    Other(#[from] anyhow::Error),
}
