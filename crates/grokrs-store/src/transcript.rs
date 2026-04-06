//! API transcript logging: log_request, log_response, log_error, list_by_session.
//!
//! `TranscriptRepo` is a borrowed handle into the `Store`'s connection. It
//! implements a two-phase logging pattern: `log_request` records the outbound
//! request before the API call completes, then `log_response` or `log_error`
//! fills in the result after the response arrives.

use rusqlite::{Connection, params};

use crate::StoreError;
use crate::session::now;
use crate::types::{TranscriptRecord, TranscriptUsage};

/// Intermediate row read from SQLite with `i64` token columns, before
/// validation and conversion to the public `TranscriptRecord` with `u64`
/// token fields.
struct RawTranscriptRow {
    id: i64,
    session_id: String,
    request_at: String,
    response_at: Option<String>,
    endpoint: String,
    method: String,
    request_body: Option<String>,
    response_body: Option<String>,
    status_code: Option<i32>,
    cost_in_usd_ticks: Option<i64>,
    input_tokens: Option<i64>,
    output_tokens: Option<i64>,
    reasoning_tokens: Option<i64>,
    error: Option<String>,
    response_id: Option<String>,
}

impl RawTranscriptRow {
    /// Convert to the public `TranscriptRecord`, validating that token counts
    /// are non-negative.
    fn into_record(self) -> Result<TranscriptRecord, StoreError> {
        Ok(TranscriptRecord {
            id: self.id,
            session_id: self.session_id,
            request_at: self.request_at,
            response_at: self.response_at,
            endpoint: self.endpoint,
            method: self.method,
            request_body: self.request_body,
            response_body: self.response_body,
            status_code: self.status_code,
            cost_in_usd_ticks: self.cost_in_usd_ticks,
            input_tokens: self
                .input_tokens
                .map(|v| i64_to_u64(v, "input_tokens"))
                .transpose()?,
            output_tokens: self
                .output_tokens
                .map(|v| i64_to_u64(v, "output_tokens"))
                .transpose()?,
            reasoning_tokens: self
                .reasoning_tokens
                .map(|v| i64_to_u64(v, "reasoning_tokens"))
                .transpose()?,
            error: self.error,
            response_id: self.response_id,
        })
    }
}

/// Safely convert a non-negative `i64` (as stored in SQLite) to `u64`.
///
/// Returns `NegativeTokenCount` if the value is negative.
fn i64_to_u64(value: i64, column: &'static str) -> Result<u64, StoreError> {
    u64::try_from(value).map_err(|_| StoreError::NegativeTokenCount { column, value })
}

/// Borrowed handle for transcript operations on the store's connection.
pub struct TranscriptRepo<'a> {
    conn: &'a Connection,
}

impl<'a> TranscriptRepo<'a> {
    pub(crate) fn new(conn: &'a Connection) -> Self {
        Self { conn }
    }

    /// Insert a partial transcript record for an outbound API request.
    ///
    /// Returns the auto-incremented transcript ID. The caller should pass this
    /// ID to `log_response` or `log_error` when the result arrives.
    ///
    /// Fails with a foreign key violation if `session_id` does not reference an
    /// existing session.
    pub fn log_request(
        &self,
        session_id: &str,
        endpoint: &str,
        method: &str,
        request_body: Option<&str>,
    ) -> Result<i64, StoreError> {
        let request_at = now();
        self.conn
            .execute(
                "INSERT INTO transcripts (session_id, request_at, endpoint, method, request_body) \
                 VALUES (?1, ?2, ?3, ?4, ?5)",
                params![session_id, &request_at, endpoint, method, request_body],
            )
            .map_err(|e| match e {
                rusqlite::Error::SqliteFailure(ref err, _)
                    if err.code == rusqlite::ErrorCode::ConstraintViolation =>
                {
                    StoreError::ForeignKeyViolation(format!(
                        "session_id '{session_id}' does not exist"
                    ))
                }
                other => StoreError::Sql(other),
            })?;
        Ok(self.conn.last_insert_rowid())
    }

