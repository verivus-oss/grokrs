# grokrs Architecture

`grokrs` is a safe Rust-only agent CLI intended to support Grok-class model workflows without inheriting the ambient-risk patterns common in agent shells.

## Objectives

- Make trust boundaries explicit in code.
- Keep the filesystem model rooted and non-escaping.
- Classify effects before execution.
- Keep the implementation modular enough to evolve into a larger agent runtime without a rewrite.

## Workspace Shape

```text
crates/
  grokrs-core      shared config (AppConfig, AgentConfig, ChatConfig, McpConfig) and summary types
  grokrs-cap       trust levels and rooted path validation
  grokrs-policy    effect model and deny-by-default evaluator
  grokrs-session   typed session lifecycle state
  grokrs-tool      tool classification, execution traits, ToolRegistry, git tools, memory tools
  grokrs-api       xAI Grok API client (Responses, Chat, Voice WebSocket, Models, Images, TTS, Embeddings)
  grokrs-store     SQLite WAL persistence (sessions, transcripts, usage, approvals, memories, cost)
  grokrs-cli       user-facing CLI (chat, agent, voice, generate, models, sessions, store, doctor)
```

## Dependency Direction

```
grokrs-cli → grokrs-api     → grokrs-core
           → grokrs-store   → grokrs-core
           → grokrs-session → grokrs-cap → grokrs-core
           → grokrs-tool    → grokrs-cap → grokrs-core
           → grokrs-policy  → grokrs-cap → grokrs-core
```

`grokrs-api` depends only on `grokrs-core` (policy gate injected at runtime, not compile-time).
`grokrs-store` depends only on `grokrs-core` and `rusqlite`.
The low-level safety model (`grokrs-cap`, `grokrs-policy`) remains independent of API and storage.

## Security Model

### Trust Levels

Trust is encoded as a type parameter:
- `Untrusted` — read-only tools: `read_file`, `list_directory`, `git_status`, `git_diff`, `recall`
- `InteractiveTrusted` — adds: `write_file`, `git_add`, `remember`, `forget`
- `AdminTrusted` — adds: `run_command`, `git_commit`

Future session transitions must require explicit elevation instead of ambient config-based trust.

### Workspace Paths

`WorkspaceRoot` holds the absolute repository root.
`WorkspacePath` accepts only relative, non-escaping paths.

The current scaffold validates syntax and ancestry. A later phase can replace the raw path wrapper with `cap-std` handles without changing higher-level policy logic.

### Policy

The default policy model classifies effects into:
- `FsRead`
- `FsWrite`
- `ProcessSpawn`
- `NetworkConnect`

The sample policy:
- allows reads inside the workspace
- allows writes only when configured
- denies shell spawning by default
- denies network access by default

### Session Model

Sessions are typed by trust level and track coarse lifecycle states:
- `Created`
- `Ready`
- `RunningTurn`
- `WaitingApproval`
- `Closed`
- `Failed`

This is intentionally small but gives a safe expansion path.

## State Persistence Model

Static configuration (workspace, model, policy, session, API settings) uses TOML files read at startup.

Runtime state uses SQLite WAL via `grokrs-store`:
- Session lifecycle state (durable across crashes)
- API request/response transcripts (append-only audit trail)
- Token usage and cost accumulation (queryable across sessions)
- Approval decisions and evidence records
- Agent memories (cross-session context with configurable limit)

Database location: `.grokrs/state.db` (workspace-local, not version-controlled).

See `docs/design/01_SQLITE_STATE.md` for the full ADR.

## Configuration Profiles

Profiles layer environment-specific overrides on top of a base config:

1. Base config loaded from `--config` path (default: `configs/grokrs.example.toml`).
2. Profile resolved from `--profile` flag or `GROKRS_PROFILE` env var.
3. Profile overlay loaded from `configs/grokrs.NAME.toml` and merged on top of base.
4. CLI flags override both base and profile values.

Profiles share the same schema as the base config. Any subset of keys can be overridden. This supports dev/staging/prod configurations, team-specific tool policies, and per-environment API endpoints.

## Interactive Chat (REPL)

