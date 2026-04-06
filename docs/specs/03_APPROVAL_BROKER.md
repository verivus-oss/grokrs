# Approval Broker Spec

Date: 2026-04-05

## Summary

The approval broker resolves `Decision::Ask` into a concrete `Allow` or `Deny` through user interaction. Today, `Ask` is treated as a blocking denial with the message "interactive approval is not yet implemented" -- both in the `PolicyEngine` evaluation path and in the `grokrs-api` transport layer (which surfaces `TransportError::ApprovalRequired`). This spec defines the `ApprovalBroker` trait, an interactive terminal implementation, persistence of approval decisions in SQLite via `grokrs-store`, integration with the `grokrs-api` transport layer, timeout handling, and batch approval for same-type effects within a session.

The approval broker lives in a new `grokrs-approval` crate. It depends on `grokrs-core` (config), `grokrs-policy` (effect and decision types), and `grokrs-store` (persistence). It does not depend on `grokrs-api`, `grokrs-cap`, or `grokrs-cli`. The CLI wires the broker into both the policy evaluation flow and the API transport layer.

## Goals

- Replace the placeholder "not yet implemented" behavior for `Decision::Ask` with a real interactive approval flow.
- Define an `ApprovalBroker` trait that is backend-agnostic (terminal, GUI, programmatic test stubs).
- Persist every approval decision in SQLite for audit and session-scoped remembering.
- Integrate with the `grokrs-api` transport layer so `PolicyDecision::Ask` triggers the broker instead of returning `ApprovalRequired`.
- Support configurable timeouts with a sensible default.
- Support batch approval: approve or deny all effects of a given type for the remainder of a session.

## Non-Goals

- GUI or web-based approval interfaces (the trait allows them; this spec delivers terminal only).
- Remote approval delegation (e.g., Slack bot, webhook callback).
- Approval workflows that span multiple users or roles.
- Retroactive approval of previously denied effects.
- Automatic approval based on machine-learned trust signals.
- Approval for effects that the `PolicyEngine` returns `Allow` or `Deny` for -- the broker only handles `Ask`.

## Functional Requirements

### ApprovalDecision Type

1. `ApprovalDecision` enum in `grokrs-approval`:
   - `Approved` -- the user explicitly approved the effect.
   - `Denied { reason: String }` -- the user explicitly denied the effect, with an optional reason.
   - `Timeout` -- the approval prompt expired without a response.

2. `ApprovalDecision` implements `Clone`, `Debug`, `PartialEq`, `Eq`. It is the return type of every broker method.

3. `Timeout` is treated as `Deny` by all callers. The broker never silently allows an effect that timed out.

### ApprovalBroker Trait

4. The core trait:
   ```rust
   #[async_trait]
   pub trait ApprovalBroker: Send + Sync {
       async fn request_approval(
           &self,
           session_id: &str,
           effect: &Effect,
           context: &str,
       ) -> ApprovalDecision;
   }
   ```
   `effect` is the `grokrs_policy::Effect` that triggered `Ask`. `context` is a human-readable description of why the effect is being requested (e.g., "API call to api.x.ai for model listing", "shell spawn: cargo test"). `session_id` identifies the session for batch-approval scoping.

5. The trait is object-safe (`dyn ApprovalBroker`). Implementations are injected at runtime, not selected at compile time via generics.

6. The trait uses `async_trait` from the `async-trait` crate. The crate does not impose a specific async runtime -- `tokio` is expected but not required by the trait definition.

### Interactive Terminal Broker

7. `TerminalBroker` implements `ApprovalBroker`. It reads from stdin and writes to stderr (not stdout, which may carry tool output or streaming data).

8. The prompt format:
   ```
   [grokrs] Approval required for: <effect description>
            Context: <context string>
            Allow? [y]es / [n]o / [Y]es-all / [N]o-all / [s]kip-timeout:
   ```
   - `y` or `yes` (case-insensitive) -> `Approved`
   - `n` or `no` (case-insensitive) -> `Denied { reason: "user denied" }`
   - `Y` or `yes-all` -> `Approved`, and remember approval for this effect type for the session
   - `N` or `no-all` -> `Denied`, and remember denial for this effect type for the session
   - No input within the timeout -> `Timeout`

