# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## What This Is

`grokrs` is a safety-first Rust-only CLI scaffold for Grok-family model workflows. It prioritizes explicit trust boundaries, rooted filesystem access, and deny-by-default policy evaluation over feature breadth.

## Build and Test

```bash
cargo fmt --all                          # format all crates
cargo test                               # run all tests
cargo clippy --workspace --all-targets   # lint
cargo run -p grokrs-cli -- doctor        # smoke-test the CLI
```

Run a single crate's tests:
```bash
cargo test -p grokrs-policy
```

Run a single test by name:
```bash
cargo test -p grokrs-cap -- rejects_absolute_paths
```

CLI commands for manual verification:
```bash
cargo run -p grokrs-cli -- show-config configs/grokrs.example.toml
cargo run -p grokrs-cli -- eval read README.md
cargo run -p grokrs-cli -- eval network api.x.ai
```

Build with optional features:
```bash
cargo build --features audio             # Voice agent with microphone support
cargo build --features otel              # OpenTelemetry tracing
cargo build --features audio,otel        # Both
```

## Architecture

### Dependency Direction

```
grokrs-cli → grokrs-api     → grokrs-core
           → grokrs-store   → grokrs-core
           → grokrs-session → grokrs-cap → grokrs-core
           → grokrs-tool    → grokrs-cap → grokrs-core
           → grokrs-policy  → grokrs-cap → grokrs-core
```

Lower crates know nothing about the CLI. Safety primitives (`grokrs-cap`, `grokrs-core`) are leaf dependencies. `grokrs-api` depends only on `grokrs-core` (policy gate injected at runtime).

### Crate Responsibilities

- **grokrs-core** — `AppConfig` (TOML deserialization with `AgentConfig`, `ChatConfig`, `McpConfig` optional sections), `ConfigError`, `resolve_profile()`, `check_deprecated_model()`, shared types. All other crates depend on this for config structs.
- **grokrs-cap** — Trust levels (`Untrusted`, `InteractiveTrusted`, `AdminTrusted`) as sealed marker types, `WorkspaceRoot` (absolute), `WorkspacePath` (relative, non-escaping). Trust is a type parameter via `TrustLevel` trait, not a runtime flag.
- **grokrs-policy** — `Effect` enum (`FsRead`, `FsWrite`, `ProcessSpawn`, `NetworkConnect`), `Decision` enum (`Allow`, `Ask`, `Deny`), and `PolicyEngine` that evaluates effects against `PolicyConfig`.
- **grokrs-session** — `Session<T: TrustLevel>` with state machine: `Created -> Ready -> RunningTurn -> WaitingApproval -> Closed | Failed`.
- **grokrs-tool** — `Classify` trait (input -> effects), `ToolSpec` trait (name + execute + description + input_schema), `ToolRegistry` with trust-rank filtering. Built-in tools: `read_file`, `write_file`, `list_directory`, `run_command`, `git_status`, `git_diff`, `git_add`, `git_commit`, `remember`, `recall`, `forget`. Tools must declare their effects before execution.
- **grokrs-api** — xAI Grok API client: Responses API (streaming, function calling, tool loop, prompt caching), Chat Completions, Voice Agent (WebSocket transport), Models, Images, TTS, Embeddings, Tokenizer, Documents. `PolicyGate` trait for injected network gating. `BuiltinTool` and `SearchParameters` types for server-side search. MCP client for external tool server connectivity.
- **grokrs-store** — SQLite WAL-backed persistence: sessions, transcripts, token usage, approvals, memories, cost aggregation. V2 migration with `ON DELETE CASCADE`, `find_by_prefix`, and `memories` table. Cost queries with group-by and date-range filtering. Depends only on `grokrs-core` and `rusqlite`.
- **grokrs-cli** — Clap-based CLI: `chat` (interactive REPL with prompt caching), `agent` (agentic task execution with headless mode, MCP, git tools, memory), `voice` (interactive voice sessions via WebSocket), `generate` (image/video), `models` (list/info/pricing), `sessions` (list/show/transcript/clean), `api` (one-shot endpoints), `collections` (management API), `store` (status/usage/cost reporting), `doctor` (R2 feature status diagnostics), `show-config`, `eval`. Global flags: `--profile`, `--otel-endpoint`.

### Persistence Model

**TOML** for static configuration (read once at startup, human-edited). Supports profile overlays (`configs/grokrs.NAME.toml`) merged on top of the base config.
**SQLite WAL** for runtime state (sessions, transcripts, token usage, approvals, memories, evidence). Database at `.grokrs/state.db`, workspace-local, not version-controlled. See `docs/design/01_SQLITE_STATE.md`.

### Security Model

- **Trust is typed**: `Session<Untrusted>` and `Session<AdminTrusted>` are different types at compile time.
- **Paths are validated**: `WorkspacePath::new()` rejects absolute paths, `..` traversal, and empty paths. `WorkspaceRoot` requires an absolute path.
- **Effects before execution**: Every tool input must `Classify` into effects; `PolicyEngine::evaluate()` runs before any side effect.
- **Deny by default**: Network and shell are denied in the sample config. Even when enabled, shell and network get `Ask` (not `Allow`).
- **Agent trust gating**: Tools declare a `trust_rank()`. The `ToolRegistry` filters by rank before offering tools to the model. `PolicyGatedExecutor` evaluates every effect before execution.
- **MCP tool gating**: MCP tools inherit trust rank from their server config entry and go through the same `PolicyGatedExecutor` as built-in tools.
- **Approval mode**: `session.approval_mode` resolves `Ask` decisions: `"allow"` auto-approves (development only), `"deny"` auto-denies, `"interactive"` forwards to the approval broker (not yet implemented, effectively denies). Headless mode defaults to `"deny"`.

