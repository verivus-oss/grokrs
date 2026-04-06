# grokrs Implementation Plan

## Phase 1

- Create the repo surface and Cargo workspace
- Establish capability, policy, session, and CLI crates
- Add bootstrap documentation and review artifacts

## Phase 2

- Add xAI API client crate (`grokrs-api`) — see `docs/specs/01_XAI_API_CLIENT.md`
- Add SQLite WAL store crate (`grokrs-store`) — see `docs/specs/02_SQLITE_STORE.md`
- Wire API transcripts and session persistence through the store

## Phase 3

- Add approval broker (logic crate, storage via `grokrs-store`)
- Introduce tool execution harness with explicit effect classification
- Add audit export into `aivcs` (queryable from `grokrs-store`)

## Phase 4

- Add MCP client/server support
- Add `sqry`-aware symbol retrieval and context planning