9. The prompt repeats on invalid input (anything other than the recognized responses) with a short "unrecognized input" message, without consuming the timeout budget. The timeout clock resets on each invalid input so the user is never penalized for a typo.

10. `TerminalBroker` is constructed with a `TerminalBrokerConfig` containing:
    - `timeout: Duration` -- maximum time to wait for a response (default: 30 seconds).
    - `store: Option<Arc<Store>>` -- optional handle to the SQLite store for persistence and batch recall.

### Batch Approval

11. Batch approval covers a specific effect *type*, not a specific effect *instance*. The four effect types are: `FsRead`, `FsWrite`, `ProcessSpawn`, `NetworkConnect`. When the user selects "yes-all" for a `NetworkConnect` effect, all subsequent `NetworkConnect` effects in that session are auto-approved without prompting.

12. Batch decisions are scoped to a single session. They do not persist across process restarts. They are held in an in-memory `HashMap<(String, EffectType), ApprovalDecision>` keyed by `(session_id, effect_type)`.

13. Before prompting the user, the broker checks the batch map. If a batch decision exists for the `(session_id, effect_type)`, it returns immediately without prompting.

14. Batch decisions are also written to the `approvals` table in SQLite (if a store is configured) with `decided_by = "batch"` so the audit trail reflects that the decision was derived from a batch approval.

15. `EffectType` is an enum with variants `FsRead`, `FsWrite`, `ProcessSpawn`, `NetworkConnect` -- derived from `Effect` by discarding the payload. It implements `Hash`, `Eq`, `Clone`, `Copy`.

### Approval Persistence

16. Every approval decision (individual or batch-derived) is written to the `approvals` table in `grokrs-store`. The table schema (already defined in spec 02 as a future-extension table) is:
    ```sql
    CREATE TABLE approvals (
        id INTEGER PRIMARY KEY AUTOINCREMENT,
        session_id TEXT NOT NULL REFERENCES sessions(id),
        effect TEXT NOT NULL,
        decision TEXT NOT NULL,
        decided_at TEXT NOT NULL,
        decided_by TEXT,
        context TEXT,
        timeout_secs REAL
    );
    ```
    Columns added beyond the original schema: `context TEXT` (the context string passed to the broker) and `timeout_secs REAL` (the configured timeout at decision time, for audit). The migration extends the existing v1 `approvals` table or creates a v2 migration if v1 is already applied.

17. `store.approvals().record(session_id, effect, decision, decided_by, context, timeout_secs) -> Result<i64>` inserts an approval record and returns the row ID.

18. `store.approvals().list_by_session(session_id) -> Result<Vec<ApprovalRecord>>` returns all approval records for a session, ordered by `decided_at`.

19. `store.approvals().count_by_decision(session_id) -> Result<ApprovalCounts>` returns `{ approved: u64, denied: u64, timed_out: u64 }` for a session.

20. `effect` is stored as a human-readable string: `"FsRead(src/main.rs)"`, `"NetworkConnect(api.x.ai)"`, `"ProcessSpawn(cargo)"`, `"FsWrite(output.txt)"`. The format is `"{EffectType}({payload})"`.

21. `decision` is stored as `"Approved"`, `"Denied: <reason>"`, or `"Timeout"`.

22. `decided_by` values: `"user"` for interactive individual decisions, `"batch"` for batch-derived decisions, `"test"` for test stubs, `"timeout"` for timeout-derived decisions.

23. Persistence is best-effort. If the store is unavailable (e.g., not configured, disk full), the broker logs a warning and continues -- the approval decision itself is not affected by storage failure. The broker never blocks or fails an approval because of a storage error.

### Integration with grokrs-api Transport

24. The `PolicyGate` trait in `grokrs-api` currently returns `PolicyDecision::Ask`, which the transport maps to `TransportError::ApprovalRequired`. With the broker, the integration layer (in `grokrs-cli` or a new bridge module) intercepts `Ask` and delegates to the `ApprovalBroker` before the request proceeds.

