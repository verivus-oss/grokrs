//! V3 migration: add memories table for cross-session agent memory.
//!
//! Creates the `memories` table for key-value semantic context persistence
//! across agent sessions. Each memory has a category (fact, decision,
//! preference), timestamps, and an access count for ranking.

/// V3 migration SQL. Creates the memories table.
pub const SQL: &str = r"
CREATE TABLE IF NOT EXISTS memories (
    id            INTEGER PRIMARY KEY AUTOINCREMENT,
    key           TEXT    NOT NULL UNIQUE,
    value         TEXT    NOT NULL,
    category      TEXT    NOT NULL DEFAULT 'fact',
    created_at    TEXT    NOT NULL,
    updated_at    TEXT    NOT NULL,
    access_count  INTEGER NOT NULL DEFAULT 0
);

CREATE INDEX IF NOT EXISTS idx_memories_category ON memories(category);
CREATE INDEX IF NOT EXISTS idx_memories_key ON memories(key);
";