The `grokrs chat` command provides an interactive REPL backed by the `ChatBackend` trait.

- **ChatBackend trait** (`repl/backend.rs`): Decouples the REPL loop from any concrete API client. Mock implementations enable full testing without network access.
- **GrokChatBackend** (`repl/grok_backend.rs`): Concrete backend that streams responses token-by-token via the xAI Responses API. Supports stateless and stateful (server-side chaining via `previous_response_id`) conversation modes.
- **Conversation modes**: Stateless (default, `store=false`) for local history; stateful (`--stateful`, `store=true`) for server-side context.
- **Session persistence**: Each chat session creates a store entry. Transcripts (request/response pairs) are logged per turn. Sessions can be resumed via `--resume <id>`.
- **Search integration**: `--search` and `--x-search` flags add server-side `BuiltinTool::WebSearch` / `BuiltinTool::XSearch` to the request tools array. Citations from search results are extracted and displayed as numbered references after the response text.
- **Prompt caching**: `--cache-key KEY` sends `prompt_cache_key` on every API request, enabling server-side KV cache reuse for the matched prefix. Per-turn usage reports include cached token counts.
- **Encrypted reasoning**: When `include: ["reasoning.encrypted_content"]` is configured, encrypted reasoning tokens are captured and automatically replayed in stateful conversations.
- **Slash commands**: `/exit`, `/clear`, `/model`, `/system`, `/help`, `/history` for in-REPL control.

### Security: Chat

Chat requires `allow_network=true` in policy and an approval mode that resolves `Ask` decisions. The `check_network_policy()` function fails fast with actionable instructions before any API call.

## Agent Framework

The `grokrs agent` command runs a single agentic task with a function-calling loop.

- **ToolRegistry** (`grokrs-tool/src/registry.rs`): Flat registry of `ErasedTool` wrappers, each with a `trust_rank()`. Tools are filtered by the session's trust level before being offered to the model.
- **Trust-level gating**: `untrusted` (rank 0) gets `read_file`, `list_directory`, `git_status`, `git_diff`, `recall`. `interactive` (rank 1) adds `write_file`, `git_add`, `remember`, `forget`. `admin` (rank 2) adds `run_command`, `git_commit`.
- **PolicyGatedExecutor** (`agent/executor.rs`): Implements `FunctionExecutor`. Every tool call is classified -> policy-evaluated -> executed. Approval mode controls `Ask` resolution.
- **AgentExecutor flow**: Initial prompt with tool definitions -> model responds with function calls -> executor dispatches -> results fed back -> loop until text response or `max_iterations`.
- **Search in agent**: `--search` / `--x-search` flags add `BuiltinTool` variants alongside function tool definitions. The tool loop handles server-side search results as pass-through items (no local dispatch).
- **Agent memory**: `remember`, `recall`, and `forget` tools allow the agent to persist context across sessions. On each run, existing memories are automatically loaded and injected into the system prompt. Memory count is limited by `agent.memory_limit` config (default 50).
- **MCP tool integration**: Tools discovered from MCP servers are registered alongside built-in tools with the same trust-rank filtering and policy gating.
- **Prompt caching**: `--cache-key KEY` sends `prompt_cache_key` on the initial API request, enabling server-side KV cache reuse.

### Security: Agent

The agent's trust level is a type parameter at the tool level. Tools declare their required trust rank via `trust_rank()`. The `PolicyGatedExecutor` evaluates every effect against the policy engine before execution -- even `admin` tools go through policy gates for `ProcessSpawn` and `FsWrite` effects. MCP tools inherit the trust rank configured on their server entry and go through the same policy evaluation path.

### Headless Mode

`grokrs agent --headless` runs without TTY interaction for CI/CD pipelines:

- **No interactive I/O**: No color, no spinners, no approval prompts.
- **Structured output**: `--output json` emits newline-delimited JSON events (`tool_call`, `tool_result`, `message`, `error`, `usage`) to stdout. Diagnostics go to stderr.
- **Deterministic exit codes**: 0 (success), 1 (policy denied), 2 (timeout), 3 (tool error), 4 (internal error).
- **Stdin piping**: Task can be provided via stdin when no positional argument is given.
- **Timeout**: Defaults to 300s in headless mode (no timeout in interactive mode). Override with `--timeout`.
- **Approval mode**: Defaults to `deny` in headless mode (fail-closed). Override with `--approval-mode allow` for trusted pipelines.

