//! Embedded binary migration runner.
//!
//! Migrations are stored as `const &str` SQL in submodules (one per version).
//! The runner tracks the current schema version using the SQLite `user_version`
//! pragma (not a separate table) and applies only migrations whose version
//! exceeds the current `user_version`.
//!
//! All pending migrations run inside a single transaction. If any migration
//! fails, the entire batch is rolled back and `user_version` remains unchanged.

pub mod v001;
pub mod v002;
pub mod v003;

use rusqlite::{params, Connection, Transaction};

use crate::StoreError;

/// A single embedded migration.
struct Migration {
    /// Monotonically increasing version number (starting at 1).
    version: u32,
    /// SQL to execute for this migration.
    sql: &'static str,
}

/// All registered migrations, in version order.
const MIGRATIONS: &[Migration] = &[
    Migration {
        version: 1,
        sql: v001::SQL,
    },
    Migration {
        version: 2,
        sql: v002::SQL,
    },
    Migration {
        version: 3,
        sql: v003::SQL,
    },
];

/// Return the current schema version from `PRAGMA user_version`.
pub(crate) fn current_version(conn: &Connection) -> Result<u32, StoreError> {
    let version: u32 = conn
        .pragma_query_value(None, "user_version", |row| row.get(0))
        .map_err(|e| StoreError::Migration(format!("failed to read user_version: {e}")))?;
    Ok(version)
}

/// Apply all pending migrations (those with version > current `user_version`).
///
/// Runs inside a single transaction. On success, `user_version` is updated to
/// the highest applied migration version. On failure, the transaction is rolled
/// back and `user_version` remains at its prior value.
pub(crate) fn run_pending(conn: &mut Connection) -> Result<(), StoreError> {
    let current = current_version(conn)?;

    let pending: Vec<&Migration> = MIGRATIONS.iter().filter(|m| m.version > current).collect();

    if pending.is_empty() {
        // Backfill: ensure schema_versions table exists for databases created
        // before this table was added to the V1 migration. Uses CREATE TABLE
        // IF NOT EXISTS so it is a no-op for databases that already have it.
        backfill_schema_versions(conn, current)?;
        return Ok(());
    }

    let tx: Transaction = conn.transaction().map_err(|e| {
        StoreError::Migration(format!("failed to begin migration transaction: {e}"))
    })?;

    for migration in &pending {
        tx.execute_batch(migration.sql).map_err(|e| {
            StoreError::Migration(format!("migration v{:03} failed: {e}", migration.version))
        })?;

        // Record the applied version in the supplementary schema_versions table
        // for auditability. PRAGMA user_version remains the authoritative source.
        let now = crate::session::now();
        tx.execute(
            "INSERT OR REPLACE INTO schema_versions (version, applied_at) VALUES (?1, ?2)",
            params![migration.version, &now],
        )
        .map_err(|e| {
            StoreError::Migration(format!(
                "failed to record schema_versions row for v{:03}: {e}",
                migration.version
            ))
        })?;
    }

    // Update user_version to the highest applied migration.
    let latest = pending.last().unwrap().version;
    tx.pragma_update(None, "user_version", latest)
        .map_err(|e| StoreError::Migration(format!("failed to update user_version: {e}")))?;

    tx.commit()
        .map_err(|e| StoreError::Migration(format!("failed to commit migrations: {e}")))?;

    Ok(())
}

