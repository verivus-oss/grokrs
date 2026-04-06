//! SQLite WAL-backed persistence for grokrs runtime state.
//!
//! `grokrs-store` provides durable, queryable, crash-recoverable storage for
//! session lifecycle state, API request/response transcripts, token usage, and
//! cost tracking. The database lives at `.grokrs/state.db` inside the workspace
//! root and uses WAL mode for crash safety and concurrent read access.
//!
//! # Dependency direction
//!
//! ```text
//! grokrs-cli -> grokrs-store -> grokrs-core + rusqlite
//! ```
//!
//! This crate does **not** depend on `grokrs-cap`, `grokrs-policy`, or
//! `grokrs-api`. Trust levels and session states are stored as plain strings.
//!
//! # Future extension tables
//!
//! The V1 migration creates two additional tables (`approvals` and `evidence`)
//! that do not have Rust APIs in this version. These tables reserve the schema
//! shape for upcoming specs:
//!
//! - **approvals** — will record effect approval decisions (effect, decision,
//!   decided_at, decided_by) linked to sessions. The approval broker spec will
//!   add the Rust API.
//! - **evidence** — will store attestation evidence with TTL (kind, payload,
//!   created_at, expires_at) linked to sessions. The evidence/audit spec will
//!   add the Rust API.

pub mod cost;
pub mod memory;
pub mod migrations;
pub mod session;
pub mod transcript;
pub mod types;
pub mod usage;

use std::path::{Path, PathBuf};

use rusqlite::Connection;
use thiserror::Error;

use cost::CostRepo;
use memory::MemoryRepo;
use session::SessionRepo;
use transcript::TranscriptRepo;
use usage::UsageRepo;

/// Errors that can occur during store operations.
#[derive(Debug, Error)]
pub enum StoreError {
    /// The configured store path contains `..` traversal or is absolute.
    #[error("invalid store path '{0}': must be workspace-relative without '..' traversal")]
    InvalidPath(String),

    /// Failed to create the `.grokrs/` directory.
    #[error("failed to create store directory '{path}': {source}")]
    CreateDir {
        path: String,
        #[source]
        source: std::io::Error,
    },

    /// Failed to set file permissions on the database file.
    #[error("failed to set permissions on '{path}': {source}")]
    SetPermissions {
        path: String,
        #[source]
        source: std::io::Error,
    },

    /// A migration failed.
    #[error("migration error: {0}")]
    Migration(String),

    /// An underlying SQLite error.
    #[error("sqlite error: {0}")]
    Sql(#[from] rusqlite::Error),

    /// Attempted to create a session with a duplicate ID.
    #[error("session '{0}' already exists")]
    DuplicateSession(String),

    /// Attempted to transition a session that does not exist.
    #[error("session '{0}' not found")]
    SessionNotFound(String),

    /// Attempted to update a transcript that does not exist.
    #[error("transcript {0} not found")]
    TranscriptNotFound(i64),

    /// Foreign key constraint violation.
    #[error("foreign key violation: {0}")]
    ForeignKeyViolation(String),

    /// A token count read from SQLite was negative, which indicates data
    /// corruption or a bug in the writing path.
    #[error("negative token count in column '{column}': {value}")]
    NegativeTokenCount { column: &'static str, value: i64 },
}

/// SQLite WAL-backed persistence store.
///
/// Holds a single `rusqlite::Connection`. The raw connection is never exposed
/// publicly. Access data through the typed repository handles: [`sessions()`],
/// [`transcripts()`], [`usage()`].
///
/// [`sessions()`]: Store::sessions
/// [`transcripts()`]: Store::transcripts
/// [`usage()`]: Store::usage
pub struct Store {
    conn: Connection,
    /// Absolute path to the database file (for diagnostics).
    db_path: PathBuf,
}

impl Store {
    /// Open or create the store database.
    ///
    /// `workspace_root` must be an absolute path to the workspace directory.
    /// The database is created at `<workspace_root>/<store_path>`, where
    /// `store_path` defaults to `.grokrs/state.db`.
    ///
    /// On every open:
    /// - `PRAGMA journal_mode=WAL`
    /// - `PRAGMA busy_timeout=5000`
    /// - `PRAGMA foreign_keys=ON`
    /// - All pending migrations are applied.
    ///
    /// The database file is created with permissions `0600` on Unix.
    pub fn open(workspace_root: &Path) -> Result<Self, StoreError> {
        Self::open_with_path(workspace_root, ".grokrs/state.db")
    }

    /// Open or create the store database at a custom path relative to the
    /// workspace root.
    ///
    /// The `store_path` must be workspace-relative (no `..` traversal, not
    /// absolute).
    pub fn open_with_path(workspace_root: &Path, store_path: &str) -> Result<Self, StoreError> {
        // Validate store_path: reject absolute paths and `..` traversal.
        validate_store_path(store_path)?;

        let db_path = workspace_root.join(store_path);

        // Create parent directory if absent.
        if let Some(parent) = db_path.parent() {
            std::fs::create_dir_all(parent).map_err(|source| StoreError::CreateDir {
                path: parent.display().to_string(),
                source,
            })?;
        }

        let is_new = !db_path.exists();

        let mut conn =
            Connection::open(&db_path).map_err(|e| StoreError::Migration(format!("{e}")))?;

        // Set file permissions to 0600 on Unix after creation.
        if is_new {
            set_permissions_0600(&db_path)?;
        }

        // Set pragmas unconditionally on every open.
        conn.pragma_update(None, "journal_mode", "WAL")
            .map_err(|e| StoreError::Migration(format!("failed to set journal_mode=WAL: {e}")))?;
        conn.pragma_update(None, "busy_timeout", 5000)
            .map_err(|e| StoreError::Migration(format!("failed to set busy_timeout: {e}")))?;
        conn.pragma_update(None, "foreign_keys", "ON")
            .map_err(|e| StoreError::Migration(format!("failed to set foreign_keys=ON: {e}")))?;

        // Run pending migrations.
        migrations::run_pending(&mut conn)?;

        Ok(Self { conn, db_path })
    }

    /// Return the absolute path to the database file.
    pub fn db_path(&self) -> &Path {
        &self.db_path
    }

    /// Return the current schema version.
    pub fn schema_version(&self) -> Result<u32, StoreError> {
        migrations::current_version(&self.conn)
    }

    /// Return a handle for session operations.
    pub fn sessions(&self) -> SessionRepo<'_> {
        SessionRepo::new(&self.conn)
    }

    /// Return a handle for transcript operations.
    pub fn transcripts(&self) -> TranscriptRepo<'_> {
        TranscriptRepo::new(&self.conn)
    }

