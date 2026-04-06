//! Session persistence: create, transition, get, list_active.
//!
//! `SessionRepo` is a borrowed handle into the `Store`'s connection. It provides
//! CRUD operations for session lifecycle state. Trust levels and states are stored
//! as plain strings to avoid compile-time coupling with `grokrs-cap` or
//! `grokrs-session`.

use rusqlite::{Connection, params};

use crate::StoreError;
use crate::types::SessionRecord;

/// Borrowed handle for session operations on the store's connection.
pub struct SessionRepo<'a> {
    conn: &'a Connection,
}

impl<'a> SessionRepo<'a> {
    pub(crate) fn new(conn: &'a Connection) -> Self {
        Self { conn }
    }

    /// Insert a new session in `Created` state with current timestamps.
    ///
    /// `trust_level` should be one of `"Untrusted"`, `"InteractiveTrusted"`,
    /// or `"AdminTrusted"`.
    pub fn create(&self, id: &str, trust_level: &str) -> Result<(), StoreError> {
        let now = now_rfc3339();
        self.conn
            .execute(
                "INSERT INTO sessions (id, trust_level, state, created_at, updated_at) VALUES (?1, ?2, ?3, ?4, ?5)",
                params![id, trust_level, "Created", &now, &now],
            )
            .map_err(|e| match e {
                rusqlite::Error::SqliteFailure(ref err, _)
                    if err.code == rusqlite::ErrorCode::ConstraintViolation =>
                {
                    StoreError::DuplicateSession(id.to_owned())
                }
                other => StoreError::Sql(other),
            })?;
        Ok(())
    }

    /// Update the session's state and `updated_at` timestamp.
    ///
    /// Returns an error if the session does not exist. For `Failed` state,
    /// callers should pass `"Failed: <message>"` as `new_state`.
    pub fn transition(&self, id: &str, new_state: &str) -> Result<(), StoreError> {
        let now = now_rfc3339();
        let affected = self
            .conn
            .execute(
                "UPDATE sessions SET state = ?1, updated_at = ?2 WHERE id = ?3",
                params![new_state, &now, id],
            )
            .map_err(StoreError::Sql)?;
        if affected == 0 {
            return Err(StoreError::SessionNotFound(id.to_owned()));
        }
        Ok(())
    }

    /// Retrieve a session by ID, or `None` if it does not exist.
    pub fn get(&self, id: &str) -> Result<Option<SessionRecord>, StoreError> {
        let mut stmt = self
            .conn
            .prepare(
                "SELECT id, trust_level, state, created_at, updated_at FROM sessions WHERE id = ?1",
            )
            .map_err(StoreError::Sql)?;

        let mut rows = stmt
            .query_map(params![id], |row| {
                Ok(SessionRecord {
                    id: row.get(0)?,
                    trust_level: row.get(1)?,
                    state: row.get(2)?,
                    created_at: row.get(3)?,
                    updated_at: row.get(4)?,
                })
            })
            .map_err(StoreError::Sql)?;

        match rows.next() {
            Some(Ok(record)) => Ok(Some(record)),
            Some(Err(e)) => Err(StoreError::Sql(e)),
            None => Ok(None),
        }
    }

    /// Return the total number of sessions (all states).
    pub fn count_total(&self) -> Result<i64, StoreError> {
        let count: i64 = self
            .conn
            .query_row("SELECT COUNT(*) FROM sessions", [], |row| row.get(0))
            .map_err(StoreError::Sql)?;
        Ok(count)
    }

    /// Return all sessions ordered by `updated_at` descending, with optional
    /// limit.
    pub fn list_all(&self, limit: Option<u32>) -> Result<Vec<SessionRecord>, StoreError> {
        let sql = match limit {
            Some(_) => {
                "SELECT id, trust_level, state, created_at, updated_at FROM sessions \
                 ORDER BY updated_at DESC LIMIT ?1"
            }
            None => {
                "SELECT id, trust_level, state, created_at, updated_at FROM sessions \
                 ORDER BY updated_at DESC"
            }
        };

        let mut stmt = self.conn.prepare(sql).map_err(StoreError::Sql)?;

        let map_row = |row: &rusqlite::Row<'_>| -> rusqlite::Result<SessionRecord> {
            Ok(SessionRecord {
                id: row.get(0)?,
                trust_level: row.get(1)?,
                state: row.get(2)?,
                created_at: row.get(3)?,
                updated_at: row.get(4)?,
            })
        };