/// Ensure the `schema_versions` table exists for legacy databases that were
/// created before this table was added to the V1 migration SQL. Backfills
/// rows for all migrations up to `current_version`.
/// Backfill `schema_versions` for legacy databases that predate this table.
///
/// Only performs writes when the table is actually missing — on up-to-date
/// databases this is a single read-only query and returns immediately,
/// avoiding write locks on every `Store::open()`.
fn backfill_schema_versions(conn: &Connection, current_version: u32) -> Result<(), StoreError> {
    // Check if schema_versions already exists (read-only query).
    let exists: bool = conn
        .query_row(
            "SELECT COUNT(*) FROM sqlite_master WHERE type='table' AND name='schema_versions'",
            [],
            |row| row.get::<_, i64>(0).map(|c| c > 0),
        )
        .map_err(|e| {
            StoreError::Migration(format!("failed to check schema_versions existence: {e}"))
        })?;

    if exists {
        return Ok(()); // Already present, nothing to backfill.
    }

    // Table is missing — create it and backfill rows atomically so a
    // partial failure (table created but rows missing) cannot leave the
    // database in a state where future opens skip the backfill.
    conn.execute_batch("BEGIN")
        .map_err(|e| StoreError::Migration(format!("failed to begin backfill transaction: {e}")))?;

    let result = (|| -> Result<(), StoreError> {
        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS schema_versions (
                version     INTEGER PRIMARY KEY NOT NULL,
                applied_at  TEXT    NOT NULL
            );",
        )
        .map_err(|e| {
            StoreError::Migration(format!("failed to create schema_versions table: {e}"))
        })?;

        let now = crate::session::now();
        for m in MIGRATIONS {
            if m.version <= current_version {
                conn.execute(
                    "INSERT OR IGNORE INTO schema_versions (version, applied_at) VALUES (?1, ?2)",
                    params![m.version, &now],
                )
                .map_err(|e| {
                    StoreError::Migration(format!(
                        "failed to backfill schema_versions for v{:03}: {e}",
                        m.version
                    ))
                })?;
            }
        }
        Ok(())
    })();

    match result {
        Ok(()) => {
            if let Err(e) = conn.execute_batch("COMMIT") {
                // COMMIT failure can leave the transaction active in SQLite
                // (e.g., SQLITE_BUSY). Roll back to avoid leaking state.
                let _ = conn.execute_batch("ROLLBACK");
                return Err(StoreError::Migration(format!(
                    "failed to commit backfill: {e}"
                )));
            }
            Ok(())
        }
        Err(e) => {
            let _ = conn.execute_batch("ROLLBACK");
            Err(e)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rusqlite::Connection;

    #[test]
    fn migrations_are_ordered() {
        let mut prev = 0u32;
        for m in MIGRATIONS {
            assert!(
                m.version > prev,
                "migration versions must be strictly increasing: {} <= {}",
                m.version,
                prev
            );
            prev = m.version;
        }
    }

    #[test]
    fn run_pending_on_fresh_db() {
        let mut conn = Connection::open_in_memory().unwrap();
        conn.execute_batch("PRAGMA foreign_keys = ON;").unwrap();

        run_pending(&mut conn).unwrap();

        let version = current_version(&conn).unwrap();
        assert_eq!(version, 3);

        // Verify sessions table exists
        let count: i64 = conn
            .query_row("SELECT COUNT(*) FROM sessions", [], |r| r.get(0))
            .unwrap();
        assert_eq!(count, 0);
    }

    #[test]
    fn run_pending_populates_schema_versions_table() {
        let mut conn = Connection::open_in_memory().unwrap();
        conn.execute_batch("PRAGMA foreign_keys = ON;").unwrap();

        run_pending(&mut conn).unwrap();

        // Verify schema_versions table has a row for version 1.
        let (version, applied_at): (u32, String) = conn
            .query_row(
                "SELECT version, applied_at FROM schema_versions WHERE version = 1",
                [],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .unwrap();
        assert_eq!(version, 1);
        assert!(
            applied_at.ends_with('Z'),
            "applied_at should be RFC 3339: {applied_at}"
        );
    }

    #[test]
    fn run_pending_backfills_schema_versions_for_legacy_db() {
        // Simulate a legacy V1 database that was created before schema_versions
        // existed: set user_version=1 and create the tables WITHOUT schema_versions.
        let mut conn = Connection::open_in_memory().unwrap();
        conn.execute_batch("PRAGMA foreign_keys = ON;").unwrap();
        conn.pragma_update(None, "user_version", 1).unwrap();
        // Create the full V1 schema (minus schema_versions) so the DB looks
        // like a real V1 DB and V2 migration can run successfully.
        conn.execute_batch(
            "CREATE TABLE sessions (
                id TEXT PRIMARY KEY NOT NULL,
                trust_level TEXT NOT NULL,
                state TEXT NOT NULL,
                created_at TEXT NOT NULL,
                updated_at TEXT NOT NULL
            );
            CREATE TABLE transcripts (
                id                  INTEGER PRIMARY KEY AUTOINCREMENT,
                session_id          TEXT    NOT NULL REFERENCES sessions(id),
                request_at          TEXT    NOT NULL,
                response_at         TEXT,
                endpoint            TEXT    NOT NULL,
                method              TEXT    NOT NULL,
                request_body        TEXT,
                response_body       TEXT,
                status_code         INTEGER,
                cost_in_usd_ticks   INTEGER,
                input_tokens        INTEGER,
                output_tokens       INTEGER,
                reasoning_tokens    INTEGER,
                error               TEXT
            );
            CREATE TABLE approvals (
                id          INTEGER PRIMARY KEY AUTOINCREMENT,
                session_id  TEXT    NOT NULL REFERENCES sessions(id),
                effect      TEXT    NOT NULL,
                decision    TEXT    NOT NULL,
                decided_at  TEXT    NOT NULL,
                decided_by  TEXT
            );
            CREATE TABLE evidence (
                id          INTEGER PRIMARY KEY AUTOINCREMENT,
                session_id  TEXT    NOT NULL REFERENCES sessions(id),
                kind        TEXT    NOT NULL,
                payload     TEXT    NOT NULL,
                created_at  TEXT    NOT NULL,
                expires_at  TEXT
            );",
        )
        .unwrap();

        // schema_versions should NOT exist yet.
        let exists: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM sqlite_master WHERE type='table' AND name='schema_versions'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(exists, 0, "schema_versions should not exist in legacy DB");

        // Running migrations should apply V2 (pending) and create schema_versions
        // as part of the V2 migration SQL.
        run_pending(&mut conn).unwrap();

        let exists_after: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM sqlite_master WHERE type='table' AND name='schema_versions'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(
            exists_after, 1,
            "schema_versions should exist after migration on legacy DB"
        );

        // V2 and V3 were applied, so schema_versions should have rows for both.
        let version: u32 = conn
            .query_row(
                "SELECT version FROM schema_versions WHERE version = 3",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(version, 3);

        // user_version should be 3 after V2+V3 migrations.
        let uv = current_version(&conn).unwrap();
        assert_eq!(uv, 3);
    }

    #[test]
    fn run_pending_is_idempotent() {
        let mut conn = Connection::open_in_memory().unwrap();
        conn.execute_batch("PRAGMA foreign_keys = ON;").unwrap();

        run_pending(&mut conn).unwrap();
        let v1 = current_version(&conn).unwrap();

        run_pending(&mut conn).unwrap();
        let v2 = current_version(&conn).unwrap();

        assert_eq!(v1, v2);
    }
}