    /// Return a handle for usage aggregation queries.
    pub fn usage(&self) -> UsageRepo<'_> {
        UsageRepo::new(&self.conn)
    }

    /// Return a handle for cost aggregation queries.
    pub fn cost(&self) -> CostRepo<'_> {
        CostRepo::new(&self.conn)
    }

    /// Return a handle for memory operations.
    pub fn memories(&self) -> MemoryRepo<'_> {
        MemoryRepo::new(&self.conn)
    }

    /// Expose the raw connection for integration tests only.
    ///
    /// This is **not** part of the public API surface for production code.
    /// Integration tests need direct SQL access to verify future extension
    /// tables (approvals, evidence) that have no Rust API.
    ///
    /// Gated behind `#[cfg(test)]` (unit tests within this crate) or the
    /// `test-support` feature flag (integration tests in `tests/`).
    #[doc(hidden)]
    #[cfg(any(test, feature = "test-support"))]
    pub fn conn_for_testing(&self) -> &Connection {
        &self.conn
    }

    /// Checkpoint the WAL and consume the store.
    ///
    /// Calls `PRAGMA wal_checkpoint(TRUNCATE)` to compact the WAL file before
    /// the connection is dropped. This is a best-effort operation; errors are
    /// returned but the connection will still close on drop.
    pub fn close(self) -> Result<(), StoreError> {
        self.conn
            .execute_batch("PRAGMA wal_checkpoint(TRUNCATE);")
            .map_err(|e| StoreError::Migration(format!("WAL checkpoint failed: {e}")))?;
        // Connection drops here.
        Ok(())
    }
}

/// Validate that a store path is workspace-relative: not absolute, no `..`
/// components.
fn validate_store_path(path: &str) -> Result<(), StoreError> {
    if path.is_empty() {
        return Err(StoreError::InvalidPath("(empty)".to_owned()));
    }

    let p = Path::new(path);

    if p.is_absolute() {
        return Err(StoreError::InvalidPath(path.to_owned()));
    }

    for component in p.components() {
        if let std::path::Component::ParentDir = component {
            return Err(StoreError::InvalidPath(path.to_owned()));
        }
    }

    Ok(())
}

/// Set file permissions to 0600 (owner read/write only) on Unix.
///
/// This is a no-op on non-Unix platforms. The platform limitation is documented
/// but not treated as an error.
#[cfg(unix)]
fn set_permissions_0600(path: &Path) -> Result<(), StoreError> {
    use std::os::unix::fs::PermissionsExt;
    let perms = std::fs::Permissions::from_mode(0o600);
    std::fs::set_permissions(path, perms).map_err(|source| StoreError::SetPermissions {
        path: path.display().to_string(),
        source,
    })?;
    Ok(())
}

