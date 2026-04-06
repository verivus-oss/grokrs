# grokrs Design Notes

This document mirrors the root [ARCHITECTURE.md](../../ARCHITECTURE.md) and captures the initial delivery boundary.

## Current Buildable Surface

- config parsing in `grokrs-core`
- trust and path validation in `grokrs-cap`
- effect evaluation in `grokrs-policy`
- typed session state in `grokrs-session`
- tool classification trait in `grokrs-tool`
- operator commands in `grokrs-cli`

## Immediate Design Decisions

- Rust-only implementation for core logic
- deny-by-default execution posture
- no runtime shell adapter yet
- documentation and review artifacts checked in with the code scaffold

## State Persistence

Static configuration stays TOML. Runtime state moves to SQLite WAL via `grokrs-store`:
- Session lifecycle, API transcripts, token usage, approval decisions, evidence records
- Database at `.grokrs/state.db`, WAL mode, single connection, embedded migrations
- See `docs/design/01_SQLITE_STATE.md` (ADR-01) and `docs/specs/02_SQLITE_STORE.md`

## Deferred Work

- approval broker (storage surface in `grokrs-store`, logic in separate crate)
- tool execution harness
- model provider integration (API client in `grokrs-api`)
- MCP transport
- symbol-aware context assembly from `sqry`