25. A new `BrokeredPolicyGate` struct implements `PolicyGate`. It wraps an inner `PolicyEngine` and an `Arc<dyn ApprovalBroker>`. When the engine returns `Ask`, the gate calls `broker.request_approval()` and maps the result:
    - `Approved` -> `PolicyDecision::Allow`
    - `Denied { reason }` -> `PolicyDecision::Deny { reason }`
    - `Timeout` -> `PolicyDecision::Deny { reason: "approval timed out" }`

26. `BrokeredPolicyGate` lives in `grokrs-cli` (or a thin integration crate), not in `grokrs-api` or `grokrs-approval`. This preserves the compile-time isolation: `grokrs-api` knows only `PolicyGate`; `grokrs-approval` knows only `Effect` and `ApprovalBroker`; the CLI wires them together.

27. Because `PolicyGate::evaluate_network` is currently synchronous and `ApprovalBroker::request_approval` is async, `BrokeredPolicyGate` must bridge the sync/async boundary. The implementation uses `tokio::runtime::Handle::current().block_on()` inside `evaluate_network`. This is acceptable because the transport layer already runs inside a tokio runtime. If the project later makes `PolicyGate` async, this bridge is removed.

28. The `TransportError::ApprovalRequired` variant remains in the codebase for backward compatibility and for cases where no broker is configured. When a `BrokeredPolicyGate` is in use, `ApprovalRequired` is never surfaced -- the broker resolves it before the transport sees it.

### Integration with grokrs-cli

29. The CLI constructs a `TerminalBroker` at startup when `session.approval_mode` is `"interactive"` in the config.

30. The CLI passes the `TerminalBroker` (wrapped in `Arc`) to `BrokeredPolicyGate`, which is then injected into the `GrokClient` as the `PolicyGate` implementation.

31. When `session.approval_mode` is `"deny"`, no broker is constructed. The `PolicyGate` maps `Ask` directly to `Deny` (current behavior). When `approval_mode` is `"allow"` (for development/testing only), `Ask` maps directly to `Allow` with a warning logged.

32. The `doctor` subcommand reports the configured approval mode and whether a store is available for persistence.

33. New CLI subcommand: `grokrs store approvals [--session <id>]` -- lists approval decisions from the store, optionally filtered by session. Output includes timestamp, effect, decision, and decided-by.

### Timeout Handling

34. The default timeout is 30 seconds, configurable via `session.approval_timeout_secs` in the TOML config (new field in `SessionConfig`).

35. `AppConfig` in `grokrs-core` gains `approval_timeout_secs: Option<u64>` in `SessionConfig`. Default is 30 when absent.

36. The timeout is per-prompt, not per-session. Each individual approval request has its own timeout window.

37. On timeout, the broker returns `ApprovalDecision::Timeout`. The integration layer treats this as `Deny`. The decision is persisted with `decided_by = "timeout"`.

38. The timeout is implemented using `tokio::time::timeout` wrapping the stdin read future. No busy-wait or polling loops.

### Test Support

39. `AlwaysApproveBroker` -- a test stub that returns `Approved` for every request. Constructed with `AlwaysApproveBroker::new()`.

40. `AlwaysDenyBroker` -- a test stub that returns `Denied { reason: "test deny" }` for every request.

41. `RecordingBroker` -- a test stub that records all requests and returns a configurable sequence of decisions. Constructed with `RecordingBroker::new(vec![decision1, decision2, ...])`. Panics if more requests are made than decisions provided.

42. All test stubs implement `ApprovalBroker` and live in a `test_support` module behind `#[cfg(test)]` or a `test-support` feature flag.

### Configuration

43. Updated `configs/grokrs.example.toml`:
    ```toml
    [session]
    approval_mode = "interactive"    # "interactive", "deny", or "allow"
    approval_timeout_secs = 30       # per-prompt timeout in seconds
    transcript_dir = ".grokrs/sessions"
    ```

