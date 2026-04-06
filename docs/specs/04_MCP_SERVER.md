# MCP Server Surface Spec

Date: 2026-04-05

## Summary

This spec defines `grokrs-mcp`, a new crate that exposes grokrs as a Model Context Protocol (MCP) server. MCP clients (Claude Code, Cursor, Zed, custom agents) connect to grokrs and invoke its tools, read its workspace resources, and retrieve its prompts -- all gated through the existing `PolicyEngine` and trust-level model. This is the inverse of a planned future feature where grokrs acts as an MCP *client* calling external MCP servers. Here, grokrs IS the server.

The crate implements the MCP specification (2024-11-05 revision or later) over stdio and HTTP+SSE transports. It maps `grokrs-tool` `ToolSpec` instances to MCP tool definitions, workspace files to MCP resources, and system instructions to MCP prompts. Every tool invocation passes through `PolicyEngine::evaluate` before execution. MCP client connections are assigned a trust level from configuration, creating a `Session<T>` for each connection.

`grokrs-mcp` depends on `grokrs-core` (config), `grokrs-policy` (effect evaluation), `grokrs-cap` (trust levels, workspace paths), `grokrs-session` (session lifecycle), `grokrs-tool` (tool traits), and optionally `grokrs-store` (session and approval persistence). It does not depend on `grokrs-api` (the xAI client is a separate concern).

## Goals

- Expose grokrs capabilities to any MCP-compliant client without requiring the client to understand grokrs internals.
- Gate every tool invocation through the existing effect classification and policy evaluation pipeline.
- Map MCP client connections to typed trust levels so the security model applies uniformly whether the user is at the CLI or connected via MCP.
- Expose workspace files as read-only MCP resources bounded by `WorkspacePath` validation.
- Support both stdio (single-client, launched as subprocess) and HTTP+SSE (multi-client, long-running) transports.
- Persist MCP sessions in SQLite via `grokrs-store` for audit and crash recovery.

## Non-Goals

- grokrs as an MCP *client* (calling external MCP servers) -- that is a separate spec for remote MCP tool integration.
- Proxying MCP requests to the xAI API -- the MCP server exposes grokrs tools, not raw API passthrough.
- MCP Sampling (server-initiated LLM calls) -- grokrs does not initiate model inference on behalf of connected clients in this spec.
- OAuth 2.1 / PKCE authentication for HTTP+SSE transport -- deferred to a follow-up security hardening pass. Initial HTTP+SSE uses a shared bearer token from config.
- Custom MCP protocol extensions beyond the base specification.
- Bidirectional tool calling (client tools exposed to server) -- the server exposes tools; it does not call client-provided tools.

## Functional Requirements

### MCP Protocol Compliance

1. Implement the MCP base protocol: `initialize`, `initialized`, `ping`, `notifications/cancelled`, capability negotiation. The server advertises `tools`, `resources`, and `prompts` capabilities during initialization.

2. Protocol version negotiation: the server supports MCP protocol version `2024-11-05` and later. If a client requests an unsupported version, the server responds with the latest supported version per the MCP spec.

3. JSON-RPC 2.0 message framing over all transports. Request IDs are opaque (string or integer). Batch requests are not supported (per MCP spec).

4. The server implementation is `ServerInfo { name: "grokrs", version: <crate version> }`.

### Transport: stdio

5. stdio transport: the MCP server reads JSON-RPC messages from stdin and writes responses to stdout, one message per line (newline-delimited JSON). This is the default transport for single-client subprocess usage (e.g., Claude Code launching grokrs as a child process).

6. Diagnostic and log output goes to stderr exclusively. stdout is reserved for MCP protocol messages. This aligns with the existing grokrs convention of stderr for human-readable output.

7. The server runs until stdin closes (EOF) or a `notifications/cancelled` for the session is received.

8. CLI entrypoint: `grokrs mcp stdio` starts the MCP server on stdio. It loads the config, constructs the policy engine, registers tools, and enters the message loop.

### Transport: HTTP+SSE

9. HTTP+SSE transport: the server listens on a configurable `host:port` (default `127.0.0.1:3000`). Clients connect via `GET /sse` to establish a Server-Sent Events stream for server-to-client messages, and send client-to-server messages via `POST /message`.

10. Each SSE connection is a separate MCP session. The server assigns a unique session ID and creates a `Session<T>` for it.

11. The SSE endpoint sends an initial `endpoint` event with the URL for the client to POST messages to (per MCP HTTP+SSE spec).

12. Connection lifecycle: the SSE stream stays open for the duration of the session. If the client disconnects, the session transitions to `Closed`. Reconnection creates a new session.

13. CLI entrypoint: `grokrs mcp serve [--host <host>] [--port <port>]` starts the HTTP+SSE server. Requires `allow_network = true` in policy config (the server itself is a network listener).