    /// Update a transcript record with the API response data.
    ///
    /// Sets `response_at` to the current time and fills in status code, response
    /// body, token counts, and cost from the provided `usage`.
    ///
    /// Token counts are stored as `i64` in SQLite (which has no unsigned integer
    /// type). The `u64` values in `TranscriptUsage` are converted via
    /// `i64::try_from`, which will fail if a value exceeds `i64::MAX` (~9.2e18
    /// tokens, which is not physically reachable).
    ///
    /// Returns an error if no transcript exists with the given `transcript_id`.
    pub fn log_response(
        &self,
        transcript_id: i64,
        status_code: i32,
        response_body: Option<&str>,
        usage: &TranscriptUsage,
        response_id: Option<&str>,
    ) -> Result<(), StoreError> {
        // Convert u64 token counts to i64 for SQLite storage.
        let input_tokens = usage
            .input_tokens
            .map(|v| {
                i64::try_from(v).map_err(|_| StoreError::NegativeTokenCount {
                    column: "input_tokens",
                    value: -1,
                })
            })
            .transpose()?;
        let output_tokens = usage
            .output_tokens
            .map(|v| {
                i64::try_from(v).map_err(|_| StoreError::NegativeTokenCount {
                    column: "output_tokens",
                    value: -1,
                })
            })
            .transpose()?;
        let reasoning_tokens = usage
            .reasoning_tokens
            .map(|v| {
                i64::try_from(v).map_err(|_| StoreError::NegativeTokenCount {
                    column: "reasoning_tokens",
                    value: -1,
                })
            })
            .transpose()?;

        let response_at = now();
        let affected = self
            .conn
            .execute(
                "UPDATE transcripts SET \
                 response_at = ?1, \
                 status_code = ?2, \
                 response_body = ?3, \
                 cost_in_usd_ticks = ?4, \
                 input_tokens = ?5, \
                 output_tokens = ?6, \
                 reasoning_tokens = ?7, \
                 response_id = ?8 \
                 WHERE id = ?9",
                params![
                    &response_at,
                    status_code,
                    response_body,
                    usage.cost_in_usd_ticks,
                    input_tokens,
                    output_tokens,
                    reasoning_tokens,
                    response_id,
                    transcript_id,
                ],
            )
            .map_err(StoreError::Sql)?;
        if affected == 0 {
            return Err(StoreError::TranscriptNotFound(transcript_id));
        }
        Ok(())
    }

    /// Record an error for a transcript (failed API request).
    ///
    /// Returns an error if no transcript exists with the given `transcript_id`.
    pub fn log_error(&self, transcript_id: i64, error: &str) -> Result<(), StoreError> {
        let affected = self
            .conn
            .execute(
                "UPDATE transcripts SET error = ?1 WHERE id = ?2",
                params![error, transcript_id],
            )
            .map_err(StoreError::Sql)?;
        if affected == 0 {
            return Err(StoreError::TranscriptNotFound(transcript_id));
        }
        Ok(())
    }

    /// Return the `response_id` from the most recent transcript entry for a
    /// session (by `request_at` descending). Returns `None` if the session has
    /// no transcripts or the most recent transcript has no `response_id`.
    ///
    /// This is used for stateful session resume: the caller chains
    /// `previous_response_id` from the last turn.
    ///
    /// Returns the `response_id` from the most recent transcript entry
    /// (by autoincrement `id` DESC), which may be `None` if the latest
    /// transcript has no response_id. This prevents callers from
    /// accidentally resuming against a stale older response_id.
    pub fn get_last_response_id(&self, session_id: &str) -> Result<Option<String>, StoreError> {
        let mut stmt = self
            .conn
            .prepare(
                "SELECT response_id FROM transcripts \
                 WHERE session_id = ?1 \
                 ORDER BY id DESC LIMIT 1",
            )
            .map_err(StoreError::Sql)?;

        let mut rows = stmt
            .query_map(params![session_id], |row| row.get::<_, Option<String>>(0))
            .map_err(StoreError::Sql)?;

        match rows.next() {
            Some(Ok(id)) => Ok(id),
            Some(Err(e)) => Err(StoreError::Sql(e)),
            None => Ok(None), // No transcripts at all
        }
    }

    /// Return all transcripts for a session, ordered by `request_at` ascending.
    ///
    /// Token counts are stored as `i64` in SQLite and converted to `u64` on read.
    /// Negative values (which should never occur if data was written through this
    /// API) produce a `NegativeTokenCount` error rather than silently wrapping.
    pub fn list_by_session(&self, session_id: &str) -> Result<Vec<TranscriptRecord>, StoreError> {
        let mut stmt = self
            .conn
            .prepare(
                "SELECT id, session_id, request_at, response_at, endpoint, method, \
                 request_body, response_body, status_code, cost_in_usd_ticks, \
                 input_tokens, output_tokens, reasoning_tokens, error, response_id \
                 FROM transcripts WHERE session_id = ?1 ORDER BY request_at ASC",
            )
            .map_err(StoreError::Sql)?;

        let rows = stmt
            .query_map(params![session_id], |row| {
                Ok(RawTranscriptRow {
                    id: row.get(0)?,
                    session_id: row.get(1)?,
                    request_at: row.get(2)?,
                    response_at: row.get(3)?,
                    endpoint: row.get(4)?,
                    method: row.get(5)?,
                    request_body: row.get(6)?,
                    response_body: row.get(7)?,
                    status_code: row.get(8)?,
                    cost_in_usd_ticks: row.get(9)?,
                    input_tokens: row.get(10)?,
                    output_tokens: row.get(11)?,
                    reasoning_tokens: row.get(12)?,
                    error: row.get(13)?,
                    response_id: row.get(14)?,
                })
            })
            .map_err(StoreError::Sql)?;

        let mut results = Vec::new();
        for row in rows {
            let raw = row.map_err(StoreError::Sql)?;
            results.push(raw.into_record()?);
        }
        Ok(results)
    }
}