## Voice Agent

The `grokrs voice` command provides interactive voice sessions via the xAI Voice Agent API.

### WebSocket Transport

Voice sessions use a WebSocket connection to the xAI Voice Agent endpoint. The transport layer (`grokrs-api/src/transport/websocket.rs`) provides:

- **WsClientConfig**: Connection configuration (URL, auth headers, ping interval).
- **Binary and text frames**: Audio data flows as binary frames; control messages and transcripts flow as JSON text frames.
- **Reconnection**: The transport supports clean shutdown and reconnection for session continuity.

### Audio and Text Modes

- **Audio mode** (default, requires `audio` feature): Captures microphone audio via `cpal`, streams it to the voice agent as PCM frames, and plays back agent speech through the default output device. VAD (voice activity detection) with configurable sensitivity handles turn detection.
- **Text-only mode** (`--text-only`): Text REPL that sends text messages to the voice agent and displays transcripts. Works without audio dependencies and is automatically selected when the `audio` feature is not compiled in.

### Voice Configuration

- `--voice`: Agent voice selection (eve, ara, rex, sal, leo)
- `--language`: BCP 47 language code (en-US, de-DE, etc.)
- `--turn-detection`: server_vad (default) or manual
- `--vad-sensitivity`: low, medium, high
- `--system`: System instructions for the voice agent
- `--max-duration`: Maximum session duration in seconds

## Search Integration

Search is a cross-cutting enhancement available on both `chat` and `agent` commands.

- **Server-side tools**: `BuiltinTool::WebSearch` and `BuiltinTool::XSearch` are server-side -- no local HTTP calls for search. The xAI API executes the search and returns results in the response output.
- **SearchParameters**: Optional date range, max results, and citation control passed at the request top level.
- **Citation display**: Citations extracted from `ResponseCompleted` events (chat) or `OutputItem::WebSearchCall` / `OutputItem::XSearchCall` (agent) are formatted as numbered references.
- **No additional policy effects**: Search is server-side, so only `NetworkConnect` is required (already needed for the API call itself).

## MCP Client

The agent framework includes a Model Context Protocol (MCP) client for connecting to external tool servers.

### Configuration

MCP servers are configured in the `[mcp.servers]` TOML section:

```toml
[mcp.servers.my-tools]
url = "http://localhost:8080/mcp"
label = "My Custom Tools"
trust_rank = 1        # 0=untrusted, 1=interactive, 2=admin
timeout_secs = 30
```

Alternatively, ad-hoc servers can be specified on the command line: `--mcp-server http://localhost:8080/mcp`.

### Tool Discovery

On session startup, the MCP client connects to each configured server, performs the `tools/list` handshake, and registers discovered tools in the `ToolRegistry`. Each MCP tool inherits:

- **Trust rank**: From the server's `trust_rank` config (controls which trust levels can invoke it).
- **Policy gating**: Every MCP tool call goes through the same `PolicyGatedExecutor` as built-in tools.
- **Timeout**: Per-server timeout prevents hung connections from blocking the tool loop.

### Security: MCP

MCP tools are not trusted by default. Each server entry declares a `trust_rank` that governs which session trust levels can access its tools. The `PolicyGatedExecutor` classifies MCP tool effects and evaluates them against the policy engine before execution. Servers without explicit trust rank default to rank 1 (interactive).

## Git Tools

Built-in git tools provide version control operations within the agent framework, powered by `git2` (libgit2 bindings):

| Tool | Trust Rank | Effects | Description |
|------|-----------|---------|-------------|
| `git_status` | 0 (untrusted) | `FsRead` | Show working tree status |
| `git_diff` | 0 (untrusted) | `FsRead` | Show unstaged or staged changes |
| `git_add` | 1 (interactive) | `FsWrite` | Stage files for commit |
| `git_commit` | 2 (admin) | `FsWrite` | Create a commit with a message |

