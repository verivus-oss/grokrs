//! Concrete [`ChatBackend`] implementation that wires the REPL to
//! [`GrokClient`] via the Responses API with SSE streaming.
//!
//! `GrokChatBackend` holds an `Arc<GrokClient>` and mutable conversation
//! state (model, system instructions, `previous_response_id`). Each
//! `send_message` call constructs a `CreateResponseRequest`, streams
//! the response token-by-token (printing to stdout), accumulates the
//! full text, extracts usage from the `ResponseCompleted` event, and
//! returns a [`ChatResponse`].
//!
//! Two conversation modes are supported:
//!
//! - **Stateless** (default, `store=false`): The caller is responsible for
//!   managing local conversation history. Each request is independent.
//! - **Stateful** (`store=true`): The backend chains requests via
//!   `previous_response_id`, letting the server maintain conversation
//!   context. Requires the `--stateful` flag at the CLI layer.

use std::io::Write as IoWrite;
use std::sync::Arc;

use futures::StreamExt;

use grokrs_api::client::GrokClient;
use grokrs_api::streaming::parser::parse_response_stream;
use grokrs_api::types::common::Role;
use grokrs_api::types::message::InputMessage;
use grokrs_api::types::responses::{CreateResponseBuilder, InputItem, ResponseInput};
use grokrs_api::types::stream::ResponseStreamEvent;

use crate::commands::search::{self, Citation, SearchConfig};

use super::backend::{BackendError, ChatBackend, ChatResponse, TokenUsage};

/// An encrypted reasoning blob captured from a response's `OutputItem::Reasoning`.
///
/// Stored between turns so it can be replayed as `InputItem::Reasoning` in the
/// next request, giving the model reasoning continuity.
#[derive(Debug, Clone)]
struct EncryptedReasoningBlob {
    /// The reasoning block ID from the original response (if any).
    id: Option<String>,
    /// The opaque encrypted content string.
    encrypted_content: String,
}

/// Configuration for [`GrokChatBackend`].
#[derive(Debug, Clone)]
pub struct GrokBackendConfig {
    /// Initial model name (e.g. `"grok-4"`).
    pub model: String,
    /// Whether to use server-side conversation chaining (`store=true`).
    pub stateful: bool,
    /// Search configuration (which built-in tools to include, search parameters).
    pub search: SearchConfig,
    /// Optional prompt cache key for server-side prompt caching.
    ///
    /// When set, the key is sent as `prompt_cache_key` in every Responses API
    /// request. The server may return cache-hit information in
    /// `usage.prompt_tokens_details.cached_tokens`, which is surfaced in the
    /// per-turn usage display.
    pub cache_key: Option<String>,
}

/// A [`ChatBackend`] backed by the xAI Grok Responses API.
///
/// Streams response tokens to stdout as they arrive, then returns the
/// accumulated [`ChatResponse`] with text, usage, and optional
/// `previous_response_id`.
pub struct GrokChatBackend {
    /// Shared API client.
    client: Arc<GrokClient>,
    /// Current model name.
    model: String,
    /// Optional system instructions applied to every request.
    system_instructions: Option<String>,
    /// Previous response ID for stateful (server-side) conversation chaining.
    /// Only populated when `stateful` is `true`.
    previous_response_id: Option<String>,
    /// Encrypted reasoning blobs captured from the most recent response.
    ///
    /// In stateful mode these are replayed as `InputItem::Reasoning` items
    /// in the next turn's input so the model can continue its reasoning
    /// chain without exposing raw chain-of-thought.
    encrypted_reasoning: Vec<EncryptedReasoningBlob>,
    /// Whether to use server-side conversation chaining.
    stateful: bool,
    /// Search configuration (built-in tools and search parameters).
    search: SearchConfig,
    /// Optional prompt cache key sent as `prompt_cache_key` in every request.
    ///
    /// When `Some`, the server-side prompt cache is engaged. Cache hit/miss
    /// information from `usage.prompt_tokens_details.cached_tokens` is surfaced
    /// in the per-turn [`TokenUsage`] returned from `send_message`.
    cache_key: Option<String>,
    /// Writer for streaming output. Defaults to stdout but injectable for
    /// testing.
    output: Box<dyn IoWrite + Send>,
}

impl GrokChatBackend {
    /// Create a new backend from a shared client and configuration.
    pub fn new(client: Arc<GrokClient>, config: GrokBackendConfig) -> Self {
        Self {
            client,
            model: config.model,
            system_instructions: None,
            previous_response_id: None,
            encrypted_reasoning: Vec::new(),
            stateful: config.stateful,
            search: config.search,
            cache_key: config.cache_key,
            output: Box::new(std::io::stdout()),
        }
    }

    /// Create a backend with a custom writer (for testing).
    #[cfg(test)]
    fn with_output(
        client: Arc<GrokClient>,
        config: GrokBackendConfig,
        output: Box<dyn IoWrite + Send>,
    ) -> Self {
        Self {
            client,
            model: config.model,
            system_instructions: None,
            previous_response_id: None,
            encrypted_reasoning: Vec::new(),
            stateful: config.stateful,
            search: config.search,
            cache_key: config.cache_key,
            output,
        }
    }

    /// Set the previous response ID for stateful conversation chaining.
    ///
    /// Used by `--resume` to restore the chain from a persisted session.
    /// Has no effect in stateless mode (the field is only read when
    /// `self.stateful` is true).
    pub fn set_previous_response_id(&mut self, id: &str) {
        self.previous_response_id = Some(id.to_owned());
    }

