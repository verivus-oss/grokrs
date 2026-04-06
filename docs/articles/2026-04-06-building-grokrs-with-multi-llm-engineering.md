# Building grokrs: Multi-LLM Engineering at Production Scale

*2026-04-06*

## What We Built

**grokrs** is a safety-first Rust CLI for xAI's Grok API. Not a wrapper — a full agent runtime with typed trust levels, policy-gated tool execution, server-side search integration, media generation, and persistent session management. 39,000 lines of Rust across 8 crates, 101 source files, and 1,223 tests.

The competitive features implementation — the focus of this article — added interactive chat with streaming, an agentic tool-calling loop, web and X search, image and video generation, model discovery, and session management. All built on a deny-by-default security model where trust is a compile-time type parameter, not a runtime flag.

## The Architecture

```
grokrs-cli → grokrs-api     → grokrs-core
           → grokrs-store   → grokrs-core
           → grokrs-session → grokrs-cap → grokrs-core
           → grokrs-tool    → grokrs-cap → grokrs-core
           → grokrs-policy  → grokrs-cap → grokrs-core
```

Eight crates with strict dependency direction. Lower crates know nothing about the CLI. The safety primitives (`grokrs-cap` for trust levels and path validation, `grokrs-policy` for effect evaluation) are leaf dependencies that never import upward.

Key design decisions:

- **Trust as types**: `Session<Untrusted>` and `Session<AdminTrusted>` are different types at compile time. You can't accidentally give an untrusted session admin tools.
- **Effects before execution**: Every tool call classifies its effects (`FsRead`, `FsWrite`, `ProcessSpawn`, `NetworkConnect`), and the policy engine evaluates them before any side effect occurs.
- **Policy injection, not dependency**: `grokrs-api` takes a `PolicyGate` trait object at runtime. It never imports `grokrs-policy` at compile time. The CLI wires them together.

## The DAG: How We Planned the Work

The 21 units of work were organized in a TOML-based implementation DAG (`docs/reviews/competitive-features/2026-04-06/IMPLEMENTATION_DAG.toml`) with explicit dependency tracking:

```
Section 1: Foundation (U09-U12, U23)     — Approval bypass, tool traits, REPL, trust gates, store
Section 2: Backend+Registry (U13-U14)    — Grok chat backend, tool registry
Section 3: Features (U15-U16, U18, U20-U24) — Agent executor, chat command, agent command, media, models, sessions
Section 4: Search (U17)                  — Search integration across chat and agent
Section 5: Polish (U30-U35)              — Integration tests, doctor, help text, config, docs, deprecation
```

Each unit had:
- **Acceptance criteria**: Specific, testable conditions (e.g., "ReadFileTool rejects absolute paths at classification time")
- **Dependency graph**: `depends_on` and `blocks` fields ensuring correct implementation order
- **Critical decisions**: Documented rationale for non-obvious choices
- **Failure modes**: Anticipated edge cases and mitigations
- **LOC estimates**: Proved accurate within ~15%

The DAG wasn't just a plan — it was a contract. Every unit was implemented against its acceptance criteria, and reviews verified compliance.

## The Tools: Multi-LLM Engineering

This project used three AI models in distinct roles, orchestrated through a custom LLM gateway:

### Claude Opus 4.6 — Primary Engineer

Claude wrote all the code, managed the implementation order, ran tests, and iterated on review feedback. Working in Claude Code with access to the full codebase, it maintained context across the entire 8-crate workspace while implementing units that touched multiple crates simultaneously.

Key capabilities leveraged:
- **Parallel tool calls**: Reading multiple files, running tests, and checking compilation simultaneously
- **Background agents**: Spawning subagents for integration test generation while continuing other work
- **MCP server integration**: `sqry` for semantic code search, `aivcs` for version control, `llm-gateway` for multi-LLM orchestration

### OpenAI Codex (gpt-5.4) — Code Reviewer

