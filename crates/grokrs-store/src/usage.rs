//! Usage accumulation: session_totals, all_totals.
//!
//! `UsageRepo` is a borrowed handle into the `Store`'s connection. It provides
//! SQL-level aggregation of token counts and cost from the `transcripts` table.
//! NULL values are treated as zero via `COALESCE`.

use rusqlite::{Connection, params};

use crate::StoreError;
use crate::types::UsageSummary;

/// Safely convert a non-negative `i64` (as returned by SQLite SUM) to `u64`.
///
/// Returns `NegativeTokenCount` if the value is negative, which should never
/// happen for token counts written through our API (since `TranscriptUsage`
/// stores `u64`). This guard catches data corruption or direct SQL tampering.
fn i64_to_u64(value: i64, column: &'static str) -> Result<u64, StoreError> {
    u64::try_from(value).map_err(|_| StoreError::NegativeTokenCount { column, value })
}

/// Borrowed handle for usage aggregation queries.
pub struct UsageRepo<'a> {
    conn: &'a Connection,
}

impl<'a> UsageRepo<'a> {
    pub(crate) fn new(conn: &'a Connection) -> Self {
        Self { conn }
    }

    /// Return aggregated usage for a single session.
    ///
    /// If the session has no transcripts, returns a zero-valued `UsageSummary`.
    /// Token counts are validated as non-negative when converting from SQLite's
    /// signed `i64` representation.
    pub fn session_totals(&self, session_id: &str) -> Result<UsageSummary, StoreError> {
        let (cost, input, output, reasoning, count) = self
            .conn
            .query_row(
                "SELECT \
                 COALESCE(SUM(COALESCE(cost_in_usd_ticks, 0)), 0), \
                 COALESCE(SUM(COALESCE(input_tokens, 0)), 0), \
                 COALESCE(SUM(COALESCE(output_tokens, 0)), 0), \
                 COALESCE(SUM(COALESCE(reasoning_tokens, 0)), 0), \
                 COUNT(*) \
                 FROM transcripts WHERE session_id = ?1",
                params![session_id],
                |row| {
                    Ok((
                        row.get::<_, i64>(0)?,
                        row.get::<_, i64>(1)?,
                        row.get::<_, i64>(2)?,
                        row.get::<_, i64>(3)?,
                        row.get::<_, i64>(4)?,
                    ))
                },
            )
            .map_err(StoreError::Sql)?;

        Ok(UsageSummary {
            total_cost_ticks: cost,
            total_input_tokens: i64_to_u64(input, "total_input_tokens")?,
            total_output_tokens: i64_to_u64(output, "total_output_tokens")?,
            total_reasoning_tokens: i64_to_u64(reasoning, "total_reasoning_tokens")?,
            request_count: i64_to_u64(count, "request_count")?,
        })
    }

    /// Return aggregated usage across all sessions.
    ///
    /// If the database has no transcripts, returns a zero-valued `UsageSummary`.
    /// Token counts are validated as non-negative when converting from SQLite's
    /// signed `i64` representation.
    pub fn all_totals(&self) -> Result<UsageSummary, StoreError> {
        let (cost, input, output, reasoning, count) = self
            .conn
            .query_row(
                "SELECT \
                 COALESCE(SUM(COALESCE(cost_in_usd_ticks, 0)), 0), \
                 COALESCE(SUM(COALESCE(input_tokens, 0)), 0), \
                 COALESCE(SUM(COALESCE(output_tokens, 0)), 0), \
                 COALESCE(SUM(COALESCE(reasoning_tokens, 0)), 0), \
                 COUNT(*) \
                 FROM transcripts",
                [],
                |row| {
                    Ok((
                        row.get::<_, i64>(0)?,
                        row.get::<_, i64>(1)?,
                        row.get::<_, i64>(2)?,
                        row.get::<_, i64>(3)?,
                        row.get::<_, i64>(4)?,
                    ))
                },
            )
            .map_err(StoreError::Sql)?;

        Ok(UsageSummary {
            total_cost_ticks: cost,
            total_input_tokens: i64_to_u64(input, "total_input_tokens")?,
            total_output_tokens: i64_to_u64(output, "total_output_tokens")?,
            total_reasoning_tokens: i64_to_u64(reasoning, "total_reasoning_tokens")?,
            request_count: i64_to_u64(count, "request_count")?,
        })
    }
}