    /// Build the `CreateResponseRequest` for a single turn.
    fn build_request(&self, message: &str) -> grokrs_api::types::responses::CreateResponseRequest {
        let input = if self.stateful && self.previous_response_id.is_some() {
            // In stateful mode with a previous response, build input items
            // that include any encrypted reasoning blobs from the last turn
            // followed by the new user message.
            let mut items: Vec<InputItem> = self
                .encrypted_reasoning
                .iter()
                .map(|blob| InputItem::Reasoning {
                    r#type: "reasoning".to_owned(),
                    id: blob.id.clone(),
                    encrypted_content: blob.encrypted_content.clone(),
                })
                .collect();
            items.push(InputItem::Message(InputMessage::text(Role::User, message)));
            ResponseInput::Items(items)
        } else {
            // Stateless (or first turn in stateful): send as plain text.
            ResponseInput::Text(message.to_owned())
        };

        let mut builder = CreateResponseBuilder::new(&self.model, input).stream(true);

        // In stateful mode, request encrypted reasoning content so the
        // model returns encrypted blobs we can replay in subsequent turns.
        if self.stateful {
            builder = builder.include(vec!["reasoning.encrypted_content".to_owned()]);
        }

        // System instructions.
        if let Some(ref instructions) = self.system_instructions {
            builder = builder.instructions(instructions.clone());
        }

        // Search tools: add BuiltinTool values to the tools array.
        if !self.search.is_empty() {
            builder = builder.tools(self.search.tool_values());
        }

        // Search parameters: date range, max results, citations.
        if let Some(params) = self.search.search_parameters() {
            builder = builder.search_parameters(params.to_value());
        }

        // Stateful: chain via previous_response_id and enable server storage.
        if self.stateful {
            builder = builder.store(true);
            if let Some(ref prev_id) = self.previous_response_id {
                builder = builder.previous_response_id(prev_id.clone());
            }
        } else {
            builder = builder.store(false);
        }

        // Prompt cache key: enables server-side prompt caching.
        if let Some(ref key) = self.cache_key {
            builder = builder.prompt_cache_key(key.clone());
        }

        builder.build()
    }

    /// Extract usage from a `ResponseCompleted` event's response JSON.
    ///
    /// Extracts `input_tokens`, `output_tokens`, and the optional
    /// `cached_tokens` from `usage.prompt_tokens_details.cached_tokens`
    /// (returned by the server when a `prompt_cache_key` was set and the
    /// server served some tokens from its cache).
    fn extract_usage(response: &serde_json::Value) -> TokenUsage {
        let Some(usage) = response.get("usage") else {
            return TokenUsage::default();
        };

        let input_tokens = usage["input_tokens"].as_u64().unwrap_or(0);
        let output_tokens = usage["output_tokens"].as_u64().unwrap_or(0);

        // cached_tokens lives inside prompt_tokens_details (Responses API)
        // or input_tokens_details; check both locations.
        let cached_tokens = usage
            .get("prompt_tokens_details")
            .and_then(|d| d.get("cached_tokens"))
            .and_then(|v| v.as_u64())
            .or_else(|| {
                usage
                    .get("input_tokens_details")
                    .and_then(|d| d.get("cached_tokens"))
                    .and_then(|v| v.as_u64())
            });

        TokenUsage {
            input_tokens,
            output_tokens,
            cached_tokens,
        }
    }

    /// Extract the response ID from a `ResponseCompleted` event's response JSON.
    fn extract_response_id(response: &serde_json::Value) -> Option<String> {
        response
            .get("id")
            .and_then(|id| id.as_str())
            .map(|s| s.to_owned())
    }

    /// Extract encrypted reasoning blobs from a `ResponseCompleted` event's
    /// response JSON.
    ///
    /// Scans `output` for items with `"type": "reasoning"` that carry an
    /// `encrypted_content` field. Returns an empty vec when the model did
    /// not produce any encrypted reasoning (e.g. non-reasoning models or
    /// when `include` was not set).
    fn extract_encrypted_reasoning(response: &serde_json::Value) -> Vec<EncryptedReasoningBlob> {
        let Some(output) = response.get("output").and_then(|o| o.as_array()) else {
            return Vec::new();
        };

        output
            .iter()
            .filter_map(|item| {
                if item.get("type")?.as_str()? != "reasoning" {
                    return None;
                }
                let encrypted_content = item.get("encrypted_content")?.as_str()?;
                Some(EncryptedReasoningBlob {
                    id: item.get("id").and_then(|v| v.as_str()).map(String::from),
                    encrypted_content: encrypted_content.to_owned(),
                })
            })
            .collect()
    }
}

impl ChatBackend for GrokChatBackend {
    async fn send_message(&mut self, message: &str) -> Result<ChatResponse, BackendError> {
        let request = self.build_request(message);

        let raw_stream = self
            .client
            .responses()
            .create_stream(&request)
            .await
            .map_err(|e| BackendError::Transport(e.to_string()))?;

        let mut stream = parse_response_stream(raw_stream);

        let mut accumulated_text = String::new();
        let mut usage = TokenUsage::default();
        let mut response_id: Option<String> = None;
        let mut citations: Vec<Citation> = Vec::new();

        while let Some(event) = stream.next().await {
            match event {
                Ok(ResponseStreamEvent::ContentDelta { delta, .. }) => {
                    if let Some(ref text) = delta.text {
                        let _ = write!(self.output, "{text}");
                        let _ = self.output.flush();
                        accumulated_text.push_str(text);
                    }
                }
                Ok(ResponseStreamEvent::OutputTextDelta { delta, .. }) => {
                    let _ = write!(self.output, "{delta}");
                    let _ = self.output.flush();
                    accumulated_text.push_str(&delta);
                }
                Ok(ResponseStreamEvent::ResponseCompleted { response }) => {
                    usage = Self::extract_usage(&response);
                    response_id = Self::extract_response_id(&response);
                    citations = search::extract_citations(&response);

                    // Capture encrypted reasoning for replay in next turn.
                    if self.stateful {
                        self.encrypted_reasoning = Self::extract_encrypted_reasoning(&response);
                    }
                }
                Ok(ResponseStreamEvent::ResponseFailed { response }) => {
                    let error_msg = response
                        .get("error")
                        .and_then(|e| e.get("message"))
                        .and_then(|m| m.as_str())
                        .unwrap_or("unknown API error");
                    let status = response
                        .get("error")
                        .and_then(|e| e.get("code"))
                        .and_then(|c| c.as_u64())
                        .unwrap_or(500) as u16;
                    return Err(BackendError::Api {
                        status,
                        message: error_msg.to_owned(),
                    });
                }
                Ok(_) => {
                    // Other events (created, in_progress, output_item, etc.)
                    // are ignored — they carry metadata, not user-visible content.
                }
                Err(e) => {
                    return Err(BackendError::Transport(e.to_string()));
                }
            }
        }

        // Trailing newline after streamed text (matches existing run_chat UX).
        if !accumulated_text.is_empty() {
            let _ = writeln!(self.output);
        }

        // Display citations after the response text if any were returned.
        if !citations.is_empty() {
            let formatted = search::format_citations(&citations);
            let _ = write!(self.output, "{formatted}");
            let _ = self.output.flush();
        }

        // Update stateful chaining state.
        if self.stateful {
            self.previous_response_id = response_id.clone();
        }

        Ok(ChatResponse {
            text: accumulated_text,
            usage,
            previous_response_id: response_id,
            citations,
        })
    }

