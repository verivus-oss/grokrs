# grokrs-api Agent Orchestration Prompt

## Objective

Implement the `grokrs-api` crate (14 units, ~5,420 LOC) using parallel agent teams organized by DAG layer. Every unit receives a post-implementation review from OpenAI Codex with **full access permissions** (`fullAuto: true`, `dangerouslyBypassApprovalsAndSandbox: true`). The implementing agent must fix every issue Codex raises and resubmit until Codex provides **unconditional approval** (no remaining issues, no caveats, no suggestions).

## Execution Model

```
Orchestrator (Claude Opus — this session)
  │
  ├── Wave 1: Foundation ──────────────────────────────
  │     Agent-W1: U10 (wire types) + U11 (transport)
  │     → Codex review loop → merge to master
  │
  ├── Wave 2: Endpoint Clients + Early Auxiliary ──────
  │     Agent-W2A: U12 (Responses API)          ─┐
  │     Agent-W2B: U13 (Chat Completions)        │
  │     Agent-W2C: U14 (Streaming)               ├─ parallel worktrees
  │     Agent-W2D: U15 (Models API)              │
  │     Agent-W2E: U21+U22 (Files + Utilities)   │
  │     Agent-W2F: U23 (Policy bridge + config)  ─┘
  │     → Codex review loop each → merge all to master
  │
  ├── Wave 3: Tools + Media ───────────────────────────
  │     Agent-W3A: U16 (Function calling)        ─┐
  │     Agent-W3B: U17 (Media generation)         ├─ parallel worktrees
  │     → Codex review loop each → merge to master
  │
  ├── Wave 4: Batch ───────────────────────────────────
  │     Agent-W4: U20 (Batch API)
  │     → Codex review loop → merge to master
  │
  └── Wave 5: Facade + CLI ───────────────────────────
        Agent-W5: U30 (GrokClient) + U31 (CLI)
        → Codex review loop → merge to master
```

## Per-Agent Protocol

Each agent receives a prompt with:
1. The full spec section(s) for its unit(s) from `docs/specs/01_XAI_API_CLIENT.md`
2. The DAG unit definition(s) from `docs/reviews/xai-api-client/2026-04-05/IMPLEMENTATION_DAG.toml`
3. The existing codebase context (crate structure, dependency types)
4. Explicit file creation/modification list
5. Acceptance criteria

Each agent MUST:
1. **Implement** all code per spec — complete, production-ready, no stubs/TODOs
2. **Run** `cargo fmt --all && cargo clippy --workspace --all-targets && cargo test`
3. **Fix** any compilation, lint, or test failures
4. **Report** back the list of files created/modified and test results

## Codex Review Loop Protocol

After each agent completes implementation:

1. **Collect** all files the agent created or modified
2. **Send to Codex** via `codex_request` with:
   - `fullAuto: true`
   - `dangerouslyBypassApprovalsAndSandbox: true`
   - Prompt: spec excerpt + file list + "Review this implementation for correctness, completeness, safety, and adherence to the spec. List every issue. If there are zero issues, respond with exactly: APPROVED"
3. **Parse** Codex response:
   - If "APPROVED" → unit is done
   - If issues listed → dispatch fix agent with Codex's feedback, re-run tests, resubmit to Codex
4. **Max iterations**: 5 (if not approved after 5 rounds, escalate to user)

## Codex Review Prompt Template

```
You are reviewing a Rust implementation for the grokrs project.

## Project Context
- Safety-first Rust CLI scaffold for xAI Grok API
- Trust levels are type parameters (Untrusted, InteractiveTrusted, AdminTrusted)
- Policy engine evaluates effects (FsRead, FsWrite, ProcessSpawn, NetworkConnect) before execution
- Deny-by-default: network and shell denied in sample config

## Unit Under Review: {UNIT_ID} — {UNIT_NAME}

## Spec Requirements
{SPEC_EXCERPT}

## Acceptance Criteria
{ACCEPTANCE_CRITERIA}

## Files to Review
{FILE_LIST_WITH_PATHS}

## Instructions
1. Read every file listed above
2. Verify the implementation satisfies EVERY acceptance criterion
3. Verify serde types match the xAI wire format described in the spec
4. Verify error handling is complete (no unwrap/expect in non-test code)
5. Verify safety requirements (no secrets in logs/debug, policy gate not bypassed)
6. Verify tests exist and cover the acceptance criteria
7. Check for any TODO, FIXME, unimplemented!(), or stub code — these are NOT acceptable

If there are ZERO issues: respond with exactly "APPROVED"
If there are issues: list each one with file path, line reference, and specific fix needed
```

## Merge Strategy

After each wave:
1. Verify all agents in the wave got Codex approval
2. Merge worktree changes into master sequentially
3. Run `cargo fmt --all && cargo clippy --workspace --all-targets && cargo test` on master
4. Fix any cross-unit integration issues (primarily `mod.rs` and `lib.rs` re-exports)
5. Proceed to next wave only when master is green

## Conflict Groups (from DAG)

These files are modified by multiple units and require careful merge:
- `crates/grokrs-api/Cargo.toml` — U10, U11 (Wave 1 only, single agent)
- `crates/grokrs-api/src/types/mod.rs` — all type-adding units (each adds `pub mod X;`)
- `crates/grokrs-api/src/endpoints/mod.rs` — all endpoint units (each adds `pub mod X;`)
- `crates/grokrs-api/src/lib.rs` — U10 (creates), U12 (adds re-exports), U30 (adds client)
- `crates/grokrs-core/src/lib.rs` — U23 (adds ApiConfig)
- `configs/grokrs.example.toml` — U23 (adds [api] section)
- `crates/grokrs-cli/src/main.rs` — U31 (adds api subcommands)

Resolution: After merging each wave's worktrees, the orchestrator reconciles `mod.rs` files by combining all `pub mod` declarations.

## Wave Dependencies

| Wave | Units | Depends On | Can Start When |
|------|-------|------------|----------------|
| 1 | U10, U11 | — | Immediately |
| 2 | U12, U13, U14, U15, U21+U22, U23 | U10, U11 | Wave 1 merged + green |
| 3 | U16, U17 | U12-U15 (W2) | Wave 2 merged + green |
| 4 | U20 | U16 (W3) | Wave 3 merged + green |
| 5 | U30, U31 | All prior | Wave 4 merged + green |

## Success Criteria

- All 14 units implemented and Codex-approved
- `cargo fmt --all` — no changes
- `cargo clippy --workspace --all-targets` — zero warnings
- `cargo test` — all tests pass
- `cargo run -p grokrs-cli -- doctor` — runs successfully
- `cargo run -p grokrs-cli -- show-config configs/grokrs.example.toml` — shows [api] section
- No TODO, FIXME, unimplemented!(), or placeholder code anywhere in grokrs-api