        let mut results = Vec::new();
        if let Some(n) = limit {
            let rows = stmt
                .query_map(params![n], map_row)
                .map_err(StoreError::Sql)?;
            for row in rows {
                results.push(row.map_err(StoreError::Sql)?);
            }
        } else {
            let rows = stmt.query_map([], map_row).map_err(StoreError::Sql)?;
            for row in rows {
                results.push(row.map_err(StoreError::Sql)?);
            }
        }
        Ok(results)
    }

    /// Return sessions matching the given state (exact match).
    pub fn list_by_state(&self, state: &str) -> Result<Vec<SessionRecord>, StoreError> {
        let mut stmt = self
            .conn
            .prepare(
                "SELECT id, trust_level, state, created_at, updated_at FROM sessions \
                 WHERE state = ?1 ORDER BY updated_at DESC",
            )
            .map_err(StoreError::Sql)?;

        let rows = stmt
            .query_map(params![state], |row| {
                Ok(SessionRecord {
                    id: row.get(0)?,
                    trust_level: row.get(1)?,
                    state: row.get(2)?,
                    created_at: row.get(3)?,
                    updated_at: row.get(4)?,
                })
            })
            .map_err(StoreError::Sql)?;

        let mut results = Vec::new();
        for row in rows {
            results.push(row.map_err(StoreError::Sql)?);
        }
        Ok(results)
    }

    /// Return sessions whose ID starts with the given prefix.
    ///
    /// SQL LIKE special characters (`%`, `_`) in the prefix are escaped so
    /// they match literally. Uses the backslash as ESCAPE character.
    pub fn find_by_prefix(&self, prefix: &str) -> Result<Vec<SessionRecord>, StoreError> {
        // Escape SQL LIKE special chars in the prefix.
        let escaped = prefix
            .replace('\\', "\\\\")
            .replace('%', "\\%")
            .replace('_', "\\_");
        let pattern = format!("{escaped}%");

        let mut stmt = self
            .conn
            .prepare(
                "SELECT id, trust_level, state, created_at, updated_at FROM sessions \
                 WHERE id LIKE ?1 ESCAPE '\\' ORDER BY updated_at DESC",
            )
            .map_err(StoreError::Sql)?;

        let rows = stmt
            .query_map(params![pattern], |row| {
                Ok(SessionRecord {
                    id: row.get(0)?,
                    trust_level: row.get(1)?,
                    state: row.get(2)?,
                    created_at: row.get(3)?,
                    updated_at: row.get(4)?,
                })
            })
            .map_err(StoreError::Sql)?;

        let mut results = Vec::new();
        for row in rows {
            results.push(row.map_err(StoreError::Sql)?);
        }
        Ok(results)
    }

    /// Delete sessions in terminal states (`Closed` or `Failed: ...`) with
    /// `updated_at` before the given RFC 3339 timestamp.
    ///
    /// Failed sessions are stored as `"Failed: <message>"` (not bare `"Failed"`),
    /// so we use `LIKE 'Failed%'` to match all failure states. Active sessions
    /// (`Created`, `Ready`, `RunningTurn`, `WaitingApproval`) are never deleted
    /// regardless of age. Returns the number of deleted session rows.
    /// Associated transcripts, approvals, and evidence are cascade-deleted
    /// by the V2 schema FK constraint.
    pub fn delete_old(&self, before: &str) -> Result<u64, StoreError> {
        let affected = self
            .conn
            .execute(
                "DELETE FROM sessions WHERE updated_at < ?1 AND (state = 'Closed' OR state LIKE 'Failed%')",
                params![before],
            )
            .map_err(StoreError::Sql)?;
        Ok(affected as u64)
    }

    /// Return the number of transcript entries for a session.
    pub fn count_transcripts(&self, session_id: &str) -> Result<i64, StoreError> {
        let count: i64 = self
            .conn
            .query_row(
                "SELECT COUNT(*) FROM transcripts WHERE session_id = ?1",
                params![session_id],
                |row| row.get(0),
            )
            .map_err(StoreError::Sql)?;
        Ok(count)
    }

    /// Return all sessions whose state is not `Closed` or `Failed` (including
    /// `Failed: <message>` variants).
    pub fn list_active(&self) -> Result<Vec<SessionRecord>, StoreError> {
        let mut stmt = self
            .conn
            .prepare(
                "SELECT id, trust_level, state, created_at, updated_at FROM sessions \
                 WHERE state != 'Closed' AND state NOT LIKE 'Failed%' \
                 ORDER BY created_at ASC",
            )
            .map_err(StoreError::Sql)?;

        let rows = stmt
            .query_map([], |row| {
                Ok(SessionRecord {
                    id: row.get(0)?,
                    trust_level: row.get(1)?,
                    state: row.get(2)?,
                    created_at: row.get(3)?,
                    updated_at: row.get(4)?,
                })
            })
            .map_err(StoreError::Sql)?;

        let mut results = Vec::new();
        for row in rows {
            results.push(row.map_err(StoreError::Sql)?);
        }
        Ok(results)
    }
}