    fn model(&self) -> &str {
        &self.model
    }

    fn set_model(&mut self, model: &str) {
        self.model = model.to_owned();
    }

    fn set_system(&mut self, instructions: &str) {
        self.system_instructions = Some(instructions.to_owned());
    }

    fn clear(&mut self) {
        self.previous_response_id = None;
        self.system_instructions = None;
        self.encrypted_reasoning.clear();
    }
}

impl std::fmt::Debug for GrokChatBackend {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("GrokChatBackend")
            .field("model", &self.model)
            .field("stateful", &self.stateful)
            .field("search", &self.search)
            .field(
                "previous_response_id",
                &self.previous_response_id.as_deref().unwrap_or("<none>"),
            )
            .field(
                "system_instructions",
                &self
                    .system_instructions
                    .as_deref()
                    .map(|s| if s.len() > 50 { &s[..50] } else { s }),
            )
            .finish()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // -----------------------------------------------------------------------
    // Helper: build a GrokBackendConfig
    // -----------------------------------------------------------------------

    fn stateless_config() -> GrokBackendConfig {
        GrokBackendConfig {
            model: "grok-4".into(),
            stateful: false,
            search: SearchConfig::default(),
            cache_key: None,
        }
    }

    fn stateful_config() -> GrokBackendConfig {
        GrokBackendConfig {
            model: "grok-4".into(),
            stateful: true,
            search: SearchConfig::default(),
            cache_key: None,
        }
    }

    fn config_with_cache_key(key: &str) -> GrokBackendConfig {
        GrokBackendConfig {
            model: "grok-4".into(),
            stateful: false,
            search: SearchConfig::default(),
            cache_key: Some(key.to_owned()),
        }
    }

    // -----------------------------------------------------------------------
    // Helper: create a GrokClient from env (for unit tests that don't call API)
    // -----------------------------------------------------------------------

    fn make_client() -> Arc<GrokClient> {
        use grokrs_api::transport::policy_gate::AllowAllGate;
        use grokrs_core::{
            ApiConfig, AppConfig, ModelConfig, PolicyConfig, SessionConfig, WorkspaceConfig,
        };

        let config = AppConfig {
            workspace: WorkspaceConfig {
                name: "test".into(),
                root: ".".into(),
            },
            model: ModelConfig {
                provider: "xai".into(),
                default_model: "grok-4".into(),
            },
            policy: PolicyConfig {
                allow_network: false,
                allow_shell: false,
                allow_workspace_writes: true,
                max_patch_bytes: 1024,
            },
            session: SessionConfig {
                approval_mode: "interactive".into(),
                transcript_dir: ".grokrs/sessions".into(),
            },
            api: Some(ApiConfig {
                api_key_env: Some("GROKRS_TEST_BACKEND_KEY".into()),
                base_url: Some("https://api.x.ai".into()),
                timeout_secs: Some(60),
                max_retries: Some(2),
            }),
            management_api: None,
            store: None,
            agent: None,
            chat: None,
            mcp: None,
        };

        std::env::set_var("GROKRS_TEST_BACKEND_KEY", "test-key-for-backend");
        let client = GrokClient::from_config(&config, Some(Arc::new(AllowAllGate)))
            .expect("test client should build");
        std::env::remove_var("GROKRS_TEST_BACKEND_KEY");
        Arc::new(client)
    }

    // -----------------------------------------------------------------------
    // Construction and initial state
    // -----------------------------------------------------------------------

    #[test]
    fn new_backend_has_correct_initial_state() {
        let client = make_client();
        let backend = GrokChatBackend::new(client, stateless_config());
        assert_eq!(backend.model(), "grok-4");
        assert!(backend.system_instructions.is_none());
        assert!(backend.previous_response_id.is_none());
        assert!(!backend.stateful);
    }

    #[test]
    fn new_stateful_backend_has_correct_initial_state() {
        let client = make_client();
        let backend = GrokChatBackend::new(client, stateful_config());
        assert_eq!(backend.model(), "grok-4");
        assert!(backend.stateful);
        assert!(backend.previous_response_id.is_none());
    }

    // -----------------------------------------------------------------------
    // set_model / set_system / clear
    // -----------------------------------------------------------------------

    #[test]
    fn set_model_changes_model() {
        let client = make_client();
        let mut backend = GrokChatBackend::new(client, stateless_config());
        backend.set_model("grok-4-mini");
        assert_eq!(backend.model(), "grok-4-mini");
    }

    #[test]
    fn set_system_stores_instructions() {
        let client = make_client();
        let mut backend = GrokChatBackend::new(client, stateless_config());
        backend.set_system("You are a Rust expert");
        assert_eq!(
            backend.system_instructions.as_deref(),
            Some("You are a Rust expert")
        );
    }

    #[test]
    fn clear_resets_state() {
        let client = make_client();
        let mut backend = GrokChatBackend::new(client, stateful_config());
        backend.set_system("some instructions");
        backend.previous_response_id = Some("resp_abc".into());

        backend.clear();

        assert!(backend.system_instructions.is_none());
        assert!(backend.previous_response_id.is_none());
    }

    // -----------------------------------------------------------------------
    // build_request: stateless mode
    // -----------------------------------------------------------------------

    #[test]
    fn build_request_stateless_uses_text_input() {
        let client = make_client();
        let backend = GrokChatBackend::new(client, stateless_config());
        let req = backend.build_request("hello");

        assert_eq!(req.model, "grok-4");
        assert_eq!(req.store, Some(false));
        assert!(req.stream == Some(true));
        assert!(req.previous_response_id.is_none());
        assert!(req.instructions.is_none());

        // Verify input is Text variant.
        let json = serde_json::to_value(&req.input).unwrap();
        assert!(json.is_string(), "stateless input should be a plain string");
        assert_eq!(json.as_str().unwrap(), "hello");
    }