14. The HTTP server is built on `axum` (already a transitive dependency via `tokio`). No additional web framework dependencies.

### Tool Exposure

15. Every `ToolSpec` implementation registered with the MCP server is exposed as an MCP tool. The mapping:
    - MCP tool `name` = `ToolSpec::name()`
    - MCP tool `description` = derived from the tool's documentation or a `description()` method added to `ToolSpec`
    - MCP tool `inputSchema` = JSON Schema describing `ToolSpec::Input` (derived via `schemars::JsonSchema` on the input type)

16. `ToolSpec` trait gains two new methods with default implementations:
    ```rust
    fn description(&self) -> &'static str { "" }
    fn input_schema(&self) -> serde_json::Value { serde_json::json!({}) }
    ```
    Tools that want MCP exposure override these. Tools without overrides get an empty description and accept-anything schema (effectively hidden from useful MCP discovery, but still callable).

17. The `tools/list` MCP method returns the list of exposed tools with their names, descriptions, and input schemas. Only tools listed in the `[mcp] exposed_tools` config are included. If `exposed_tools` is empty or absent, all registered tools are exposed.

18. The `tools/call` MCP method:
    a. Deserializes the `arguments` JSON into the tool's `Input` type.
    b. Calls `input.classify()` to get the list of `Effect` values.
    c. Evaluates each effect through `PolicyEngine::evaluate()`.
    d. If any effect returns `Deny`, the tool call fails with an MCP error response containing the denial reason.
    e. If any effect returns `Ask`, the approval broker is invoked (if configured). If the broker returns `Denied` or `Timeout`, the tool call fails.
    f. If all effects are `Allow` (or approved via broker), the tool's `execute()` method is called.
    g. The result is serialized and returned as the MCP tool call response.

19. Tool call errors (policy denial, execution failure, deserialization failure) are returned as MCP error responses with appropriate error codes. Policy denials use a custom error code (`-32001`) with the denial reason in the error data.

20. Tool execution is bounded by a configurable timeout (`mcp.tool_timeout_secs`, default 60). If a tool exceeds the timeout, the call is cancelled and an error response is returned.

### Effect Gating

21. Every MCP tool call goes through the full effect classification and policy evaluation pipeline. There is no "MCP bypass" path. The security model is identical whether a tool is invoked from the CLI or from an MCP client.

22. The `PolicyEngine` instance used for MCP evaluation is the same instance used by the CLI. Configuration changes (e.g., `allow_network`) apply uniformly.

23. Effects are evaluated before execution, never after. A tool that declares `[FsRead("src/main.rs"), NetworkConnect("api.x.ai")]` must have both effects approved before any part of the tool executes.

24. MCP tool calls that trigger `Ask` effects and have no approval broker configured fail with an error message stating that interactive approval is required but no broker is available. This is consistent with the current behavior for `Decision::Ask` throughout the system.

### Trust Level Mapping

25. Each MCP transport maps to a trust level from the `[mcp]` config section:
    ```toml
    [mcp]
    trust_level = "InteractiveTrusted"
    ```
    Valid values: `"Untrusted"`, `"InteractiveTrusted"`, `"AdminTrusted"`. Default is `"Untrusted"` when absent.

26. The trust level determines the `Session<T>` type for each MCP connection. An `"Untrusted"` config creates `Session<Untrusted>`, etc.

27. Trust level affects which tools are available and what effects are permitted. The policy engine evaluates effects in the context of the session's trust level. (The current `PolicyEngine` does not yet differentiate by trust level -- this spec defines the config surface; trust-level-aware policy evaluation is a future enhancement that builds on this foundation.)

28. The trust level is per-server-instance, not per-client. All clients connecting to the same MCP server share the same trust level. Per-client trust differentiation is deferred to the OAuth authentication follow-up.

### Resource Exposure

29. Workspace files are exposed as MCP resources. The `resources/list` method returns resources for files under the workspace root, bounded by `WorkspacePath` validation.

30. Resource URIs follow the pattern `file:///<workspace-relative-path>` (e.g., `file:///src/main.rs`, `file:///README.md`). The triple-slash indicates a local file path.

31. `resources/read` validates the requested path through `WorkspacePath::new()` before reading. Paths that are absolute, contain `..` traversal, or are empty are rejected with an MCP error.

32. Resources are read-only. The MCP server does not expose `resources/write` or any mutation endpoint for resources. File writes go through tools that declare `FsWrite` effects.

33. Resource listing is bounded by a configurable depth and file count limit (`mcp.resource_max_depth`, default 5; `mcp.resource_max_files`, default 1000) to prevent excessive directory traversal in large workspaces.

