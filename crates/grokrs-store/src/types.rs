//! Plain data types returned by store queries.
//!
//! These structs are value objects representing database rows or aggregation
//! results. They carry no behaviour and do not depend on any other grokrs crate
//! beyond `grokrs-core` (for `StoreConfig`).

/// A session record as stored in the `sessions` table.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SessionRecord {
    /// Caller-provided session identifier.
    pub id: String,
    /// Trust level as a string: `"Untrusted"`, `"InteractiveTrusted"`, or `"AdminTrusted"`.
    pub trust_level: String,
    /// Session state as a string matching `SessionState` variant names.
    /// Failed sessions store `"Failed: <message>"`.
    pub state: String,
    /// RFC 3339 timestamp of session creation.
    pub created_at: String,
    /// RFC 3339 timestamp of the most recent state change.
    pub updated_at: String,
}

/// A transcript record as stored in the `transcripts` table.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TranscriptRecord {
    /// Auto-incremented transcript identifier.
    pub id: i64,
    /// Session this transcript belongs to.
    pub session_id: String,
    /// RFC 3339 timestamp when the request was sent.
    pub request_at: String,
    /// RFC 3339 timestamp when the response was received (if any).
    pub response_at: Option<String>,
    /// API endpoint that was called.
    pub endpoint: String,
    /// HTTP method used.
    pub method: String,
    /// Raw request body (may be None if not captured).
    pub request_body: Option<String>,
    /// Raw response body (filled by `log_response`).
    pub response_body: Option<String>,
    /// HTTP status code (filled by `log_response`).
    pub status_code: Option<i32>,
    /// Cost in integer ticks (xAI convention).
    pub cost_in_usd_ticks: Option<i64>,
    /// Number of input tokens consumed.
    pub input_tokens: Option<u64>,
    /// Number of output tokens generated.
    pub output_tokens: Option<u64>,
    /// Number of reasoning tokens consumed.
    pub reasoning_tokens: Option<u64>,
    /// Error message if the request failed.
    pub error: Option<String>,
    /// Response ID for stateful session resume (e.g., `previous_response_id`).
    pub response_id: Option<String>,
}

/// Usage data passed to `log_response` to record token counts and cost.
///
/// All fields are optional because not every API response includes usage data.
/// Token counts use `u64` because they are always non-negative. Cost uses `i64`
/// to allow for credits or adjustments (which could theoretically be negative).
#[derive(Debug, Clone, Default)]
pub struct TranscriptUsage {
    /// Cost in integer ticks (xAI convention).
    pub cost_in_usd_ticks: Option<i64>,
    /// Number of input tokens consumed.
    pub input_tokens: Option<u64>,
    /// Number of output tokens generated.
    pub output_tokens: Option<u64>,
    /// Number of reasoning tokens consumed.
    pub reasoning_tokens: Option<u64>,
}

/// Aggregated usage summary for one or all sessions.
///
/// Returned by `session_totals` and `all_totals`. All token counts use `u64`
/// because they are always non-negative. Cost uses `i64` to allow for credits
/// or adjustments (which could theoretically be negative).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct UsageSummary {
    /// Total cost across all included transcripts, in integer ticks.
    pub total_cost_ticks: i64,
    /// Total input tokens across all included transcripts.
    pub total_input_tokens: u64,
    /// Total output tokens across all included transcripts.
    pub total_output_tokens: u64,
    /// Total reasoning tokens across all included transcripts.
    pub total_reasoning_tokens: u64,
    /// Number of transcript records included in the aggregation.
    pub request_count: u64,
}