#[cfg(not(unix))]
fn set_permissions_0600(_path: &Path) -> Result<(), StoreError> {
    // File permissions 0600 is a no-op on non-Unix platforms.
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn validate_rejects_absolute_path() {
        assert!(validate_store_path("/tmp/state.db").is_err());
    }

    #[test]
    fn validate_rejects_parent_traversal() {
        assert!(validate_store_path("../outside/state.db").is_err());
        assert!(validate_store_path("inside/../outside/state.db").is_err());
    }

    #[test]
    fn validate_rejects_empty_path() {
        assert!(validate_store_path("").is_err());
    }

    #[test]
    fn validate_accepts_relative_path() {
        assert!(validate_store_path(".grokrs/state.db").is_ok());
        assert!(validate_store_path("data/store.db").is_ok());
    }

    #[test]
    fn store_open_creates_directory_and_db() {
        let tmp = tempfile::tempdir().unwrap();
        let store = Store::open(tmp.path()).unwrap();
        assert!(store.db_path().exists());
        assert!(tmp.path().join(".grokrs").is_dir());
    }

    #[test]
    fn store_pragmas_are_set() {
        let tmp = tempfile::tempdir().unwrap();
        let store = Store::open(tmp.path()).unwrap();

        let journal_mode: String = store
            .conn
            .pragma_query_value(None, "journal_mode", |row| row.get(0))
            .unwrap();
        assert_eq!(journal_mode, "wal");

        let busy_timeout: i64 = store
            .conn
            .pragma_query_value(None, "busy_timeout", |row| row.get(0))
            .unwrap();
        assert_eq!(busy_timeout, 5000);

        let foreign_keys: i64 = store
            .conn
            .pragma_query_value(None, "foreign_keys", |row| row.get(0))
            .unwrap();
        assert_eq!(foreign_keys, 1);
    }

    #[test]
    fn store_schema_version_after_open() {
        let tmp = tempfile::tempdir().unwrap();
        let store = Store::open(tmp.path()).unwrap();
        assert_eq!(store.schema_version().unwrap(), 3);
    }

    #[test]
    fn store_close_checkpoints_wal() {
        let tmp = tempfile::tempdir().unwrap();
        let store = Store::open(tmp.path()).unwrap();

        // Insert some data to generate WAL entries.
        store.sessions().create("s1", "Untrusted").unwrap();

        let wal_path = tmp.path().join(".grokrs/state.db-wal");
        // WAL file should exist after writes.
        assert!(wal_path.exists(), "WAL file should exist after writes");

        store.close().unwrap();

        // After close with TRUNCATE checkpoint, WAL file should be empty or gone.
        if wal_path.exists() {
            let meta = std::fs::metadata(&wal_path).unwrap();
            assert_eq!(
                meta.len(),
                0,
                "WAL file should be truncated to zero after close"
            );
        }
    }

    #[test]
    fn store_rejects_traversal_path() {
        let tmp = tempfile::tempdir().unwrap();
        let result = Store::open_with_path(tmp.path(), "../escape/state.db");
        assert!(result.is_err());
    }

    #[test]
    fn store_rejects_absolute_store_path() {
        let tmp = tempfile::tempdir().unwrap();
        let result = Store::open_with_path(tmp.path(), "/tmp/state.db");
        assert!(result.is_err());
    }

    #[cfg(unix)]
    #[test]
    fn store_file_permissions_0600() {
        use std::os::unix::fs::PermissionsExt;
        let tmp = tempfile::tempdir().unwrap();
        let store = Store::open(tmp.path()).unwrap();
        let meta = std::fs::metadata(store.db_path()).unwrap();
        let mode = meta.permissions().mode() & 0o777;
        assert_eq!(mode, 0o600, "expected 0600, got {mode:o}");
    }

    #[test]
    fn store_reopen_is_idempotent() {
        let tmp = tempfile::tempdir().unwrap();

        // First open: creates and migrates.
        {
            let store = Store::open(tmp.path()).unwrap();
            store.sessions().create("s1", "Untrusted").unwrap();
            store.close().unwrap();
        }

        // Second open: should not re-run migrations, data persists.
        {
            let store = Store::open(tmp.path()).unwrap();
            assert_eq!(store.schema_version().unwrap(), 3);
            let session = store.sessions().get("s1").unwrap();
            assert!(session.is_some());
        }
    }
}