34. Resource listing respects `.gitignore` patterns when a `.gitignore` file exists in the workspace root. Files matching gitignore patterns are excluded from the resource list.

35. Binary files (detected by extension or content sniffing) are listed in resource discovery but return a base64-encoded `blob` content type on read, not `text`.

36. The `resources/subscribe` and `resources/unsubscribe` methods are supported. The server watches for filesystem changes (via `notify` crate) and sends `notifications/resources/updated` when a subscribed resource changes. File watching is bounded to the workspace root.

### Prompt Exposure

37. MCP prompts expose system instructions and context templates. The `prompts/list` method returns available prompts.

38. A default prompt `"system"` is always available. It returns the system instruction text configured in the model section or a default grokrs system prompt describing the workspace, trust level, and available tools.

39. A `"workspace-context"` prompt returns a summary of the workspace: name, root, available tools, policy posture (what is allowed/denied), trust level. This helps MCP clients understand the environment they are operating in.

40. Custom prompts can be defined in `configs/prompts/` as Markdown files. Each file becomes an MCP prompt with the filename (minus extension) as the prompt name. The file content is the prompt text. Arguments are supported via `{{arg_name}}` template syntax.

41. The `prompts/get` method returns the prompt content with arguments substituted. Unknown arguments are left as-is (not stripped), and a warning is included in the prompt metadata.

### Session Management

42. Each MCP client connection gets a dedicated `Session<T>` where `T` is the configured trust level. The session ID is a UUID v4 generated at connection time.

43. Session lifecycle for stdio: `Created` on process start -> `Ready` after `initialize` handshake -> `RunningTurn` during tool calls -> `WaitingApproval` during broker interaction -> `Closed` on stdin EOF.

44. Session lifecycle for HTTP+SSE: `Created` on SSE connection -> `Ready` after `initialize` -> `RunningTurn` during tool calls -> `WaitingApproval` during broker interaction -> `Closed` on SSE disconnect.

45. Sessions are persisted in `grokrs-store` (if configured). The `sessions` table records the session with `trust_level` matching the MCP config and state transitions logged as they occur.

46. Multiple concurrent HTTP+SSE sessions are supported. Each has independent state, batch approvals, and effect history. The server maintains a `HashMap<SessionId, SessionContext>` in memory.

47. Session metadata (start time, trust level, transport type, client info from `initialize`) is available via `grokrs store sessions` CLI subcommand (existing infrastructure from spec 02).

### Configuration

48. `AppConfig` in `grokrs-core` gains an optional `[mcp]` section:
    ```toml
    [mcp]
    transport = "stdio"                    # "stdio" or "sse"
    trust_level = "Untrusted"              # trust level for MCP client sessions
    host = "127.0.0.1"                     # HTTP+SSE bind address
    port = 3000                            # HTTP+SSE port
    exposed_tools = []                     # tool names to expose; empty = all
    tool_timeout_secs = 60                 # per-tool-call timeout
    resource_max_depth = 5                 # max directory depth for resource listing
    resource_max_files = 1000              # max files in resource listing
    bearer_token_env = "GROKRS_MCP_TOKEN"  # env var for HTTP+SSE auth token
    ```

49. The `[mcp]` section is `Option<McpConfig>`. Existing configs without it continue to load. The `grokrs mcp` subcommands require the section to be present and fail with a clear error if it is missing.

50. `configs/grokrs.example.toml` is updated with a commented-out `[mcp]` section showing all fields with defaults.

51. `bearer_token_env` names the environment variable holding the shared bearer token for HTTP+SSE authentication. The token itself is never in the config file. When the transport is stdio, this field is ignored. When the transport is `sse` and the env var is unset, the server refuses to start with an error explaining that HTTP+SSE requires a bearer token.

### CLI Surface

52. New subcommand group `grokrs mcp`:
    - `grokrs mcp stdio` -- start MCP server on stdio transport.
    - `grokrs mcp serve [--host <host>] [--port <port>]` -- start MCP server on HTTP+SSE transport.
    - `grokrs mcp tools` -- list tools that would be exposed via MCP (dry-run, no server started).
    - `grokrs mcp resources [--depth <n>]` -- list resources that would be exposed (dry-run).
    - `grokrs mcp prompts` -- list prompts that would be exposed (dry-run).

53. `grokrs doctor` is updated to report MCP configuration when the `[mcp]` section is present: transport, trust level, number of exposed tools, resource limits.

### Error Handling

54. MCP protocol errors (malformed JSON-RPC, unknown method, invalid params) return standard JSON-RPC error codes (`-32700` parse error, `-32601` method not found, `-32602` invalid params, `-32603` internal error).

55. Policy denial errors use custom code `-32001` with structured error data: `{ "effect": "<effect description>", "reason": "<denial reason>" }`.