44. `approval_mode` values:
    - `"interactive"` -- use the `TerminalBroker` (default, production behavior).
    - `"deny"` -- all `Ask` decisions become `Deny` without prompting (CI/headless environments).
    - `"allow"` -- all `Ask` decisions become `Allow` without prompting (development only, logs a warning on every auto-approval).

## Safety Requirements

1. The broker never silently approves an effect. Every `Approved` decision requires either explicit user input (`y`/`yes`/`Y`/`yes-all`) or an explicit `"allow"` approval mode in config (which logs a warning per approval).

2. `Timeout` is always treated as `Deny`. No effect proceeds without explicit approval.

3. Batch approvals are scoped to a single session and do not persist across process restarts. A new process starts with an empty batch map regardless of what is in the database.

4. The broker does not have access to the `PolicyEngine` or `PolicyConfig` directly. It receives pre-classified `Effect` values and returns decisions. It cannot override `Allow` or `Deny` decisions from the engine -- only `Ask` flows through the broker.

5. The approval prompt writes to stderr, not stdout. This prevents approval prompts from being captured in piped output, which could lead to silent auto-approval if stdin/stdout are redirected.

6. The API key and other secrets never appear in the approval prompt's context string. The context describes the operation (e.g., "API call to api.x.ai") but never includes authentication details.

7. All approval decisions are persisted (when a store is available) for audit. The audit trail includes the session, the full effect description, the decision, who/what decided, and the configured timeout.

8. The `"allow"` approval mode is intended for development only. The CLI logs a warning on startup when this mode is active: `"WARNING: approval_mode=allow -- all Ask decisions will be auto-approved. Do not use in production."` Each individual auto-approval also logs a warning with the effect description.

9. Storage failures during persistence do not block or alter approval decisions. The broker's primary function (interactive approval) degrades gracefully when the store is unavailable.

10. The `BrokeredPolicyGate` is the only path through which `Ask` decisions reach the broker. There is no way to invoke the broker outside of the policy evaluation flow. Direct calls to `ApprovalBroker::request_approval` are possible (for testing) but the production path always goes through `BrokeredPolicyGate`.

## Deliverables

- `crates/grokrs-approval/` crate with:
  - `src/lib.rs` -- `ApprovalDecision`, `EffectType`, `ApprovalBroker` trait, re-exports
  - `src/terminal.rs` -- `TerminalBroker` implementation with stdin/stderr I/O, timeout, batch map
  - `src/brokered_gate.rs` -- `BrokeredPolicyGate` (or in `grokrs-cli` if preferred to avoid dependency on `grokrs-api` types)
  - `src/test_support.rs` -- `AlwaysApproveBroker`, `AlwaysDenyBroker`, `RecordingBroker`
- Updated `crates/grokrs-store/`:
  - `src/approval.rs` -- `record`, `list_by_session`, `count_by_decision` queries against the `approvals` table
  - Migration v2 extending the `approvals` table with `context` and `timeout_secs` columns (or creating the table if v1 migration created it as schema-only)
  - `src/types.rs` updated with `ApprovalRecord`, `ApprovalCounts`
- Updated `crates/grokrs-core/src/lib.rs`:
  - `SessionConfig` gains `approval_timeout_secs: Option<u64>`
- Updated `crates/grokrs-cli/`:
  - `BrokeredPolicyGate` wiring in the CLI startup path
  - `TerminalBroker` construction based on `approval_mode` config
  - `grokrs store approvals` subcommand
  - `doctor` updated to report approval mode and store availability
- Updated `configs/grokrs.example.toml` with `approval_timeout_secs`
- Updated `Cargo.toml` workspace members
- Tests: broker trait with test stubs (approve, deny, timeout, batch), terminal broker with simulated stdin (approve, deny, invalid input retry, timeout, batch yes-all, batch no-all), persistence round-trip (record and query approvals), `BrokeredPolicyGate` mapping (Ask->Approved->Allow, Ask->Denied->Deny, Ask->Timeout->Deny, Allow passthrough, Deny passthrough), config loading with and without `approval_timeout_secs`, batch scoping (different sessions do not share batch state)