    #[test]
    fn build_request_stateless_with_system() {
        let client = make_client();
        let mut backend = GrokChatBackend::new(client, stateless_config());
        backend.set_system("Be concise");
        let req = backend.build_request("hello");

        assert_eq!(req.instructions.as_deref(), Some("Be concise"));
    }

    #[test]
    fn build_request_stateless_ignores_previous_response_id() {
        let client = make_client();
        let mut backend = GrokChatBackend::new(client, stateless_config());
        // Manually set a previous_response_id (shouldn't happen in stateless,
        // but verify it's not sent).
        backend.previous_response_id = Some("resp_old".into());
        let req = backend.build_request("hello");

        // Stateless: store=false, no previous_response_id.
        assert_eq!(req.store, Some(false));
        assert!(req.previous_response_id.is_none());
    }

    // -----------------------------------------------------------------------
    // build_request: stateful mode
    // -----------------------------------------------------------------------

    #[test]
    fn build_request_stateful_first_turn() {
        let client = make_client();
        let backend = GrokChatBackend::new(client, stateful_config());
        let req = backend.build_request("hello");

        assert_eq!(req.store, Some(true));
        assert!(req.previous_response_id.is_none());
        // First turn: plain text input (no previous_response_id to chain from).
        let json = serde_json::to_value(&req.input).unwrap();
        assert!(json.is_string());
    }

    #[test]
    fn build_request_stateful_subsequent_turn() {
        let client = make_client();
        let mut backend = GrokChatBackend::new(client, stateful_config());
        backend.previous_response_id = Some("resp_abc123".into());
        let req = backend.build_request("follow up");

        assert_eq!(req.store, Some(true));
        assert_eq!(req.previous_response_id.as_deref(), Some("resp_abc123"));
        // Subsequent turn: messages array with just the user message.
        let json = serde_json::to_value(&req.input).unwrap();
        assert!(
            json.is_array(),
            "stateful subsequent turn should use messages array"
        );
    }

    #[test]
    fn build_request_reflects_model_change() {
        let client = make_client();
        let mut backend = GrokChatBackend::new(client, stateless_config());
        backend.set_model("grok-4-mini");
        let req = backend.build_request("test");
        assert_eq!(req.model, "grok-4-mini");
    }

    // -----------------------------------------------------------------------
    // cache_key propagation
    // -----------------------------------------------------------------------

    #[test]
    fn build_request_without_cache_key_has_none() {
        let client = make_client();
        let backend = GrokChatBackend::new(client, stateless_config());
        let req = backend.build_request("hello");
        assert!(
            req.prompt_cache_key.is_none(),
            "no cache key should be set when config has None"
        );
    }

    #[test]
    fn build_request_with_cache_key_sets_prompt_cache_key() {
        let client = make_client();
        let backend = GrokChatBackend::new(client, config_with_cache_key("my-system-prompt"));
        let req = backend.build_request("hello");
        assert_eq!(
            req.prompt_cache_key.as_deref(),
            Some("my-system-prompt"),
            "prompt_cache_key should be propagated from GrokBackendConfig"
        );
    }

    #[test]
    fn build_request_cache_key_serializes_to_json() {
        let client = make_client();
        let backend = GrokChatBackend::new(client, config_with_cache_key("cache-test-key-42"));
        let req = backend.build_request("test message");
        let json = serde_json::to_string(&req).unwrap();
        assert!(
            json.contains("\"prompt_cache_key\":\"cache-test-key-42\""),
            "prompt_cache_key should appear in serialized request JSON; got: {json}"
        );
    }

    #[test]
    fn new_backend_stores_cache_key_from_config() {
        let client = make_client();
        let backend = GrokChatBackend::new(client, config_with_cache_key("persistent-cache-key"));
        assert_eq!(
            backend.cache_key.as_deref(),
            Some("persistent-cache-key"),
            "cache_key should be stored on the backend"
        );
    }

    // -----------------------------------------------------------------------
    // extract_usage (with cached_tokens)
    // -----------------------------------------------------------------------

    #[test]
    fn extract_usage_from_response_json() {
        let response = serde_json::json!({
            "id": "resp_1",
            "status": "completed",
            "usage": {
                "input_tokens": 42,
                "output_tokens": 17,
                "total_tokens": 59
            }
        });
        let usage = GrokChatBackend::extract_usage(&response);
        assert_eq!(usage.input_tokens, 42);
        assert_eq!(usage.output_tokens, 17);
        assert!(usage.cached_tokens.is_none());
    }

    #[test]
    fn extract_usage_missing_usage_field() {
        let response = serde_json::json!({"id": "resp_1", "status": "completed"});
        let usage = GrokChatBackend::extract_usage(&response);
        assert_eq!(usage.input_tokens, 0);
        assert_eq!(usage.output_tokens, 0);
        assert!(usage.cached_tokens.is_none());
    }

    #[test]
    fn extract_usage_with_prompt_tokens_details_cached_tokens() {
        let response = serde_json::json!({
            "id": "resp_2",
            "status": "completed",
            "usage": {
                "input_tokens": 500,
                "output_tokens": 100,
                "total_tokens": 600,
                "prompt_tokens_details": {
                    "cached_tokens": 200,
                    "text_tokens": 300
                }
            }
        });
        let usage = GrokChatBackend::extract_usage(&response);
        assert_eq!(usage.input_tokens, 500);
        assert_eq!(usage.output_tokens, 100);
        assert_eq!(usage.cached_tokens, Some(200));
    }

    #[test]
    fn extract_usage_with_input_tokens_details_cached_tokens() {
        // Some API variants use input_tokens_details instead.
        let response = serde_json::json!({
            "id": "resp_3",
            "status": "completed",
            "usage": {
                "input_tokens": 300,
                "output_tokens": 50,
                "input_tokens_details": {
                    "cached_tokens": 150
                }
            }
        });
        let usage = GrokChatBackend::extract_usage(&response);
        assert_eq!(usage.input_tokens, 300);
        assert_eq!(usage.output_tokens, 50);
        assert_eq!(usage.cached_tokens, Some(150));
    }