56. Tool execution errors use custom code `-32002` with the tool name and error message in structured data.

57. Approval timeout errors use custom code `-32003` with the effect description and timeout duration.

58. All error responses include a human-readable `message` field suitable for display to the end user.

### Logging and Observability

59. MCP request/response pairs are logged to `grokrs-store` transcripts (if configured) with `endpoint` set to `mcp://<method>` (e.g., `mcp://tools/call`, `mcp://resources/read`).

60. The server logs connection events (connect, disconnect, initialize) to stderr at `info` level.

61. Tool calls are logged at `info` level with the tool name and effect classification. Policy denials are logged at `warn` level.

62. No MCP message content is logged at `info` level or below. Full request/response bodies are logged only at `debug`/`trace` level or to the store transcript.

## Safety Requirements

1. Every MCP tool call goes through `PolicyEngine::evaluate` before execution. There is no bypass, fast path, or "trusted client" exemption. The MCP server enforces the same security model as the CLI.

2. Resource reads are bounded by `WorkspacePath` validation. No path outside the workspace root is accessible through MCP resources, regardless of what URI the client requests.

3. The MCP server does not expose write access to workspace files through the resources interface. File mutations require tool calls with `FsWrite` effects that go through policy evaluation.

4. HTTP+SSE transport requires a bearer token. The server refuses to start without one. The token is read from an environment variable, never from config files. Token comparison uses constant-time equality to prevent timing attacks.

5. stdio transport inherits the security context of the parent process. No additional authentication is performed (the parent process is trusted to have launched grokrs intentionally).

6. The server does not execute arbitrary code from MCP clients. Tool calls are dispatched only to registered `ToolSpec` implementations. Unknown tool names return an error, not a fallback execution path.

7. Resource listing and file watching are bounded to the workspace root. The `notify` file watcher is configured with the workspace root as the watch path and does not follow symlinks that escape the workspace.

8. Session state is isolated per connection. One MCP client's session cannot access another client's session state, batch approvals, or effect history.

9. The MCP server does not expose the API key, bearer token, or any secret through tools, resources, or prompts. The `workspace-context` prompt includes the trust level and policy posture but never authentication credentials.

10. Tool timeout enforcement is mandatory. A tool that exceeds `tool_timeout_secs` is forcibly cancelled. The server does not wait indefinitely for a tool to complete, which prevents a malicious or buggy tool from hanging the server.

11. The server validates all incoming JSON-RPC messages against the MCP schema before dispatching. Malformed messages are rejected at the protocol layer, not passed to tool handlers.

12. File watching via `notify` is throttled (debounce interval configurable, default 500ms) to prevent resource exhaustion from rapid filesystem changes.

## Deliverables

- `crates/grokrs-mcp/` crate with:
  - `src/lib.rs` -- crate root, MCP server builder, re-exports
  - `src/protocol/` -- JSON-RPC message types, MCP request/response/notification types, capability negotiation
  - `src/transport/stdio.rs` -- stdin/stdout message loop
  - `src/transport/sse.rs` -- axum-based HTTP+SSE server, bearer token auth middleware
  - `src/tools.rs` -- `ToolSpec` to MCP tool adapter, effect gating dispatch, tool call timeout
  - `src/resources.rs` -- workspace file listing, path validation, file reading, subscription and file watching
  - `src/prompts.rs` -- system prompt, workspace context prompt, custom prompt loading from `configs/prompts/`
  - `src/session.rs` -- per-connection session management, session context map
  - `src/error.rs` -- MCP error codes, structured error data
- Updated `crates/grokrs-core/src/lib.rs` with `mcp: Option<McpConfig>` and `McpConfig` struct
- Updated `crates/grokrs-tool/src/lib.rs` with `description()` and `input_schema()` methods on `ToolSpec`
- Updated `crates/grokrs-cli/` with `mcp` subcommand group (`stdio`, `serve`, `tools`, `resources`, `prompts`)
- Updated `configs/grokrs.example.toml` with commented-out `[mcp]` section
- Updated `Cargo.toml` workspace members
- Tests: protocol handshake (initialize/initialized round-trip), tool listing (registered tools appear in tools/list), tool call with policy allow (effect classified, policy returns Allow, tool executes), tool call with policy deny (error response with -32001), tool call with approval broker (Ask -> broker -> Approved -> execute), resource listing (workspace files listed, no escape), resource read (valid path returns content, invalid path returns error), prompt listing and retrieval, session lifecycle (Created -> Ready -> RunningTurn -> Closed), HTTP+SSE bearer token validation (valid token accepted, missing/invalid token rejected), stdio message framing (newline-delimited JSON), tool timeout enforcement, concurrent sessions (independent state), config loading with and without `[mcp]` section
