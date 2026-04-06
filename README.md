# grokrs

> **⚠️ ALPHA SOFTWARE — NOT READY FOR PRODUCTION USE**
>
> This project is in early alpha. APIs, configuration formats, and CLI interfaces
> are subject to breaking changes without notice. Use at your own risk.
> Feedback and bug reports are welcome.

`grokrs` is a safe, Rust-only CLI development agent for Grok-family model workflows.

The goal is not to mimic existing agent CLIs feature-for-feature. The goal is to provide a safer foundation:
- typed trust levels instead of ambient elevation
- rooted workspace paths instead of unchecked filesystem access
- fail-closed policy evaluation before shell, network, or write effects
- a small Cargo workspace with explicit crate boundaries
- repo-native guidance, review artifacts, and local `aivcs` / `sqry` surfaces

## Features

### Interactive Chat
Stream responses from the Grok API in a REPL with conversation history, session persistence, resume, and server-side search integration.

### Agentic Task Execution
Run single-shot tasks with a function-calling tool loop. The model can read files, write files, list directories, run commands, interact with git, remember context across sessions, and connect to MCP tool servers -- all gated by typed trust levels and a deny-by-default policy engine.

### Voice Agent
Interactive voice sessions via the xAI Voice Agent WebSocket API. Audio mode captures microphone input and plays back agent speech; text-only mode provides a text REPL over the same protocol.

### Headless / CI Mode
Run agent tasks non-interactively for CI/CD pipelines, GitHub Actions, and cron jobs. Structured JSON event output, deterministic exit codes, stdin piping, and configurable timeouts.

### Media Generation
Generate images and videos using Grok models. Images are downloaded to workspace-relative paths; video output is URL-only.

### Cost Reporting
Query accumulated token usage and cost breakdowns by model, day, session, or API endpoint. Output as table, JSON, or CSV with date-range filtering.

### MCP Tool Servers
Connect to Model Context Protocol servers to discover and use remote tools alongside built-in tools. Trust rank filtering applies to MCP tools the same way it does to built-in tools.

### Git Integration
Built-in git tools (`git_status`, `git_diff`, `git_add`, `git_commit`) powered by `git2`. Trust-ranked: status and diff are available at untrusted level, add requires interactive, commit requires admin.

### Agent Memory
Cross-session context via `remember`, `recall`, and `forget` tools. Memories persist in the SQLite store and are automatically injected into agent context on each run.

### Config Profiles
Layer environment-specific configuration on top of the base config. `--profile dev` loads `configs/grokrs.dev.toml` and merges it over the base. Profiles can also be set via `GROKRS_PROFILE`.

### OpenTelemetry Observability
Feature-gated OTLP tracing with span hierarchy covering session lifecycle, API calls, tool execution, and policy evaluation. Enable with `--features otel` at build time and `--otel-endpoint` at runtime.

### Prompt Caching
Server-side KV cache reuse via `--cache-key` on `chat` and `agent` commands. Reduces latency and effective input-token cost when the prompt prefix is stable across turns.

## Repository Layout

- `crates/grokrs-core`: shared config and core types
- `crates/grokrs-cap`: trust levels and workspace path capability types
- `crates/grokrs-policy`: effect classification and default deny policy
- `crates/grokrs-session`: session state model parameterized by trust level
- `crates/grokrs-tool`: tool trait, classification surface, built-in tools (file, git, memory)
- `crates/grokrs-api`: xAI Grok API client (Responses, Chat, Voice WebSocket, Models, Images, TTS, Embeddings)
- `crates/grokrs-store`: SQLite WAL persistence (sessions, transcripts, usage, approvals, memories, cost)
- `crates/grokrs-cli`: user-facing CLI (chat, agent, voice, generate, models, sessions, store, doctor)
- `docs/specs`: product and interface specs
- `docs/design`: architecture and boundary decisions
- `docs/ops`: bootstrap and local operation guidance
- `configs`: example configuration and profile overlays
- `examples`: example inputs and workflows
- `scripts`: bootstrap and validation helpers