## Invariants

1. Trust level is a type parameter, not a boolean flag.
2. Filesystem effects use validated workspace-relative paths.
3. Policy owns effect decisions before dangerous operations.
4. Network and shell are denied by default in the sample configuration.
5. The repo remains Rust-only for core implementation code.
6. Keep crate boundaries clean -- small crate-local APIs over cross-cutting helpers.
7. Do not add non-Rust runtime dependencies for core execution paths without hard justification.
8. MCP tools go through the same policy gates as built-in tools.
9. Git operations are scoped to workspace root; paths are validated before execution.

## Configuration

TOML config at `configs/grokrs.example.toml`. Sections: `[workspace]`, `[model]`, `[policy]`, `[session]`, `[api]`, `[management_api]`, `[store]`, `[agent]`, `[chat]`, `[mcp]`. Loaded via `AppConfig::load()` in `grokrs-core`. `[agent]`, `[chat]`, and `[mcp]` are optional with serde defaults -- CLI args override config values.

### Config Profiles

Profiles overlay environment-specific settings on top of the base config:
- `--profile NAME` or `GROKRS_PROFILE=NAME` activates profile `configs/grokrs.NAME.toml`.
- Profile values merge over base config. CLI flags override both.
- Resolved via `resolve_profile()` in `grokrs-core`.

### Auth Provider Flow

- `api_key_env` remains the compatibility path for env-based auth.
- `[api.auth]` may declare a non-secret runtime provider such as Azure Key Vault.
- `management_key_env` remains the compatibility path for management API auth.
- `[management_api.auth]` may declare the same non-secret runtime provider pattern.
- Config stores only provider metadata like `vault_name` and `secret_name`, never the key value itself.
- Use `grokrs auth doctor`, `grokrs auth show-source`, and `grokrs auth test` to inspect auth safely.

### MCP Server Configuration

```toml
[mcp.servers.my-tools]
url = "http://localhost:8080/mcp"
label = "My Custom Tools"
trust_rank = 1
timeout_secs = 30
```

Ad-hoc servers via CLI: `grokrs agent --mcp-server http://localhost:8080/mcp`.

### Feature Flags

- `audio` — Enables microphone/speaker support for `grokrs voice`. Without it, `--text-only` is enforced.
- `otel` — Enables OpenTelemetry OTLP tracing. Requires `--otel-endpoint` or `GROKRS_OTEL_ENDPOINT` at runtime.

## CLI Commands

```bash
# Diagnostics
grokrs doctor                              # Feature status, store health, R2 readiness

# Chat
grokrs chat                                # Interactive REPL
grokrs chat --search --stateful            # REPL with web search, server-side history
grokrs chat --cache-key my-system-prompt   # With prompt caching
grokrs chat --resume <id>                  # Resume a previous session

# Agent
grokrs agent 'task description'            # Agentic task with tool execution
grokrs agent 'task' --trust admin --search # Admin tools + search
grokrs agent --headless 'task'             # CI/CD mode (no TTY, exit codes 0-4)
grokrs agent --headless --output json 'task'  # Structured JSON event stream
echo 'task' | grokrs agent --headless      # Pipe task via stdin
grokrs agent --mcp-server http://localhost:3000 'task'  # With MCP tools
grokrs agent 'task' --cache-key my-prompt  # With prompt caching

# Voice
grokrs voice                               # Interactive voice session
grokrs voice --text-only                   # Text-only mode (no audio I/O)
grokrs voice --voice rex --language de-DE  # Custom voice and language

# Media
grokrs generate image 'prompt' -o out.png  # Image generation
grokrs generate video 'prompt'             # Video generation (URL output)

# Models
grokrs models list                         # List models (default: grok-4)
grokrs models pricing                      # Pricing comparison

# Sessions
grokrs sessions list --active              # Active sessions
grokrs sessions transcript <id>            # View transcripts

# Store and Cost
grokrs store status                        # Database health
grokrs store cost --group-by model         # Cost breakdown by model
grokrs store cost --format json --since 2026-01-01  # JSON cost report
grokrs store cost --group-by day --session <id>     # Daily cost for a session

# Config Profiles
grokrs --profile dev chat                  # Use configs/grokrs.dev.toml overlay
GROKRS_PROFILE=staging grokrs agent 'task' # Profile via environment variable

# OpenTelemetry
grokrs --otel-endpoint http://localhost:4317 agent 'task'  # OTLP tracing

# Legacy
grokrs api chat 'prompt'                   # One-shot API call (deprecated)
```

## Key References

- `ARCHITECTURE.md` -- security model, workspace shape, WebSocket transport, MCP client, git tools, memory store
- `AGENTS.md` -- working standards and guardrails
- `docs/specs/` -- product and interface specs
- `docs/design/` -- architecture boundary decisions