Codex reviewed every unit after implementation, with access to the codebase via `sqry` semantic search (since its sandbox blocked shell access). It operated in a strict review role — no code changes, only findings.

Codex's reviews were substantive. Across 3 review rounds it found:

1. **Round 1**: A test that only verified a duplicated string literal instead of exercising the real dispatch path. The deprecation notice test asserted on a locally copied string, meaning it would pass even if the actual `eprintln!` was removed.

2. **Round 2**: Config sections (`[agent]`, `[chat]`) that parsed and displayed in summaries but weren't wired into runtime behavior. Also caught a `&sys[..50]` byte slice that could panic on non-ASCII system prompts — a valid UTF-8 boundary violation.

3. **Round 3**: Sentinel-based override detection (`args.trust != "untrusted"`) that couldn't distinguish "user explicitly passed `--trust untrusted`" from "using the default." Also found the same UTF-8 slicing pattern in `summarize_tool_call()`.

Every finding was legitimate. Every finding was fixed before approval.

### Google Gemini — Architecture Reviewer

Gemini performed broader architectural reviews, verifying coherence across the full crate structure, dependency direction, and security model integrity. Its final comprehensive review walked through all 8 crates, verified the dependency graph, examined the `PolicyGate` injection pattern, and confirmed that documentation matched implementation.

Gemini's verdict: **PASS — Production-ready.** It specifically called out the `PolicyGate` trait injection as a "standout design choice" and validated the trust-rank tool filtering, path validation, and secret handling.

### The Review Loop

The review process was iterative and non-ceremonial:

```
Implement unit → cargo test + clippy + fmt → Submit to Codex + Gemini (parallel)
     ↓                                              ↓
  Fix findings ←────────────────────────── Review findings
     ↓
  Resubmit → Approval (or another round)
```

Reviews ran in parallel with implementation of the next unit. While Codex was reviewing U35+U33, Claude was implementing U32 and U31. While Codex was reviewing the fixes, the background agent was writing 21 integration tests. No idle time.

## The Implementation: What Each Section Delivered

### Section 4: Search Integration (U17)

Search is a cross-cutting enhancement available on both `grokrs chat` and `grokrs agent`. The implementation added:

- 7 new CLI flags on `chat` (`--search`, `--x-search`, `--no-search`, `--search-from-date`, `--search-to-date`, `--search-max-results`, `--citations`)
- 3 new CLI flags on `agent` (`--search`, `--x-search`, `--citations`)
- Shared `SearchConfig` module with `BuiltinTool` construction, `SearchParameters` building, and citation formatting
- Citation extraction from both `ResponseCompleted` events (streaming chat) and `OutputItem::WebSearchCall`/`XSearchCall` (agent tool loop)
- Date validation (ISO 8601) with helpful error messages

The key insight: search tools are server-side (`BuiltinTool::WebSearch`), not client-side. They go into the request's `tools` array alongside function tool definitions, and the model decides when to invoke them. The agent's `run_tool_loop` already handled server-side tool results as pass-through items — no special agent-side handling needed beyond including the `BuiltinTool` in the request.

### Section 5: Polish (U30-U35)

The polish layer turned a working implementation into a complete product:

**U35 — Deprecation notice** (30 LOC): `grokrs api chat` now prints a notice to stderr recommending `grokrs chat` for interactive use. The notice is emitted before client construction so it appears even when the API key is missing. Verified by an integration test that captures real stderr/stdout via `std::process::Command`.

**U33 — Config extensions** (180 LOC): `[agent]` and `[chat]` optional TOML sections with serde defaults. CLI args override config values using `Option`-based resolution (not sentinel defaults — Codex caught this). UTF-8 safe truncation in all display paths.

**U32 — Help text** (200 LOC): `after_help` examples on every command. Integration tests verify the `Examples:` section is present. Running `grokrs --help` now shows a complete command tree with realistic usage examples.