    #[test]
    fn extract_usage_prompt_tokens_details_takes_precedence_over_input_tokens_details() {
        // When both are present, prompt_tokens_details wins.
        let response = serde_json::json!({
            "id": "resp_4",
            "status": "completed",
            "usage": {
                "input_tokens": 400,
                "output_tokens": 80,
                "prompt_tokens_details": { "cached_tokens": 250 },
                "input_tokens_details": { "cached_tokens": 999 }
            }
        });
        let usage = GrokChatBackend::extract_usage(&response);
        assert_eq!(usage.cached_tokens, Some(250));
    }

    // -----------------------------------------------------------------------
    // extract_response_id
    // -----------------------------------------------------------------------

    #[test]
    fn extract_response_id_present() {
        let response = serde_json::json!({"id": "resp_abc123", "status": "completed"});
        assert_eq!(
            GrokChatBackend::extract_response_id(&response),
            Some("resp_abc123".into())
        );
    }

    #[test]
    fn extract_response_id_missing() {
        let response = serde_json::json!({"status": "completed"});
        assert_eq!(GrokChatBackend::extract_response_id(&response), None);
    }

    // -----------------------------------------------------------------------
    // Debug output
    // -----------------------------------------------------------------------

    #[test]
    fn debug_output_shows_key_fields() {
        let client = make_client();
        let mut backend = GrokChatBackend::new(client, stateful_config());
        backend.set_system("test instructions");
        backend.previous_response_id = Some("resp_debug".into());

        let debug = format!("{backend:?}");
        assert!(debug.contains("grok-4"));
        assert!(debug.contains("stateful: true"));
        assert!(debug.contains("resp_debug"));
        assert!(debug.contains("test instructions"));
    }

    // -----------------------------------------------------------------------
    // Integration test: streaming with wiremock
    // -----------------------------------------------------------------------

    #[tokio::test]
    async fn send_message_streams_and_returns_response() {
        use grokrs_api::transport::policy_gate::AllowAllGate;
        use wiremock::matchers::{method, path};
        use wiremock::{Mock, MockServer, ResponseTemplate};

        let server = MockServer::start().await;

        // Build SSE response body with content delta, output text delta,
        // and completed events.
        let sse_body = [
            "data: {\"type\":\"response.created\",\"response\":{\"id\":\"resp_test1\",\"status\":\"in_progress\"}}\n\n",
            "data: {\"type\":\"response.content_part.delta\",\"output_index\":0,\"content_index\":0,\"delta\":{\"type\":\"text\",\"text\":\"Hello\"}}\n\n",
            "data: {\"type\":\"response.output_text.delta\",\"output_index\":0,\"delta\":\" world\"}\n\n",
            "data: {\"type\":\"response.completed\",\"response\":{\"id\":\"resp_test1\",\"status\":\"completed\",\"usage\":{\"input_tokens\":10,\"output_tokens\":5,\"total_tokens\":15}}}\n\n",
            "data: [DONE]\n\n",
        ]
        .join("");

        Mock::given(method("POST"))
            .and(path("/v1/responses"))
            .respond_with(
                ResponseTemplate::new(200)
                    .insert_header("content-type", "text/event-stream")
                    .set_body_string(sse_body),
            )
            .expect(1)
            .mount(&server)
            .await;

        // Build a GrokClient via from_config with the mock server URL.
        use grokrs_core::{
            ApiConfig, AppConfig, ModelConfig, PolicyConfig, SessionConfig, WorkspaceConfig,
        };
        let env_var = "GROKRS_TEST_STREAM_BACKEND_KEY";
        std::env::set_var(env_var, "stream-test-key");
        let app_config = AppConfig {
            workspace: WorkspaceConfig {
                name: "test".into(),
                root: ".".into(),
            },
            model: ModelConfig {
                provider: "xai".into(),
                default_model: "grok-4".into(),
            },
            policy: PolicyConfig {
                allow_network: false,
                allow_shell: false,
                allow_workspace_writes: true,
                max_patch_bytes: 1024,
            },
            session: SessionConfig {
                approval_mode: "allow".into(),
                transcript_dir: ".grokrs/sessions".into(),
            },
            api: Some(ApiConfig {
                api_key_env: Some(env_var.into()),
                base_url: Some(server.uri()),
                timeout_secs: Some(30),
                max_retries: Some(0),
            }),
            management_api: None,
            store: None,
            agent: None,
            chat: None,
            mcp: None,
        };
        let grok_client = GrokClient::from_config(&app_config, Some(Arc::new(AllowAllGate)))
            .expect("client should build");
        std::env::remove_var(env_var);

        let output_buf: Vec<u8> = Vec::new();
        let mut backend = GrokChatBackend::with_output(
            Arc::new(grok_client),
            stateless_config(),
            Box::new(output_buf),
        );

        let response = backend.send_message("Hi there").await.unwrap();

        assert_eq!(response.text, "Hello world");
        assert_eq!(response.usage.input_tokens, 10);
        assert_eq!(response.usage.output_tokens, 5);
        assert_eq!(response.previous_response_id.as_deref(), Some("resp_test1"));
    }

