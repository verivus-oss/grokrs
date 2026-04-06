# SQLite Store Spec

Date: 2026-04-05
ADR: `docs/design/01_SQLITE_STATE.md`

## Summary

`grokrs-store` is a new crate providing SQLite WAL-backed persistence for all runtime state in the grokrs system. It replaces in-memory-only session state with durable, queryable, crash-recoverable storage. It provides the persistence foundation that the transcript store, approval broker, and audit export features will build on.

## Goals

- Persist session lifecycle state across process restarts.
- Record API request/response transcripts as an append-only audit trail.
- Accumulate token usage and cost (`cost_in_usd_ticks`) per session, queryable across sessions.
- Provide a migration framework so the schema evolves without manual intervention.
- Keep the database local to the workspace (`.grokrs/state.db`) with no external infrastructure.
- Allow operators to inspect state with standard SQLite tooling.

## Non-Goals

- Replacing TOML for static configuration — `AppConfig` stays TOML.
- Multi-process concurrent write access — grokrs is a single-process CLI.
- Distributed state or replication.
- Approval broker logic (separate spec; this crate provides the storage surface).
- Evidence records with TTL (separate spec; this crate provides the table and expiry query).

## Functional Requirements

### Database Lifecycle

1. `Store::open(workspace_root: &Path) -> Result<Self>` creates or opens `.grokrs/state.db` under the workspace root. Creates the `.grokrs/` directory if it does not exist.

2. On open, set `PRAGMA journal_mode=WAL`, `PRAGMA busy_timeout=5000`, `PRAGMA foreign_keys=ON`.

3. Run all pending migrations on open. Migrations are embedded in the binary (not external SQL files).

4. Migration versioning: each migration has a monotonic integer version. The `schema_version` table tracks the current version. Migrations run in order, inside a transaction.

5. `Store` holds a single `rusqlite::Connection`. No connection pool — single-process CLI.

6. `Store::close(self)` calls `PRAGMA wal_checkpoint(TRUNCATE)` before closing to compact the WAL file.

### Session Persistence

7. Table: `sessions` — `id TEXT PRIMARY KEY, trust_level TEXT NOT NULL, state TEXT NOT NULL, created_at TEXT NOT NULL, updated_at TEXT NOT NULL`.

8. `store.sessions().create(id, trust_level) -> Result<()>` inserts a new session in `Created` state.

9. `store.sessions().transition(id, new_state) -> Result<()>` updates the state and `updated_at` timestamp. Returns error if session does not exist.

10. `store.sessions().get(id) -> Result<Option<SessionRecord>>` returns the session record.

11. `store.sessions().list_active() -> Result<Vec<SessionRecord>>` returns sessions not in `Closed` or `Failed` state.

12. `trust_level` is stored as a string (`"Untrusted"`, `"InteractiveTrusted"`, `"AdminTrusted"`) matching the type names in `grokrs-cap`. The store does not depend on `grokrs-cap` — it stores strings.

13. `state` is stored as a string matching `SessionState` variant names. `Failed` includes the error message: `"Failed: <message>"`.

### API Transcript Logging

14. Table: `transcripts` — `id INTEGER PRIMARY KEY AUTOINCREMENT, session_id TEXT NOT NULL REFERENCES sessions(id), request_at TEXT NOT NULL, response_at TEXT, endpoint TEXT NOT NULL, method TEXT NOT NULL, request_body TEXT, response_body TEXT, status_code INTEGER, cost_in_usd_ticks INTEGER, input_tokens INTEGER, output_tokens INTEGER, reasoning_tokens INTEGER, error TEXT`.

15. `store.transcripts().log_request(session_id, endpoint, method, request_body) -> Result<i64>` inserts a request record, returns the transcript ID.

16. `store.transcripts().log_response(transcript_id, status_code, response_body, usage) -> Result<()>` updates the record with response data, token counts, and cost.

17. `store.transcripts().log_error(transcript_id, error) -> Result<()>` records a failed request.

