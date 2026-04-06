# xAI API Client Implementation Plan

DAG: `docs/reviews/xai-api-client/2026-04-05/IMPLEMENTATION_DAG.toml`
Spec: `docs/specs/01_XAI_API_CLIENT.md`
Units: 14 across 4 layers, estimated 5 340 LOC

## Architectural Decisions

- New `grokrs-api` crate; dependency direction: `grokrs-cli` -> `grokrs-api` -> `grokrs-core`
- `grokrs-api` does NOT depend on `grokrs-cap` or `grokrs-policy` at compile time; policy gate is injected at runtime via the `PolicyGate` trait
- `PolicyGate` trait: `fn evaluate(&self, effect: &Effect) -> Decision`. CLI wires `PolicyEngine`; tests wire a stub returning `Allow`. `Ask` returns `ApprovalRequired` error (no approval broker yet)
- `AppConfig` gains `api: Option<ApiConfig>` — backward-compatible, existing configs without `[api]` still load. Config stores only the env var name for the API key, never the secret itself
- Responses API `store` defaults to `false` (overriding xAI's default of `true`) to match fail-closed posture. Callers must opt-in to server-side storage
- File upload accepts `&Path` from caller who validated via `WorkspacePath`; `grokrs-api` does not depend on `grokrs-cap`
- Hand-written serde types over code-gen or external OpenAPI crates
- `reqwest` for HTTP, `futures::Stream` for SSE streaming
- Responses API is the primary integration target; Chat Completions is legacy compat, marked `#[deprecated]`
- Monetary values (`cost_in_usd_ticks`, pricing) are `i64`, never `f64`
- Enum variants use `#[serde(rename)]` to match xAI wire names exactly
- Model names and pricing are never hardcoded; discovered dynamically via Models API at runtime
- Voice Agent API and Collections Management API are out of scope (deferred to separate specs)

## Phase 1 — Foundation (Layer 0: U10, U11)

- **U10 — API wire types: common** (~550 LOC)
  - Create `crates/grokrs-api` crate and add it to the Cargo workspace
  - Hand-write serde types for shared xAI primitives: message roles, content blocks (text, image_url, input_image), tool definitions, tool calls, tool results
  - Usage objects with `prompt_tokens_details`, `completion_tokens_details`, `cost_in_usd_ticks` as `i64`
  - Model metadata, error envelope
  - Types are pure data — zero HTTP client dependency
  - All optional fields use `skip_serializing_if = "Option::is_none"`
  - Message content modeled as enum supporting both `String` and `Vec<ContentBlock>` forms

- **U11 — HTTP transport layer** (~380 LOC)
  - Async HTTP client wrapper around `reqwest`: base URL `https://api.x.ai`, bearer auth from `XAI_API_KEY` env or config
  - Retry with exponential backoff + jitter on 429/503, configurable max retries
  - Structured error mapping into `grokrs-core` error types with status code, message, request ID
  - SSE line-protocol parser returning async `Stream` of raw event strings
  - Expose `send_json` / `send_sse` / `send_multipart` primitives
  - No endpoint-specific logic in this layer

U10 and U11 have no dependencies and can be developed in parallel. They block all subsequent units.

## Phase 2 — Endpoint Clients (Layer 1: U12, U13, U14, U15)

- **U12 — Responses API client** (~620 LOC)
  - POST `/v1/responses` (create), GET `/v1/responses/{id}` (retrieve), DELETE `/v1/responses/{id}` (delete)
  - Request types: model, input (string | message array), instructions, tools, tool_choice, previous_response_id, store, stream, temperature, top_p, max_output_tokens, max_turns, reasoning effort, text format / json_schema, search_parameters
  - Response types: id, status, output (typed message/reasoning/tool_call items), usage with `server_side_tool_usage_details`
  - Stateful conversation chaining via `previous_response_id`
  - Input modeled as `InputString(String) | InputMessages(Vec<Message>)`

- **U13 — Chat Completions API client (legacy)** (~480 LOC)
  - POST `/v1/chat/completions`, GET `/v1/chat/deferred-completion/{request_id}` for deferred polling
  - Request types: model, messages, tools, tool_choice, stream, temperature, top_p, max_completion_tokens, n, stop, seed, frequency/presence penalty, response_format, reasoning_effort, deferred
  - Response types: id, choices (message with content, reasoning_content, refusal, tool_calls), usage, system_fingerprint, citations
  - Module and public types marked `#[deprecated]` pointing to Responses API

- **U14 — Streaming support** (~420 LOC)
  - SSE-based streaming for both Responses and Chat Completions
  - Parse `data:` lines, handle `data: [DONE]` terminator
  - Deserialize deltas into typed stream events, expose as `async Stream<Item = Result<Event>>`
  - Chat Completions: stream delta chunks with partial content, tool_call fragments, optional usage in final chunk (`stream_options.include_usage`)
  - Responses: stream response events with output items
  - Function calls arrive whole, not token-by-token
  - Stream is cancellable — dropping does not leak the HTTP connection
  - Memory-bounded: no unbounded event buffering

- **U15 — Models API client** (~320 LOC)
  - All 8 model listing/detail endpoints: `/v1/models`, `/v1/language-models`, `/v1/image-generation-models`, `/v1/video-generation-models` (list + detail for each)
  - Separate `Model` (minimal: id, created, owned_by) from `LanguageModel` (full: modalities, pricing, aliases, fingerprint, version)
  - Pricing fields as integer `i64` (USD cents per 100M tokens)
  - Forward-compatible: no `deny_unknown_fields`

All four Layer 1 units depend on U10 + U11 and can be developed in parallel.

## Phase 3 — Tools, Media, and Auxiliary (Layer 2: U16, U17, U20-U23)

- **U16 — Function calling and tool use** (~480 LOC)
  - Function definitions with name, description, JSON Schema parameters (max 128 tools, validated at construction)
  - `tool_choice` variants: auto, required, none, named function
  - Built-in server-side tools as unit-struct enum variants: `web_search`, `x_search`, `code_execution`, `code_interpreter`, `collections_search`, `file_search`, `attachment_search`
  - `search_parameters`: mode, sources, from_date, to_date, max_search_results, return_citations
  - Structured outputs via `json_schema` response format
  - Tool loop helper: drives call-execute-return cycle; helper is optional, callers can drive manually
  - Function execution is the caller's responsibility; crate handles wire format only
  - Depends on U12, U13, U14

- **U17 — Media generation clients** (~440 LOC)
  - Images: POST `/v1/images/generations`, POST `/v1/images/edits` (single image + multi-reference)
  - Videos: POST `/v1/videos/generations`, POST `/v1/videos/edits`, POST `/v1/videos/extensions`, GET `/v1/videos/{request_id}` (poll)
  - Videos are async submit + poll: typed async helper yields progress updates (0-100)
  - Duration bounds enforced at construction: 1-15s generation, 1-10s extension
  - Aspect ratio and resolution are enums, not free-form strings
  - Image `response_format` defaults to URL; b64_json opt-in
  - Depends on U10, U11, U15

- **U20 — Batch API client** (~360 LOC)
  - POST `/v1/batches` (create), POST `/v1/batches/{id}/requests` (add), GET `/v1/batches/{id}` (status), GET `/v1/batches/{id}/results` (paginated), POST `/v1/batches/{id}:cancel`, GET `/v1/batches` (list)
  - Batch requests reuse the same typed request builders as realtime endpoints
  - Results paginated via `pagination_token` — client handles pagination
  - Depends on U12, U13, U16

- **U21 — Files API client** (~300 LOC)
  - POST `/v1/files` (single upload via multipart/form-data)
  - POST `/v1/files:initialize` + POST `/v1/files:uploadChunks` (chunked upload for large files)
  - GET `/v1/files` (list), GET `/v1/files/{file_id}` (metadata), PUT `/v1/files/{file_id}` (update), POST `/v1/files:download`
  - File upload passes through policy for `FsRead` before sending
  - Depends on U10, U11

- **U22 — Tokenizer and utility clients** (~160 LOC)
  - POST `/v1/tokenize-text`: returns `token_ids` with `token_id` (`u32`), `string_token`, `token_bytes` (`Vec<u8>`)
  - GET `/v1/api-key`: key metadata including name, status, acls, team_id, blocked/disabled flags
  - Token bytes handled as `Vec<u8>`, never unwrapped as `String`
  - Depends on U10, U11

- **U23 — Policy bridge: NetworkConnect gating** (~280 LOC)
  - Define `PolicyGate` trait: `fn evaluate(&self, effect: &Effect) -> Decision`
  - Gate at the transport layer: every outbound HTTP request evaluates `NetworkConnect { host }` through injected `PolicyGate`
  - `Deny` returns typed `PolicyDenied` error; `Ask` returns typed `ApprovalRequired` error (no approval broker yet — error message states this)
  - `grokrs-api` does NOT depend on `grokrs-policy` at compile time — trait is injected by caller
  - `AppConfig` gains `api: Option<ApiConfig>` — backward-compatible with existing configs
  - `ApiConfig` stores `api_key_env` (env var name only, never the secret), `base_url`, `timeout_secs`, `max_retries`
  - `summary()`, `show-config`, and embedded test config updated
  - API key never appears in logs, error messages, or Debug output
  - Tests wire a stub returning `Allow`, never bypass the gate
  - Depends on U10, U11

Layer 2 units can be developed in parallel after their respective Layer 1 dependencies complete. U21, U22, U23 only need Layer 0 and can start as soon as Phase 1 completes.

## Phase 4 — Facade and CLI (Layer 3: U30, U31)

- **U30 — Unified GrokClient facade** (~280 LOC)
  - `GrokClient::from_config(AppConfig)` composes all endpoint clients over a shared transport + policy gate
  - Builder-style sub-client accessors: `client.responses()`, `client.chat()`, `client.models()`, `client.images()`, `client.videos()`, `client.files()`, `client.batches()`, `client.tokenize()`, `client.api_key()`
  - Session-aware: optional association with `Session<T>` for trust-level-aware operations
  - Facade is pure delegation — no hidden retries, caching, or added behavior
  - Depends on all Layer 1 + Layer 2 units

- **U31 — CLI integration: api subcommands** (~350 LOC)
  - `grokrs api models` — list available models with ID and pricing
  - `grokrs api chat '<prompt>'` — one-shot prompt with streaming token-by-token output to stdout
  - `grokrs api tokenize '<text>'` — print token count and IDs
  - `grokrs api key-info` — show redacted key, team, and ACL info
  - All subcommands gated by policy engine (`allow_network=true` required)
  - Policy denial errors explain how to enable network access in config
  - Config path configurable via `--config` flag
  - CLI never stores or logs the API key
  - Depends on U30

## Critical Path

`U10 -> U12 -> U16 -> U30 -> U31` (2 280 LOC)

This is the shortest path through the DAG that delivers a working end-to-end API integration. All other units add breadth (media, batch, files, utilities) but do not gate the primary chat + tool-use flow.

## LOC Totals

| Tier | Units | LOC |
|------|-------|-----|
| Tier 1 (core) | U10-U17 | 3 690 |
| Tier 2 (auxiliary) | U20-U23 | 1 100 |
| Tier 3 (facade + CLI) | U30-U31 | 630 |
| **Total** | **14** | **5 420** |