    #[tokio::test]
    async fn send_message_stateful_updates_previous_response_id() {
        use grokrs_api::transport::policy_gate::AllowAllGate;
        use wiremock::matchers::{method, path};
        use wiremock::{Mock, MockServer, ResponseTemplate};

        let server = MockServer::start().await;

        let sse_body = [
            "data: {\"type\":\"response.created\",\"response\":{\"id\":\"resp_chain1\",\"status\":\"in_progress\"}}\n\n",
            "data: {\"type\":\"response.output_text.delta\",\"output_index\":0,\"delta\":\"OK\"}\n\n",
            "data: {\"type\":\"response.completed\",\"response\":{\"id\":\"resp_chain1\",\"status\":\"completed\",\"usage\":{\"input_tokens\":5,\"output_tokens\":1,\"total_tokens\":6}}}\n\n",
            "data: [DONE]\n\n",
        ]
        .join("");

        Mock::given(method("POST"))
            .and(path("/v1/responses"))
            .respond_with(
                ResponseTemplate::new(200)
                    .insert_header("content-type", "text/event-stream")
                    .set_body_string(sse_body),
            )
            .expect(1)
            .mount(&server)
            .await;

        let env_var = "GROKRS_TEST_STATEFUL_KEY";
        std::env::set_var(env_var, "stateful-test-key");

        use grokrs_core::{
            ApiConfig, AppConfig, ModelConfig, PolicyConfig, SessionConfig, WorkspaceConfig,
        };
        let app_config = AppConfig {
            workspace: WorkspaceConfig {
                name: "test".into(),
                root: ".".into(),
            },
            model: ModelConfig {
                provider: "xai".into(),
                default_model: "grok-4".into(),
            },
            policy: PolicyConfig {
                allow_network: false,
                allow_shell: false,
                allow_workspace_writes: true,
                max_patch_bytes: 1024,
            },
            session: SessionConfig {
                approval_mode: "allow".into(),
                transcript_dir: ".grokrs/sessions".into(),
            },
            api: Some(ApiConfig {
                api_key_env: Some(env_var.into()),
                base_url: Some(server.uri()),
                timeout_secs: Some(30),
                max_retries: Some(0),
            }),
            management_api: None,
            store: None,
            agent: None,
            chat: None,
            mcp: None,
        };
        let grok_client = GrokClient::from_config(&app_config, Some(Arc::new(AllowAllGate)))
            .expect("client should build");
        std::env::remove_var(env_var);

        let output_buf: Vec<u8> = Vec::new();
        let mut backend = GrokChatBackend::with_output(
            Arc::new(grok_client),
            stateful_config(),
            Box::new(output_buf),
        );

        assert!(backend.previous_response_id.is_none());

        let response = backend.send_message("start").await.unwrap();
        assert_eq!(response.text, "OK");

        // Stateful mode: previous_response_id should be updated.
        assert_eq!(backend.previous_response_id.as_deref(), Some("resp_chain1"));
    }