**U31 — Doctor command** (180 LOC): `grokrs doctor` reports ready/blocked status for every feature with actionable fix instructions. Each feature's readiness is config-based (no API calls) — doctor works offline.

**U30 — Integration tests** (950 LOC): 21 new tests across 3 files:
- `models_tests.rs` — 7 tests with wiremock for list, info, pricing, JSON output, image models, network denial
- `sessions_tests.rs` — 7 tests with real SQLite store for list, show, transcript, clean, prefix match, active filter
- `generate_tests.rs` — 7 tests with wiremock for image download, video polling, policy denial, path validation

**U34 — Documentation** (300 LOC): ARCHITECTURE.md and CLAUDE.md updated to reflect the full feature surface, security implications, and dependency structure.

## The Numbers

| Metric | Value |
|--------|-------|
| Rust source files | 101 |
| Lines of Rust | 39,133 |
| Crates | 8 |
| Tests passing | 1,223 |
| Clippy warnings | 0 |
| DAG units | 21 |
| Codex review rounds | 3 (all findings addressed) |
| Codex findings fixed | 5 |
| Gemini verdict | PASS |
| Integration test files | 4 |
| CLI commands | 12 (doctor, show-config, eval, chat, agent, generate, models, api, collections, sessions, store, help) |

## What We Learned

### Multi-LLM review works, but the roles matter

Using different models for implementation and review creates genuine adversarial tension. Codex found real bugs that Claude missed — the sentinel-default override issue is the kind of subtle logic error that passes all tests but fails in specific user scenarios. The key is giving each model a clear role: Claude implements, Codex reviews code, Gemini reviews architecture.

### DAG-driven implementation prevents scope creep

The TOML DAG with explicit acceptance criteria, dependencies, and LOC estimates kept the work focused. When implementing U17 (search), there was no temptation to add "just one more feature" because the acceptance criteria were specific and the next unit (U30) was waiting.

### Integration tests via `std::process::Command` catch real bugs

Unit tests with mock backends verify logic. Integration tests that actually run the binary catch wiring issues — like the deprecation notice that was placed after client construction (never reached when the API key was missing). The `cli_smoke.rs` pattern of creating tempfile configs and running the real binary proved invaluable.

### UTF-8 safety requires vigilance in Rust

Rust's string slicing (`&s[..50]`) panics on non-ASCII boundaries. This was caught twice — in `AppConfig::summary()` and in `summarize_tool_call()`. The fix is a 6-line `truncate_utf8()` helper that finds the nearest char boundary. Easy to write, easy to forget.

### Option-based CLI resolution > sentinel defaults

Clap's `default_value` makes it impossible to distinguish "user passed the default" from "not provided." Changing `--trust` from `String` with `default_value = "untrusted"` to `Option<String>` with runtime fallback was a small change with significant correctness implications for config-file integration.

## The Stack

- **Language**: Rust (exclusively — no non-Rust runtime dependencies)
- **Primary AI**: Claude Opus 4.6 (1M context) via Claude Code CLI
- **Code review**: OpenAI Codex gpt-5.4 via llm-gateway MCP
- **Architecture review**: Google Gemini via llm-gateway MCP
- **Semantic search**: sqry MCP server (AST-based code search)
- **Version control**: aivcs (AI-native VCS with episode tracking and symbol indexing)
- **HTTP mocking**: wiremock 0.6
- **CLI framework**: clap (derive)
- **Persistence**: SQLite WAL via rusqlite
- **Streaming**: Server-Sent Events via reqwest + futures
- **Serialization**: serde + serde_json + toml

## Conclusion

grokrs demonstrates that multi-LLM engineering can produce production-grade software — not by having AI models rubber-stamp each other's work, but by assigning genuine adversarial roles. The implementation model writes code. The review model finds bugs. A third model validates architecture. Each model catches things the others miss.

The result is a 39K-line Rust codebase with 1,223 tests, zero clippy warnings, a deny-by-default security model, and typed trust boundaries — built and reviewed in a single engineering session.
