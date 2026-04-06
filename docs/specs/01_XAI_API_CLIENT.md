# xAI API Client Spec

Date: 2026-04-05

## Summary

`grokrs-api` is a new crate that provides a typed, hand-written Rust client for the xAI Grok API (`https://api.x.ai/v1`). It covers both the recommended Responses API and the legacy Chat Completions API, plus model discovery, media generation, file management, batch processing, and utility endpoints. The crate is pure API types and transport -- it depends on `grokrs-core` for configuration and accepts an injected `PolicyGate` trait for `NetworkConnect` gating, but does not depend on `grokrs-cap`, `grokrs-policy`, `grokrs-session`, or `grokrs-tool` at compile time.

There is no official xAI Rust SDK. All serde types are hand-written against the xAI wire format with `reqwest` as the HTTP backend.

## Goals

- Provide complete, typed coverage of the xAI Grok API surface as of 2026-04-05.
- Gate every outbound HTTP request through an injected `PolicyGate` trait for `NetworkConnect` effect evaluation.
- Expose streaming as `async Stream<Item = Result<Event>>`, not callbacks.
- Keep API wire types in a dedicated crate (`grokrs-api`) with no compile-time dependency on safety-model crates beyond `grokrs-core`.
- Support stateful conversation chaining via `previous_response_id` on the Responses API.
- Add `api` subcommands to `grokrs-cli` that prove the client works end-to-end through the safety model.

## Non-Goals

- Wrapping or depending on an external xAI SDK or OpenAPI code generator.
- Implementing the server side of any xAI protocol (no xAI-compatible server).
- Autonomous agent loops -- tool call execution is the caller's responsibility; this crate handles wire format only.
- Caching or local persistence of API responses (transcript persistence is a separate spec).
- MCP transport bridging (separate planned work per `docs/design/00_ARCHITECTURE.md`).
- Voice Agent API (WebSocket real-time voice) -- requires a fundamentally different transport model; deferred to a separate spec.
- Collections Management API (`management-api.x.ai`) -- uses a separate auth model (Management API Key); deferred to a separate spec.
- Hardcoded model names, pricing, or rate limits -- all model information is discovered dynamically via `/v1/models` and `/v1/language-models` endpoints at runtime.

## Functional Requirements

### Wire Types {#common-types}

1. Hand-write serde types for all shared xAI API primitives in `crates/grokrs-api/src/types/`:
   - Message roles: `system`, `developer`, `user`, `assistant`, `tool`.
   - Content blocks: `text`, `image_url`, `input_image` (as an enum with serde rename matching xAI wire names).
   - Tool definitions: function name, description, `parameters` as `serde_json::Value` (JSON Schema).
   - Tool calls: `id`, `type: "function"`, `function: { name, arguments }`.
   - Tool results: `tool_call_id`, `content`.