    #[tokio::test]
    async fn send_message_transport_error_returns_backend_error() {
        use grokrs_api::transport::policy_gate::AllowAllGate;
        use wiremock::matchers::{method, path};
        use wiremock::{Mock, MockServer, ResponseTemplate};

        let server = MockServer::start().await;

        // Return a connection refused by not mounting any mock.
        // Actually, return a 500 to trigger a transport-level error.
        Mock::given(method("POST"))
            .and(path("/v1/responses"))
            .respond_with(ResponseTemplate::new(500).set_body_string("Internal Server Error"))
            .expect(1)
            .mount(&server)
            .await;

        let env_var = "GROKRS_TEST_ERR_KEY";
        std::env::set_var(env_var, "error-test-key");

        use grokrs_core::{
            ApiConfig, AppConfig, ModelConfig, PolicyConfig, SessionConfig, WorkspaceConfig,
        };
        let app_config = AppConfig {
            workspace: WorkspaceConfig {
                name: "test".into(),
                root: ".".into(),
            },
            model: ModelConfig {
                provider: "xai".into(),
                default_model: "grok-4".into(),
            },
            policy: PolicyConfig {
                allow_network: false,
                allow_shell: false,
                allow_workspace_writes: true,
                max_patch_bytes: 1024,
            },
            session: SessionConfig {
                approval_mode: "allow".into(),
                transcript_dir: ".grokrs/sessions".into(),
            },
            api: Some(ApiConfig {
                api_key_env: Some(env_var.into()),
                base_url: Some(server.uri()),
                timeout_secs: Some(30),
                max_retries: Some(0),
            }),
            management_api: None,
            store: None,
            agent: None,
            chat: None,
            mcp: None,
        };
        let grok_client = GrokClient::from_config(&app_config, Some(Arc::new(AllowAllGate)))
            .expect("client should build");
        std::env::remove_var(env_var);

        let output_buf: Vec<u8> = Vec::new();
        let mut backend = GrokChatBackend::with_output(
            Arc::new(grok_client),
            stateless_config(),
            Box::new(output_buf),
        );

        let result = backend.send_message("hello").await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn send_message_response_failed_returns_api_error() {
        use grokrs_api::transport::policy_gate::AllowAllGate;
        use wiremock::matchers::{method, path};
        use wiremock::{Mock, MockServer, ResponseTemplate};

        let server = MockServer::start().await;

        let sse_body = [
            "data: {\"type\":\"response.created\",\"response\":{\"id\":\"resp_fail\",\"status\":\"in_progress\"}}\n\n",
            "data: {\"type\":\"response.failed\",\"response\":{\"id\":\"resp_fail\",\"status\":\"failed\",\"error\":{\"code\":429,\"message\":\"rate limit exceeded\"}}}\n\n",
            "data: [DONE]\n\n",
        ]
        .join("");

        Mock::given(method("POST"))
            .and(path("/v1/responses"))
            .respond_with(
                ResponseTemplate::new(200)
                    .insert_header("content-type", "text/event-stream")
                    .set_body_string(sse_body),
            )
            .expect(1)
            .mount(&server)
            .await;

        let env_var = "GROKRS_TEST_FAIL_EVENT_KEY";
        std::env::set_var(env_var, "fail-event-test-key");

        use grokrs_core::{
            ApiConfig, AppConfig, ModelConfig, PolicyConfig, SessionConfig, WorkspaceConfig,
        };
        let app_config = AppConfig {
            workspace: WorkspaceConfig {
                name: "test".into(),
                root: ".".into(),
            },
            model: ModelConfig {
                provider: "xai".into(),
                default_model: "grok-4".into(),
            },
            policy: PolicyConfig {
                allow_network: false,
                allow_shell: false,
                allow_workspace_writes: true,
                max_patch_bytes: 1024,
            },
            session: SessionConfig {
                approval_mode: "allow".into(),
                transcript_dir: ".grokrs/sessions".into(),
            },
            api: Some(ApiConfig {
                api_key_env: Some(env_var.into()),
                base_url: Some(server.uri()),
                timeout_secs: Some(30),
                max_retries: Some(0),
            }),
            management_api: None,
            store: None,
            agent: None,
            chat: None,
            mcp: None,
        };
        let grok_client = GrokClient::from_config(&app_config, Some(Arc::new(AllowAllGate)))
            .expect("client should build");
        std::env::remove_var(env_var);

        let output_buf: Vec<u8> = Vec::new();
        let mut backend = GrokChatBackend::with_output(
            Arc::new(grok_client),
            stateless_config(),
            Box::new(output_buf),
        );

        let result = backend.send_message("trigger error").await;
        assert!(result.is_err());
        match result.unwrap_err() {
            BackendError::Api { status, message } => {
                assert_eq!(status, 429);
                assert!(message.contains("rate limit"));
            }
            other => panic!("expected Api error, got: {other}"),
        }
    }

    // -----------------------------------------------------------------------
    // Verify request structure via wiremock body inspection
    // -----------------------------------------------------------------------

    #[tokio::test]
    async fn request_includes_system_instructions() {
        use grokrs_api::transport::policy_gate::AllowAllGate;
        use wiremock::matchers::{method, path};
        use wiremock::{Mock, MockServer, ResponseTemplate};

        let server = MockServer::start().await;

        let sse_body = [
            "data: {\"type\":\"response.completed\",\"response\":{\"id\":\"resp_sys\",\"status\":\"completed\",\"usage\":{\"input_tokens\":1,\"output_tokens\":1,\"total_tokens\":2}}}\n\n",
            "data: [DONE]\n\n",
        ]
        .join("");

        Mock::given(method("POST"))
            .and(path("/v1/responses"))
            .respond_with(
                ResponseTemplate::new(200)
                    .insert_header("content-type", "text/event-stream")
                    .set_body_string(sse_body),
            )
            .expect(1)
            .mount(&server)
            .await;

        let env_var = "GROKRS_TEST_SYS_KEY";
        std::env::set_var(env_var, "sys-test-key");

        use grokrs_core::{
            ApiConfig, AppConfig, ModelConfig, PolicyConfig, SessionConfig, WorkspaceConfig,
        };
        let app_config = AppConfig {
            workspace: WorkspaceConfig {
                name: "test".into(),
                root: ".".into(),
            },
            model: ModelConfig {
                provider: "xai".into(),
                default_model: "grok-4".into(),
            },
            policy: PolicyConfig {
                allow_network: false,
                allow_shell: false,
                allow_workspace_writes: true,
                max_patch_bytes: 1024,
            },
            session: SessionConfig {
                approval_mode: "allow".into(),
                transcript_dir: ".grokrs/sessions".into(),
            },
            api: Some(ApiConfig {
                api_key_env: Some(env_var.into()),
                base_url: Some(server.uri()),
                timeout_secs: Some(30),
                max_retries: Some(0),
            }),
            management_api: None,
            store: None,
            agent: None,
            chat: None,
            mcp: None,
        };
        let grok_client = GrokClient::from_config(&app_config, Some(Arc::new(AllowAllGate)))
            .expect("client should build");
        std::env::remove_var(env_var);

        let output_buf: Vec<u8> = Vec::new();
        let mut backend = GrokChatBackend::with_output(
            Arc::new(grok_client),
            stateless_config(),
            Box::new(output_buf),
        );
        backend.set_system("You are a pirate");

        let _ = backend.send_message("ahoy").await;

        // Inspect the request body sent to the mock server.
        let received = server.received_requests().await.unwrap();
        assert_eq!(received.len(), 1);
        let body: serde_json::Value = serde_json::from_slice(&received[0].body).unwrap();
        assert_eq!(body["instructions"], "You are a pirate");
        assert_eq!(body["model"], "grok-4");
        assert_eq!(body["store"], false);
        assert_eq!(body["stream"], true);
    }

    // -----------------------------------------------------------------------
    // build_request: search integration
    // -----------------------------------------------------------------------

    fn search_config_web() -> GrokBackendConfig {
        GrokBackendConfig {
            model: "grok-4".into(),
            stateful: false,
            search: SearchConfig {
                web_search: true,
                ..Default::default()
            },
            cache_key: None,
        }
    }

    fn search_config_both_with_params() -> GrokBackendConfig {
        GrokBackendConfig {
            model: "grok-4".into(),
            stateful: false,
            search: SearchConfig {
                web_search: true,
                x_search: true,
                from_date: Some("2025-01-01".into()),
                to_date: Some("2025-06-30".into()),
                max_results: Some(5),
                citations: true,
            },
            cache_key: None,
        }
    }

    #[test]
    fn build_request_no_search_has_no_tools() {
        let client = make_client();
        let backend = GrokChatBackend::new(client, stateless_config());
        let req = backend.build_request("hello");
        assert!(req.tools.is_none());
        assert!(req.search_parameters.is_none());
    }

    #[test]
    fn build_request_web_search_adds_tool() {
        let client = make_client();
        let backend = GrokChatBackend::new(client, search_config_web());
        let req = backend.build_request("search for rust");

        let tools = req.tools.expect("should have tools");
        assert_eq!(tools.len(), 1);
        assert_eq!(tools[0]["type"], "web_search");
        // No search parameters when only search tool is enabled (no date/max/citations).
        assert!(req.search_parameters.is_none());
    }

    #[test]
    fn build_request_both_search_with_params() {
        let client = make_client();
        let backend = GrokChatBackend::new(client, search_config_both_with_params());
        let req = backend.build_request("search everything");

        let tools = req.tools.expect("should have tools");
        assert_eq!(tools.len(), 2);
        assert_eq!(tools[0]["type"], "web_search");
        assert_eq!(tools[1]["type"], "x_search");

        let params = req
            .search_parameters
            .expect("should have search_parameters");
        assert_eq!(params["from_date"], "2025-01-01");
        assert_eq!(params["to_date"], "2025-06-30");
        assert_eq!(params["max_search_results"], 5);
        assert_eq!(params["return_citations"], true);
        assert_eq!(params["mode"], "auto");
    }

    #[test]
    fn build_request_search_with_stateful() {
        let client = make_client();
        let config = GrokBackendConfig {
            model: "grok-4".into(),
            stateful: true,
            search: SearchConfig {
                web_search: true,
                ..Default::default()
            },
            cache_key: None,
        };
        let backend = GrokChatBackend::new(client, config);
        let req = backend.build_request("hello");

        // Should have both tools and store=true.
        assert!(req.tools.is_some());
        assert_eq!(req.store, Some(true));
    }

    // -----------------------------------------------------------------------
    // Integration: streaming with search citations
    // -----------------------------------------------------------------------

    #[tokio::test]
    async fn send_message_with_search_extracts_citations() {
        use grokrs_api::transport::policy_gate::AllowAllGate;
        use wiremock::matchers::{method, path};
        use wiremock::{Mock, MockServer, ResponseTemplate};

        let server = MockServer::start().await;

        // SSE response that includes citations in the completed response.
        let sse_body = [
            "data: {\"type\":\"response.created\",\"response\":{\"id\":\"resp_search1\",\"status\":\"in_progress\"}}\n\n",
            "data: {\"type\":\"response.output_text.delta\",\"output_index\":0,\"delta\":\"Rust is great.\"}\n\n",
            "data: {\"type\":\"response.completed\",\"response\":{\"id\":\"resp_search1\",\"status\":\"completed\",\"usage\":{\"input_tokens\":20,\"output_tokens\":10,\"total_tokens\":30},\"citations\":[{\"url\":\"https://rust-lang.org\",\"title\":\"Rust Language\"},{\"url\":\"https://doc.rust-lang.org\"}]}}\n\n",
            "data: [DONE]\n\n",
        ]
        .join("");

        Mock::given(method("POST"))
            .and(path("/v1/responses"))
            .respond_with(
                ResponseTemplate::new(200)
                    .insert_header("content-type", "text/event-stream")
                    .set_body_string(sse_body),
            )
            .expect(1)
            .mount(&server)
            .await;

        let env_var = "GROKRS_TEST_SEARCH_CITE_KEY";
        std::env::set_var(env_var, "search-cite-test-key");

        use grokrs_core::{
            ApiConfig, AppConfig, ModelConfig, PolicyConfig, SessionConfig, WorkspaceConfig,
        };
        let app_config = AppConfig {
            workspace: WorkspaceConfig {
                name: "test".into(),
                root: ".".into(),
            },
            model: ModelConfig {
                provider: "xai".into(),
                default_model: "grok-4".into(),
            },
            policy: PolicyConfig {
                allow_network: false,
                allow_shell: false,
                allow_workspace_writes: true,
                max_patch_bytes: 1024,
            },
            session: SessionConfig {
                approval_mode: "allow".into(),
                transcript_dir: ".grokrs/sessions".into(),
            },
            api: Some(ApiConfig {
                api_key_env: Some(env_var.into()),
                base_url: Some(server.uri()),
                timeout_secs: Some(30),
                max_retries: Some(0),
            }),
            management_api: None,
            store: None,
            agent: None,
            chat: None,
            mcp: None,
        };
        let grok_client = GrokClient::from_config(&app_config, Some(Arc::new(AllowAllGate)))
            .expect("client should build");
        std::env::remove_var(env_var);

        let output_buf: Vec<u8> = Vec::new();
        let mut backend = GrokChatBackend::with_output(
            Arc::new(grok_client),
            search_config_web(),
            Box::new(output_buf),
        );

        let response = backend.send_message("tell me about rust").await.unwrap();

        assert_eq!(response.text, "Rust is great.");
        assert_eq!(response.citations.len(), 2);
        assert_eq!(response.citations[0].url, "https://rust-lang.org");
        assert_eq!(
            response.citations[0].title.as_deref(),
            Some("Rust Language")
        );
        assert_eq!(response.citations[1].url, "https://doc.rust-lang.org");
        assert!(response.citations[1].title.is_none());
    }

    #[tokio::test]
    async fn request_with_search_includes_tools_in_body() {
        use grokrs_api::transport::policy_gate::AllowAllGate;
        use wiremock::matchers::{method, path};
        use wiremock::{Mock, MockServer, ResponseTemplate};

        let server = MockServer::start().await;

        let sse_body = [
            "data: {\"type\":\"response.completed\",\"response\":{\"id\":\"resp_st\",\"status\":\"completed\",\"usage\":{\"input_tokens\":1,\"output_tokens\":1,\"total_tokens\":2}}}\n\n",
            "data: [DONE]\n\n",
        ]
        .join("");

        Mock::given(method("POST"))
            .and(path("/v1/responses"))
            .respond_with(
                ResponseTemplate::new(200)
                    .insert_header("content-type", "text/event-stream")
                    .set_body_string(sse_body),
            )
            .expect(1)
            .mount(&server)
            .await;

        let env_var = "GROKRS_TEST_SEARCH_BODY_KEY";
        std::env::set_var(env_var, "search-body-test-key");

        use grokrs_core::{
            ApiConfig, AppConfig, ModelConfig, PolicyConfig, SessionConfig, WorkspaceConfig,
        };
        let app_config = AppConfig {
            workspace: WorkspaceConfig {
                name: "test".into(),
                root: ".".into(),
            },
            model: ModelConfig {
                provider: "xai".into(),
                default_model: "grok-4".into(),
            },
            policy: PolicyConfig {
                allow_network: false,
                allow_shell: false,
                allow_workspace_writes: true,
                max_patch_bytes: 1024,
            },
            session: SessionConfig {
                approval_mode: "allow".into(),
                transcript_dir: ".grokrs/sessions".into(),
            },
            api: Some(ApiConfig {
                api_key_env: Some(env_var.into()),
                base_url: Some(server.uri()),
                timeout_secs: Some(30),
                max_retries: Some(0),
            }),
            management_api: None,
            store: None,
            agent: None,
            chat: None,
            mcp: None,
        };
        let grok_client = GrokClient::from_config(&app_config, Some(Arc::new(AllowAllGate)))
            .expect("client should build");
        std::env::remove_var(env_var);

        let output_buf: Vec<u8> = Vec::new();
        let mut backend = GrokChatBackend::with_output(
            Arc::new(grok_client),
            search_config_both_with_params(),
            Box::new(output_buf),
        );

        let _ = backend.send_message("search test").await;

        // Inspect the request body.
        let received = server.received_requests().await.unwrap();
        assert_eq!(received.len(), 1);
        let body: serde_json::Value = serde_json::from_slice(&received[0].body).unwrap();

        // Verify tools array contains both search tools.
        let tools = body["tools"].as_array().expect("tools should be an array");
        assert_eq!(tools.len(), 2);
        assert_eq!(tools[0]["type"], "web_search");
        assert_eq!(tools[1]["type"], "x_search");

        // Verify search_parameters.
        let params = &body["search_parameters"];
        assert_eq!(params["from_date"], "2025-01-01");
        assert_eq!(params["to_date"], "2025-06-30");
        assert_eq!(params["max_search_results"], 5);
        assert_eq!(params["return_citations"], true);
    }
}
