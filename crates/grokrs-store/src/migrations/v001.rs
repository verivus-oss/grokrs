//! V1 migration: initial schema.
//!
//! Creates all tables required by the store:
//!
//! - **sessions** — session lifecycle state, trust level, timestamps.
//! - **transcripts** — API request/response audit trail with token usage and cost.
//! - **approvals** — future extension table for recording effect approval decisions.
//!   No Rust API is exposed in v1; this table reserves the schema shape for the
//!   upcoming approval broker spec.
//! - **evidence** — future extension table for storing attestation evidence with TTL.
//!   No Rust API is exposed in v1; this table reserves the schema shape for the
//!   upcoming evidence/audit spec.

/// V1 migration SQL. Creates sessions, transcripts, approvals, and evidence tables.
///
/// All tables are created in a single migration so the initial schema ships
/// atomically. The approvals and evidence tables are schema-only in this version
/// (no Rust API); they exist so future specs can add Rust APIs without requiring
/// a schema migration.
pub const SQL: &str = r"
CREATE TABLE IF NOT EXISTS schema_versions (
    version     INTEGER PRIMARY KEY NOT NULL,
    applied_at  TEXT    NOT NULL
);

CREATE TABLE IF NOT EXISTS sessions (
    id          TEXT    PRIMARY KEY NOT NULL,
    trust_level TEXT    NOT NULL,
    state       TEXT    NOT NULL,
    created_at  TEXT    NOT NULL,
    updated_at  TEXT    NOT NULL
);

CREATE TABLE IF NOT EXISTS transcripts (
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

CREATE INDEX IF NOT EXISTS idx_transcripts_session_id ON transcripts(session_id);

CREATE TABLE IF NOT EXISTS approvals (
    id          INTEGER PRIMARY KEY AUTOINCREMENT,
    session_id  TEXT    NOT NULL REFERENCES sessions(id),
    effect      TEXT    NOT NULL,
    decision    TEXT    NOT NULL,
    decided_at  TEXT    NOT NULL,
    decided_by  TEXT
);

CREATE INDEX IF NOT EXISTS idx_approvals_session_id ON approvals(session_id);

CREATE TABLE IF NOT EXISTS evidence (
    id          INTEGER PRIMARY KEY AUTOINCREMENT,
    session_id  TEXT    NOT NULL REFERENCES sessions(id),
    kind        TEXT    NOT NULL,
    payload     TEXT    NOT NULL,
    created_at  TEXT    NOT NULL,
    expires_at  TEXT
);

CREATE INDEX IF NOT EXISTS idx_evidence_session_id ON evidence(session_id);
";