/// Return the current UTC time as an RFC 3339 string.
///
/// Uses `std::time::SystemTime` to avoid adding a `chrono` dependency.
/// Format: `YYYY-MM-DDTHH:MM:SSZ`.
pub(crate) fn now_rfc3339() -> String {
    use std::time::SystemTime;
    let duration = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .expect("system clock before UNIX epoch");
    let secs = duration.as_secs();

    // Break down into date/time components.
    let days = secs / 86400;
    let time_of_day = secs % 86400;
    let hours = time_of_day / 3600;
    let minutes = (time_of_day % 3600) / 60;
    let seconds = time_of_day % 60;

    // Convert days since epoch to year/month/day.
    let (year, month, day) = days_to_ymd(days);

    format!("{year:04}-{month:02}-{day:02}T{hours:02}:{minutes:02}:{seconds:02}Z")
}

/// Convert days since UNIX epoch (1970-01-01) to (year, month, day).
fn days_to_ymd(days: u64) -> (u64, u64, u64) {
    // Algorithm from Howard Hinnant's `civil_from_days`.
    // RATIONALE: the intermediate signed arithmetic is required by the
    // algorithm.  `days` is seconds-since-epoch / 86400 and stays well
    // within i64 range for any realistic timestamp.  `doe` is guaranteed
    // non-negative by the algorithm (day-of-era ∈ [0, 146096]).  `year`
    // is positive for all post-epoch dates.
    #[allow(clippy::cast_possible_wrap)]
    let z = days as i64 + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    #[allow(clippy::cast_sign_loss)]
    let doe = (z - era * 146_097) as u64;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    #[allow(clippy::cast_possible_wrap)]
    let y = yoe as i64 + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = if m <= 2 { y + 1 } else { y };
    #[allow(clippy::cast_sign_loss)]
    {
        (year as u64, m, d)
    }
}

// Re-export for use by transcript module.
pub(crate) use now_rfc3339 as now;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn now_rfc3339_format() {
        let ts = now_rfc3339();
        // Basic format check: YYYY-MM-DDTHH:MM:SSZ
        assert!(ts.ends_with('Z'), "timestamp should end with Z: {ts}");
        assert_eq!(ts.len(), 20, "expected 20-char RFC 3339: {ts}");
        assert_eq!(&ts[4..5], "-");
        assert_eq!(&ts[7..8], "-");
        assert_eq!(&ts[10..11], "T");
    }
}