All git operations are scoped to the workspace root. Path arguments are validated as workspace-relative paths before any git operation executes.

## Agent Memory Store

Agent memory provides cross-session context persistence:

- **remember**: Store a key-value memory entry. Persisted in the SQLite `memories` table.
- **recall**: Retrieve memories by key prefix or list all. Automatically injected into the agent system prompt at session start.
- **forget**: Delete a memory entry by key.

Memory entries are scoped to the workspace and survive across agent sessions. The `agent.memory_limit` config (default 50) caps the number of memories stored; oldest entries are evicted when the limit is reached.

## Media Generation

- **Image generation**: `grokrs generate image` -- POST to `/v1/images/generations`, downloads the result to a workspace-relative output path. Policy-gated for `FsWrite` on the output path.
- **Video generation**: `grokrs generate video` -- POST to create, then polling until completion. Output is URL-only (no download). Supports `--extend` for video extension.

## Model Discovery

- **`grokrs models list`** -- Lists language, image, or video models from the xAI API. Table or JSON output.
- **`grokrs models info <id>`** -- Detailed model information.
- **`grokrs models pricing`** -- Pricing comparison table sorted by cost.

The default model is `grok-4`. Deprecated model names (`grok-2`, `grok-3`) trigger a warning at startup. The `doctor` command includes a model freshness check.

## Session Management

- **`grokrs sessions list`** -- All sessions with state, trust level, timestamps, transcript count. Filters: `--active`, `--state`.
- **`grokrs sessions show <id>`** -- Full session details with prefix matching on ID.
- **`grokrs sessions transcript <id>`** -- Full request/response transcript.
- **`grokrs sessions clean`** -- Deletes old Closed/Failed sessions. Configurable retention via `--older-than`.

### Store Extensions (V2 Migration)

The V2 store migration adds `ON DELETE CASCADE` for session-transcript relationships, `find_by_prefix` for session ID prefix matching, and the `memories` table for agent memory persistence.

## Cost Reporting

`grokrs store cost` provides queryable cost breakdowns from the transcript store:

- **Group by**: model, day, session, or API endpoint.
- **Output formats**: table (human-readable), JSON, CSV.
- **Date filtering**: `--since` and `--until` for date-range queries.
- **Session filtering**: `--session <id>` for per-session cost analysis.

Cost data is accumulated from token usage recorded in each API transcript entry.

## OpenTelemetry Observability

Feature-gated behind the `otel` Cargo feature. When enabled and an endpoint is configured, grokrs emits OpenTelemetry traces via OTLP:

- **Configuration**: `--otel-endpoint URL` flag or `GROKRS_OTEL_ENDPOINT` env var.
- **Span hierarchy**: Root span per CLI invocation, child spans for session lifecycle, API requests, tool execution, policy evaluation, and MCP client calls.
- **Attributes**: Spans carry structured attributes (model name, trust level, tool name, effect type, decision outcome, token counts).
- **Flush on drop**: The telemetry guard ensures all pending spans are exported before process exit.

When the `otel` feature is not compiled in, all tracing calls are no-ops with zero runtime cost.

## Approval Mode

`approval_mode` in `[session]` config is a temporary escape hatch for `Decision::Ask` resolution:

- `"interactive"` (default): Forward to the approval broker. Until the broker is implemented (spec 03), this effectively denies.
- `"deny"`: Automatically deny all Ask decisions. Fail-closed.
- `"allow"`: Automatically approve all Ask decisions. **WARNING: bypasses the approval boundary.** Development use only.

Only `Ask` decisions are affected -- `Allow` and `Deny` from the policy engine pass through unchanged.

In headless mode, approval defaults to `deny` unless explicitly overridden with `--approval-mode`.

## Planned Expansion

- approval broker crate (storage via `grokrs-store`)
- transcript and evidence store (storage via `grokrs-store`)
- sandbox profiles
- audit log export into `aivcs` (queryable from `grokrs-store`)
- `sqry`-backed symbol-aware context assembly