2. Usage object {#usage-object}: `input_tokens`, `output_tokens`, `reasoning_tokens` (all `u64`), `prompt_tokens_details` and `completion_tokens_details` sub-objects, `cost_in_usd_ticks` as `i64` (10 billion ticks = $1 USD), `server_side_tool_usage_details` with per-tool token counts.

3. Error envelope {#error-envelope}: top-level `{ error: { message, type, code } }` with HTTP status code and `x-request-id` header captured as `request_id: Option<String>`.

4. Model metadata: `id`, `created` (Unix timestamp `i64`), `owned_by`, `aliases: Vec<String>`, `input_modalities`, `output_modalities`, pricing fields (`prompt_text_token_price`, `completion_text_token_price`, `cached_prompt_text_token_price` -- all `i64`, integer cents per 100 million tokens), `fingerprint`, `version`.

5. All optional fields serialize with `#[serde(skip_serializing_if = "Option::is_none")]`.

6. Message content supports both `String` and `Vec<ContentBlock>` forms via an untagged enum.

7. Types must not depend on any HTTP client -- they are pure data structs in `types/`.

### Transport {#http-client}

8. Thin async HTTP client in `crates/grokrs-api/src/transport/` wrapping `reqwest::Client`.

9. Authentication {#auth}: read `XAI_API_KEY` from environment variable (falling back to `AppConfig.api.api_key_env` for the env var name). Every request includes `Authorization: Bearer <key>`. The key must never appear in logs, error messages, or debug output.

10. Base URL defaults to `https://api.x.ai` and is overridable via `AppConfig.api.base_url` (for testing against mocks).

11. Retry {#retry}: on HTTP 429 and 503, apply exponential backoff with jitter, up to a configurable max retries (default 3). Backoff formula: `min(base * 2^attempt + jitter, max_delay)`.

12. Error handling {#error-handling}: non-2xx responses deserialize into `ApiError` with status code, message, error type, and `request_id` from the `x-request-id` response header.

13. Three transport primitives: `send_json<Req, Resp>`, `send_sse<Req>` (returns async stream of raw SSE event strings), `send_multipart`. No endpoint-specific logic in the transport layer.

14. Request timeout configurable via `AppConfig.api.timeout_secs` (default 120).

### Policy Integration {#network-gating}

15. The transport layer accepts an injected policy evaluator trait (`PolicyGate`) rather than depending on `grokrs-policy` at compile time {#policy-bridge}. The trait has a single method: `fn evaluate(&self, effect: &Effect) -> Decision`. The CLI wires in the real `PolicyEngine`; tests wire in a stub that returns `Allow`. This avoids `grokrs-api` depending on `grokrs-policy` at compile time -- the `Effect` and `Decision` types used by the trait are re-exported from `grokrs-core` or defined locally in `grokrs-api`.

16. Every outbound HTTP request must first evaluate a `NetworkConnect { host }` effect through the injected `PolicyGate`. The effect includes the target hostname extracted from the request URL.

17. If the gate returns `Deny`, the request fails with a typed `PolicyDenied` error (not a generic HTTP error).

18. If the gate returns `Ask`, the request fails with a typed `ApprovalRequired` error. The current codebase has no approval broker; `Ask` is treated as a blocking denial until the approval broker crate exists (per `ARCHITECTURE.md` planned expansion). The error message must state that interactive approval is not yet implemented.

19. Gate at the transport layer (`crates/grokrs-api/src/transport/policy_gate.rs`), not at each endpoint client.

20. Tests use an explicit allow-all stub for the `PolicyGate` trait; the gate is never bypassed or compiled out, even in tests.

### API Configuration {#api-config}

21. `AppConfig` in `grokrs-core` gains an optional `[api]` section via `api: Option<ApiConfig>`:
    ```toml
    [api]
    api_key_env = "XAI_API_KEY"     # env var name holding the bearer token (NOT the key itself)
    base_url = "https://api.x.ai"   # override for testing
    timeout_secs = 120
    max_retries = 3
    ```
    The `[api]` section is `Option` so existing configs without it continue to load. `ApiConfig` is a new struct in `grokrs-core` alongside the existing config types. The section stores ONLY `api_key_env` (env var name), `base_url`, `timeout_secs`, and `max_retries` -- no actual secrets.

22. `configs/grokrs.example.toml` is updated with the `[api]` section. The section contains only the env var name (`api_key_env`), never the actual secret. A comment explains that the bearer token is read from the environment at runtime.

23. `AppConfig::summary()` is updated to include API config fields when present (gated on `api.is_some()`). `show-config` reflects the new section.

24. The embedded sample config in `grokrs-core` tests is updated to include the `[api]` section. Existing tests without `[api]` must continue to pass because the field is `Option`.

25. No actual API key or secret is ever stored in config files, committed TOML, or AppConfig fields. The config stores only the environment variable name from which to read the key at request time.

### Responses API {#responses-create}

26. `POST /v1/responses` -- create a response {#responses-create-endpoint}. Request fields: `model` (required), `input` (string or message array), `instructions`, `tools` (up to 128), `tool_choice`, `previous_response_id`, `store`, `stream`, `temperature`, `top_p`, `max_output_tokens`, `max_turns`, `reasoning` (`{ effort: "low" | "medium" | "high" }`), `text` (`{ format: { type: "json_schema", ... } }`), `search_parameters`.

27. Server-side storage {#server-side-storage}: xAI defaults `store=true` with 30-day retention. **grokrs overrides this: the client defaults `store` to `false`** to match the repo's fail-closed posture. Callers must explicitly opt in to server-side storage by setting `store: true`. When `store=true` is used, the response `id` can be used with `previous_response_id` for stateful continuation and with `GET /v1/responses/{id}` for retrieval. The CLI `api chat` subcommand always sets `store=false`. Stateful conversations and response retrieval only work with explicit `store=true`.

28. `input` is modeled as an enum: `InputString(String)` | `InputMessages(Vec<InputMessage>)` to match the xAI spec.

29. Response object: `id`, `status` (`completed`, `incomplete`, `failed`), `output` (vec of typed items: message, reasoning, function_call_output), `usage` with full detail including `server_side_tool_usage_details`, `model`, `instructions`, `metadata`.

30. Stateful conversations {#stateful-conversations}: chain requests via `previous_response_id`. The client validates that the ID is a non-empty opaque string but does not parse it. Only usable when a previous request was made with `store=true`.

31. `GET /v1/responses/{id}` {#responses-retrieve} -- retrieve a stored response by ID. Only returns results for responses created with `store=true`.

32. `DELETE /v1/responses/{id}` {#responses-delete} -- delete a stored response.

### Chat Completions API (Legacy) {#chat-completions}

33. `POST /v1/chat/completions` -- create a chat completion. Request fields: `model`, `messages`, `tools`, `tool_choice`, `stream`, `temperature`, `top_p`, `max_completion_tokens`, `n`, `stop`, `seed`, `frequency_penalty`, `presence_penalty`, `response_format`, `reasoning_effort`, `search_parameters`, `deferred`.

34. Response: `id`, `choices` (vec of `{ index, message: { role, content, reasoning_content, refusal, tool_calls }, finish_reason }`), `usage`, `system_fingerprint`, `citations`.

35. `finish_reason` enum: `stop`, `length`, `end_turn`, `tool_calls`.

36. Deferred completions {#deferred-completions}: when `deferred: true`, the response includes a `request_id`. Poll via `GET /v1/chat/deferred-completion/{request_id}` -- returns 202 (still processing) or 200 (result ready).

37. All Chat Completions public types and the module carry `#[deprecated]` doc attributes pointing to the Responses API.

### Streaming {#sse-transport}

38. SSE-based streaming {#stream-events} for both Responses and Chat Completions. Parse `data:` lines from the HTTP response body. `data: [DONE]` terminates the stream.

39. Chat Completions stream deltas {#chat-stream-deltas}: partial `content` strings, `tool_call` fragments (function calls arrive as complete chunks, not token-by-token), optional `usage` in the final chunk when `stream_options: { include_usage: true }`.

40. Responses stream events {#responses-stream-events}: typed output item events with complete function call chunks.

41. Exposed as `async Stream<Item = Result<StreamEvent, StreamError>>`. Dropping the stream does not leak the HTTP connection.

42. Memory-bounded: no unbounded internal buffering of events. Partial JSON in SSE data fields surfaces as `StreamError`, not a panic.

### Models API {#models-list}

43. `GET /v1/models` -- list all models. Returns minimal `Model` objects (`id`, `created`, `owned_by`).

44. `GET /v1/models/{model_id}` -- get a single model.

45. `GET /v1/language-models` {#language-models} -- list language models with extended info: modalities, pricing (integer `i64`), aliases, fingerprint, version.

46. `GET /v1/language-models/{model_id}` -- get a single language model.

47. `GET /v1/image-generation-models` {#image-models}, `GET /v1/image-generation-models/{model_id}`.

48. `GET /v1/video-generation-models` {#video-models}, `GET /v1/video-generation-models/{model_id}`.

49. Model aliases {#model-aliases}: `aliases` is `Vec<String>` (may be empty).

50. Separate `Model` (minimal, from `/v1/models`) and `LanguageModel` / `ImageModel` / `VideoModel` (extended) types.

51. Pricing fields are `i64` -- integer cents per 100 million tokens. Never floating point.

52. Model names and pricing are never hardcoded {#dynamic-models}. All model information is discovered at runtime via the Models API endpoints. The client does not maintain a static allowlist of model IDs. No model name, pricing value, or rate limit is compiled into the binary.

### Tools and Function Calling {#tool-definitions}

53. Function definition: `{ type: "function", function: { name, description, parameters } }` where `parameters` is a JSON Schema object. Max 128 tools per request -- validate at construction time.

54. Tool choice {#tool-choice}: `auto`, `required`, `none`, or `{ type: "function", function: { name } }` for a specific function.

55. Parallel tool calls toggle: `parallel_tool_calls: bool`.

56. Function calling loop {#function-calling-loop}: model returns `tool_call` items -> caller executes the function -> caller sends `tool_result` message -> model continues. The `tool_loop` helper in `crates/grokrs-api/src/tool_loop.rs` drives this cycle but is optional -- callers can drive it manually.

57. Function call execution is the caller's responsibility. The API crate handles wire format serialization/deserialization only.

58. Built-in server-side tools {#builtin-tools}: `web_search`, `x_search`, `code_execution` / `code_interpreter`, `collections_search` / `file_search`, `attachment_search`. Modeled as unit-struct enum variants, serialized as `{ type: "<name>" }`.

59. Search parameters {#search-parameters}: `mode` (`auto`, `on`, `off`), `sources` (vec), `from_date`, `to_date`, `max_search_results`, `return_citations`.

60. Structured outputs {#structured-outputs}: `response_format: { type: "json_schema", json_schema: { name, strict, schema } }` for Chat Completions, `text: { format: { type: "json_schema", ... } }` for Responses.

### Image Generation {#image-gen}

61. `POST /v1/images/generations` -- generate images. Request: `prompt`, `model` (e.g. `grok-imagine-image`, `grok-imagine-image-pro`), `n`, `aspect_ratio`, `quality`, `resolution`, `response_format` (`url` or `b64_json`, default `url`).

62. `POST /v1/images/edits` {#image-edit} -- edit images. Request: `prompt`, `image` (single) or `images` (multiple reference images), `mask`, `model`.

63. `aspect_ratio` and `resolution` are enums, not free-form strings.

### Video Generation {#video-gen}

64. `POST /v1/videos/generations` -- generate video. Request: `prompt`, `image` (optional reference), `reference_images`, `duration` (1-15 seconds), `aspect_ratio`, `resolution`. Returns `request_id` (videos are async).

65. `POST /v1/videos/edits` {#video-edit} -- edit video. Request: `prompt`, `video`.

66. `POST /v1/videos/extensions` {#video-extend} -- extend video. Request: `prompt`, `video`, `duration` (1-10 seconds).

67. `GET /v1/videos/{request_id}` {#video-poll} -- poll video status. Returns `status` (`pending` with `progress` 0-100, or `done` with `video.url`).

68. Video duration bounds enforced at construction: 1-15s for generation, 1-10s for extension. Invalid values fail at type construction, not at the wire.

69. A typed async poll helper yields progress updates with backoff, not raw poll loops.

### Files API {#file-upload}

70. `POST /v1/files` -- upload a single file (multipart/form-data).

71. `POST /v1/files:initialize` + `POST /v1/files:uploadChunks` {#file-multipart} -- chunked multipart upload for large files.

72. `GET /v1/files` {#file-list} -- list files (paginated).

73. `GET /v1/files/{file_id}` -- get file metadata.

74. `PUT /v1/files/{file_id}` -- update file metadata.

75. `POST /v1/files:download` {#file-download} -- download file content.

76. `file_id` is an opaque string, not parsed.

77. File upload path safety {#file-upload-path-safety}: file upload functions accept `&Path` as input, not raw `PathBuf`. The caller is responsible for path validation through `WorkspacePath` (from `grokrs-cap`) before passing the path to the upload function. The `grokrs-api` crate does NOT depend on `grokrs-cap` -- it accepts pre-validated paths from the caller. Policy `FsRead` evaluation is also the caller's responsibility before reading the local file.

### Batch API {#batch-create}

78. `POST /v1/batches` -- create a batch.

79. `POST /v1/batches/{id}/requests` {#batch-add-requests} -- add requests to a batch. Accepts `Vec` of Chat Completions or Responses request payloads (reuses the same typed request builders as realtime endpoints).

80. `GET /v1/batches/{id}` {#batch-status} -- batch status: `num_pending`, `num_success`, `num_error`.

81. `GET /v1/batches/{id}/results` {#batch-results} -- paginated results. Client handles `pagination_token` for full result retrieval.

82. `POST /v1/batches/{id}:cancel` {#batch-cancel} -- cancel a batch.

83. `GET /v1/batches` -- list batches.

### Utility Endpoints

84. `POST /v1/tokenize-text` {#tokenize-text}: request `{ text, model }`, response `{ token_ids: [{ token_id: u32, string_token: String, token_bytes: Vec<u8> }] }`. Token bytes may contain invalid UTF-8 -- stored as `Vec<u8>`, not `String`.

85. `GET /v1/api-key` {#api-key-info}: returns key metadata (`name`, `status`, `acls`, `team_id`, `blocked`, `disabled`). The response is informational only and must not be cached.

### Client Facade {#grok-client}

86. `GrokClient` in `crates/grokrs-api/src/client.rs` composes all endpoint clients behind a single constructor: `GrokClient::from_config(config: &AppConfig) -> Result<Self>`.

87. Sub-clients accessed via builder-style methods: `client.responses()`, `client.chat()`, `client.models()`, `client.images()`, `client.videos()`, `client.files()`, `client.batches()`, `client.tokenize(...)`, `client.api_key()`.

88. Session integration {#session-integration}: `GrokClient` can optionally be associated with a `Session<T>` for trust-level-aware operations. Session association is not required -- the client works without one for simple scripts.

89. The facade delegates only -- no hidden retries, caching, or behavior beyond what the underlying endpoint clients provide.

### CLI Surface {#cli-api-commands}

90. Add `api` subcommand group to `grokrs-cli`:
    - `grokrs api models` -- list available models with id and pricing.
    - `grokrs api chat '<prompt>'` -- one-shot prompt with streaming output to stdout (token-by-token). Always sets `store=false`.
    - `grokrs api tokenize '<text>'` -- print token count and IDs.
    - `grokrs api key-info` -- show redacted key name, team, and ACL info.

91. All `api` subcommands respect the policy engine. When `allow_network = false`, they fail with an error message explaining how to enable network access in config.

92. Config path is configurable via `--config` flag.

93. The CLI must not store or log the API key.

## Safety Requirements

1. Every outbound HTTP request evaluates `NetworkConnect` through the injected `PolicyGate` before leaving the process. No bypass path exists.

2. The API key is read from the environment at request time and injected into the `Authorization` header. It never appears in logs, error messages, `Debug` output, or on-disk state.

3. File upload operations require the caller to evaluate `FsRead` through `PolicyEngine` and validate paths through `WorkspacePath` before passing the path to the upload function. The `grokrs-api` crate accepts pre-validated `&Path` and does not perform policy or path validation itself.

4. SSE stream processing is memory-bounded. No unbounded buffering of events. Connection drops surface as typed errors.

5. Retry logic includes jitter to prevent thundering-herd on rate-limit responses.

6. Video and image duration/dimension constraints are enforced at type construction time, not deferred to the API server.

7. `deny_unknown_fields` is NOT set on response types (the xAI API may add fields at any time). Request types may use `deny_unknown_fields` where appropriate.

8. The `grokrs-api` crate does not execute tool calls. It serializes and deserializes the wire format. Execution is the caller's responsibility, subject to the caller's policy evaluation.

9. `grokrs-api` has no compile-time dependency on `grokrs-policy`. Policy integration is achieved through the injected `PolicyGate` trait. The CLI is responsible for wiring the real `PolicyEngine` implementation.

10. `Decision::Ask` from the policy gate is treated as a blocking denial (`ApprovalRequired` error) until the approval broker crate exists. The error message explicitly states that interactive approval is not yet implemented.

11. Server-side storage (`store`) defaults to `false` in all request builders. Callers must explicitly opt in to `store=true`. The CLI `api chat` command always sends `store=false`.

12. Model names, pricing, and rate limits are never hardcoded or compiled into the binary. All model information is discovered dynamically at runtime via the Models API.

## Deliverables

- `crates/grokrs-api/` crate with:
  - `src/types/` -- all wire types (common, message, tool, usage, error, model, responses, chat, stream, function_call, builtin_tools, structured_output, images, videos, files, batches, tokenize, api_key)
  - `src/transport/` -- HTTP client, auth, retry with jitter, SSE parser, error mapping, policy gate trait and module
  - `src/endpoints/` -- typed clients for responses, chat, models, images, videos, files, batches, tokenize, api_key
  - `src/streaming/` -- SSE event parser and typed stream adapter
  - `src/tool_loop.rs` -- optional function-calling loop helper
  - `src/client.rs` -- `GrokClient` facade
  - `src/lib.rs` -- crate root re-exports
- Updated `Cargo.toml` workspace members
- Updated `crates/grokrs-core/` with `api: Option<ApiConfig>` config section
- Updated `configs/grokrs.example.toml` with `[api]` section
- Updated `crates/grokrs-cli/` with `api` subcommand group
- Tests: serde round-trip for all wire types, transport retry/error behavior, policy gate deny/allow/ask paths (including `ApprovalRequired` for `Ask`), streaming parser with `[DONE]` termination and mid-stream errors, config loading with and without `[api]` section