18. `store.transcripts().list_by_session(session_id) -> Result<Vec<TranscriptRecord>>` returns all transcripts for a session, ordered by `request_at`.

### Usage Accumulation

19. `store.usage().session_totals(session_id) -> Result<UsageSummary>` returns `SUM(cost_in_usd_ticks)`, `SUM(input_tokens)`, `SUM(output_tokens)`, `SUM(reasoning_tokens)`, `COUNT(*)` for a session.

20. `store.usage().all_totals() -> Result<UsageSummary>` returns the same aggregation across all sessions.

21. `UsageSummary` struct: `total_cost_ticks: i64, total_input_tokens: u64, total_output_tokens: u64, total_reasoning_tokens: u64, request_count: u64`.

### Future Extension Tables (schema only, no API in v1)

22. Table: `approvals` — `id INTEGER PRIMARY KEY AUTOINCREMENT, session_id TEXT NOT NULL REFERENCES sessions(id), effect TEXT NOT NULL, decision TEXT NOT NULL, decided_at TEXT NOT NULL, decided_by TEXT`. Schema created by migration; no Rust API in this spec.

23. Table: `evidence` — `id INTEGER PRIMARY KEY AUTOINCREMENT, session_id TEXT NOT NULL REFERENCES sessions(id), kind TEXT NOT NULL, payload TEXT NOT NULL, created_at TEXT NOT NULL, expires_at TEXT`. Schema created by migration; no Rust API in this spec.

### Configuration

24. `AppConfig` in `grokrs-core` gains an optional `[store]` section:
    ```toml
    [store]
    path = ".grokrs/state.db"   # relative to workspace root
    ```
    Defaults to `.grokrs/state.db` when the section is absent. `store: Option<StoreConfig>`.

25. `.grokrs/` is added to `.gitignore` (runtime state, not version-controlled).

### CLI Integration

26. `grokrs doctor` is updated to report store status: database path, schema version, session count, total cost.

27. `grokrs api chat` creates a session in the store before making API calls, logs transcripts, and reports usage summary on exit.

28. New subcommand: `grokrs store status` — prints database path, schema version, WAL mode, session count, total cost across all sessions.

29. New subcommand: `grokrs store usage [--session <id>]` — prints token usage and cost summary, optionally filtered by session.

## Safety Requirements

1. The database file is created inside the workspace root only — never outside it. The path is validated as workspace-relative (no `..` traversal).

2. WAL mode is set unconditionally on every open — the store never operates in rollback journal mode.

3. All state-changing operations use transactions. A crashed process leaves the database in a consistent state.

4. The store does not depend on `grokrs-cap`, `grokrs-policy`, or `grokrs-api` at compile time. It depends only on `grokrs-core` (for config) and `rusqlite`.

5. Request/response bodies in transcripts may contain sensitive data (API keys are already stripped by the transport layer). No additional scrubbing is performed by the store — the transport layer's guarantee that keys never reach log output is the upstream invariant.

6. The database file permissions are set to `0600` (owner read/write only) on creation.

## Deliverables

- `crates/grokrs-store/` crate with:
  - `src/lib.rs` — `Store` struct, open/close, migration runner
  - `src/migrations/` — embedded SQL migrations (v1: sessions, transcripts, approvals, evidence tables)
  - `src/session.rs` — session CRUD
  - `src/transcript.rs` — transcript logging
  - `src/usage.rs` — usage aggregation queries
  - `src/types.rs` — `SessionRecord`, `TranscriptRecord`, `UsageSummary`, `StoreConfig`
- Updated `Cargo.toml` workspace members
- Updated `crates/grokrs-core/src/lib.rs` with optional `[store]` config section
- Updated `crates/grokrs-cli/` with `store` subcommands and transcript logging in `api chat`
- Updated `.gitignore` with `.grokrs/`
- Tests: migration forward-compat, session state transitions, transcript round-trip, usage aggregation, concurrent read during write (WAL verification), crash recovery (incomplete transaction rollback)