## Installation

```bash
# Clone and build
git clone <repo-url> && cd grokrs
cargo build --release

# With optional features
cargo build --release --features audio   # Voice agent with microphone support
cargo build --release --features otel    # OpenTelemetry tracing
cargo build --release --features audio,otel  # Both
```

## Quick Start

```bash
# Verify your environment
grokrs doctor

# Interactive chat
grokrs chat
grokrs chat --search --stateful              # With web search and server-side history
grokrs chat --cache-key my-system-prompt     # With prompt caching

# Agent tasks
grokrs agent 'explain src/main.rs'           # Read-only task (untrusted)
grokrs agent 'fix the tests' --trust admin   # With shell access
grokrs agent --headless 'summarize changes'  # CI/CD mode (exit codes 0-4)
grokrs agent --headless --output json 'task' # Structured JSON event stream
echo 'task' | grokrs agent --headless        # Pipe task via stdin

# Voice
grokrs voice                                 # Interactive voice session
grokrs voice --text-only                     # Text-only mode (no audio I/O)
grokrs voice --voice rex --language de-DE    # Custom voice and language

# MCP tool servers
grokrs agent 'task' --mcp-server http://localhost:3000  # Ad-hoc MCP server
# Or configure persistent servers in [mcp.servers] config section

# Config profiles
grokrs --profile dev chat                    # Use configs/grokrs.dev.toml overlay
GROKRS_PROFILE=staging grokrs agent 'task'   # Profile via environment variable

# Media generation
grokrs generate image 'a cat on a keyboard' -o cat.png
grokrs generate video 'a sunset over mountains'

# Cost reporting
grokrs store cost --group-by model           # Cost breakdown by model
grokrs store cost --format json --since 2026-01-01  # JSON report with date filter
grokrs store cost --group-by day --until 2026-03-31 # Daily costs in a range

# Model discovery
grokrs models list
grokrs models pricing

# Session management
grokrs sessions list --active
grokrs sessions transcript <id>

# OpenTelemetry
grokrs --otel-endpoint http://localhost:4317 agent 'task'
GROKRS_OTEL_ENDPOINT=http://localhost:4317 grokrs chat
```

## Headless Exit Codes

When running `grokrs agent --headless`, the exit code indicates the outcome:

| Code | Meaning |
|------|---------|
| 0 | Success: task completed normally |
| 1 | Policy denied: a required effect was denied by policy |
| 2 | Timeout: execution exceeded the time limit (default 300s) |
| 3 | Tool error: a tool execution failed |
| 4 | Internal error: unexpected failure |

## Safety Position

This scaffold starts from a deny-by-default posture:
- network effects are denied by default
- shell spawning is denied by default
- writes are limited to validated workspace-relative paths
- trust escalation is explicit and typed
- git commit requires admin trust level
- MCP tool calls go through the same policy gates as built-in tools

## Configuration

Base configuration lives in `configs/grokrs.example.toml`. Key sections:

- `[workspace]` — workspace root and path constraints
- `[model]` — default model (grok-4), model overrides
- `[policy]` — effect allow/deny rules
- `[session]` — approval mode, session defaults
- `[api]` — xAI API base URL and key
- `[store]` — SQLite database path
- `[agent]` — max iterations, default trust, memory limit, search defaults
- `[chat]` — default model override, stateful mode, token limits
- `[mcp]` — MCP server definitions (URL, trust rank, timeout, label)

### Profiles

Create `configs/grokrs.NAME.toml` with any subset of config keys. Activate with:
```bash
grokrs --profile NAME <command>
# or
GROKRS_PROFILE=NAME grokrs <command>
```

Profile values merge on top of the base config. CLI flags override both.

## Local Agent Surfaces

- `.claude/settings.local.json` carries forward the useful local permissions shape seen in `shipctl`
- `.aivcsignore` and local `.aivcs/` state are ready for repository registration
- `.sqry/` is expected to be generated after scaffold completion
