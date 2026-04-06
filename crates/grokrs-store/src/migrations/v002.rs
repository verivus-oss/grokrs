//! V2 migration: add ON DELETE CASCADE to transcripts FK, add response_id column.
//!
//! SQLite does not support `ALTER TABLE ... ADD CONSTRAINT` or modifying
//! foreign key constraints in place. The only way to add `ON DELETE CASCADE`
//! to an existing FK is to recreate the table. This migration:
//!
//! 1. Creates `transcripts_new` with the CASCADE FK and the new `response_id` column.
//! 2. Copies all existing data from `transcripts` into `transcripts_new`.
//! 3. Drops the old `transcripts` table and its index.
//! 4. Renames `transcripts_new` to `transcripts`.
//! 5. Recreates the `idx_transcripts_session_id` index.
//!
//! Similarly recreates `approvals` and `evidence` tables to add CASCADE FKs,
//! since they also reference `sessions(id)` without CASCADE in V1.

/// V2 migration SQL.
pub const SQL: &str = r"
-- Ensure schema_versions exists for legacy V1 databases that predate this table.
CREATE TABLE IF NOT EXISTS schema_versions (
    version     INTEGER PRIMARY KEY NOT NULL,
    applied_at  TEXT    NOT NULL
);

-- ----------------------------------------------------------------
-- Recreate transcripts with ON DELETE CASCADE + response_id column
-- ----------------------------------------------------------------

CREATE TABLE IF NOT EXISTS transcripts_new (
    id                  INTEGER PRIMARY KEY AUTOINCREMENT,
    session_id          TEXT    NOT NULL REFERENCES sessions(id) ON DELETE CASCADE,
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
    error               TEXT,
    response_id         TEXT
);

INSERT INTO transcripts_new (
    id, session_id, request_at, response_at, endpoint, method,
    request_body, response_body, status_code, cost_in_usd_ticks,
    input_tokens, output_tokens, reasoning_tokens, error
)
SELECT
    id, session_id, request_at, response_at, endpoint, method,
    request_body, response_body, status_code, cost_in_usd_ticks,
    input_tokens, output_tokens, reasoning_tokens, error
FROM transcripts;

DROP TABLE IF EXISTS transcripts;

ALTER TABLE transcripts_new RENAME TO transcripts;

CREATE INDEX IF NOT EXISTS idx_transcripts_session_id ON transcripts(session_id);

-- ----------------------------------------------------------------
-- Recreate approvals with ON DELETE CASCADE
-- ----------------------------------------------------------------

CREATE TABLE IF NOT EXISTS approvals_new (
    id          INTEGER PRIMARY KEY AUTOINCREMENT,
    session_id  TEXT    NOT NULL REFERENCES sessions(id) ON DELETE CASCADE,
    effect      TEXT    NOT NULL,
    decision    TEXT    NOT NULL,
    decided_at  TEXT    NOT NULL,
    decided_by  TEXT
);

INSERT INTO approvals_new (id, session_id, effect, decision, decided_at, decided_by)
SELECT id, session_id, effect, decision, decided_at, decided_by
FROM approvals;

DROP TABLE IF EXISTS approvals;

ALTER TABLE approvals_new RENAME TO approvals;

CREATE INDEX IF NOT EXISTS idx_approvals_session_id ON approvals(session_id);

-- ----------------------------------------------------------------
-- Recreate evidence with ON DELETE CASCADE
-- ----------------------------------------------------------------

CREATE TABLE IF NOT EXISTS evidence_new (
    id          INTEGER PRIMARY KEY AUTOINCREMENT,
    session_id  TEXT    NOT NULL REFERENCES sessions(id) ON DELETE CASCADE,
    kind        TEXT    NOT NULL,
    payload     TEXT    NOT NULL,
    created_at  TEXT    NOT NULL,
    expires_at  TEXT
);

INSERT INTO evidence_new (id, session_id, kind, payload, created_at, expires_at)
SELECT id, session_id, kind, payload, created_at, expires_at
FROM evidence;

DROP TABLE IF EXISTS evidence;

ALTER TABLE evidence_new RENAME TO evidence;

CREATE INDEX IF NOT EXISTS idx_evidence_session_id ON evidence(session_id);
";
