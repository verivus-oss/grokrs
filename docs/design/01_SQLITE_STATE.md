# ADR-01: SQLite WAL for Runtime State, TOML for Static Config

Date: 2026-04-05

## Status

Accepted

## Context

grokrs needs two fundamentally different persistence patterns:

1. **Static configuration** — human-authored declarations read once at startup (workspace name, model provider, policy flags, API endpoint). These change between runs, not during runs.

2. **Runtime state** — machine-written data that changes during execution (session lifecycle, API transcripts, token usage, approval decisions, evidence records, audit logs). This data is written concurrently, needs atomicity, and accumulates across sessions.

TOML files work well for (1) but fail at (2):
- No atomic writes — concurrent readers see partial state
- No transactions — multi-field updates can tear
- No concurrent access — file locks are advisory and fragile
- No queryability — aggregating token usage across sessions requires loading everything into memory

The openfang project (16K stars) hit this exact failure mode: TOML config and SQLite DB drifted apart, manifest edits were silently ignored on restart, leading to invisible state corruption (openfang#219).

## Decision

**TOML for static configuration. SQLite WAL for all runtime state.**

### Stays TOML

| Item | Rationale |
|------|-----------|
| `AppConfig` (`[workspace]`, `[model]`, `[policy]`, `[session]`, `[api]`) | Human-edited, read once at startup, immutable during execution |
| `configs/grokrs.example.toml` | Reference config for operators |
| Policy declarations (`allow_network`, `allow_shell`, etc.) | Security posture is a declaration, not runtime state |

### Moves to SQLite

| Item | Rationale |
|------|-----------|
| Session lifecycle state | State machine transitions (Created→Ready→RunningTurn→etc) are concurrent, need atomicity |
| API request/response transcripts | Append-only audit trail, needs durability and queryability |
| Token usage and cost accumulation | `cost_in_usd_ticks` aggregation across requests within a session |
| Approval decisions | Future approval broker needs transactional decision records |
| Evidence records with TTL | Test results linked to episodes, need expiry queries |
| Audit log | Export surface for aivcs — needs structured, queryable records |

### New crate: `grokrs-store`

A dedicated crate providing SQLite-backed persistence:

```
grokrs-store/
  src/
    lib.rs          — Store struct, migrations, connection pool
    migrations/     — Embedded SQL migrations (refinery or built-in)
    session.rs      — Session state persistence
    transcript.rs   — API request/response log
    usage.rs        — Token/cost accumulation
    approval.rs     — Approval decision records (future)
    evidence.rs     — Evidence records with TTL (future)
```

### Dependency direction

```
grokrs-cli → grokrs-store → grokrs-core
                           → rusqlite
```

`grokrs-store` depends on `grokrs-core` for config types and on `rusqlite` for SQLite access. It does not depend on `grokrs-api`, `grokrs-cap`, or `grokrs-policy`.

### SQLite configuration

- **WAL mode** (`PRAGMA journal_mode=WAL`) — concurrent readers, single writer, no reader blocking
- **Database location** — `{workspace_root}/.grokrs/state.db` (alongside `.aivcs/`, `.sqry/`)
- **Migrations** — embedded in the binary, run on first open, versioned
- **Connection** — single `rusqlite::Connection` (not pooled — single-process CLI)
- **Busy timeout** — `PRAGMA busy_timeout=5000` for write contention

### What does NOT go in SQLite

- Configuration (stays TOML — human-readable, version-controllable)
- API wire types (stay in `grokrs-api` — pure data)
- Policy rules (stay in `grokrs-policy` — compile-time safety)
- Workspace path validation (stays in `grokrs-cap` — type-level safety)

## Consequences

- Session state survives process crashes (can resume from last known state)
- Token usage is queryable across sessions (`SELECT SUM(cost_in_usd_ticks) FROM transcripts WHERE session_id = ?`)
- Audit export to aivcs is a SQL query, not a file-system scan
- Single additional runtime dependency (`rusqlite` with `bundled` feature — no system SQLite required)
- `.grokrs/state.db` must be in `.gitignore` (runtime state, not version-controlled)
- Operators can inspect state with standard SQLite tools (`sqlite3 .grokrs/state.db`)

## Alternatives Considered

| Alternative | Why rejected |
|-------------|-------------|
| TOML for everything | No atomicity, no concurrent access, no queryability |
| JSON files | Same problems as TOML plus worse human readability |
| sled / RocksDB | Heavier dependencies, less tooling, no SQL queryability |
| PostgreSQL/Redis | Requires external infrastructure — violates single-binary CLI goal |
| In-memory only | State lost on crash, no cross-session persistence |
