//! Unified facade over all xAI endpoint clients.
//!
//! `GrokClient` constructs a single `HttpClient` from `AppConfig` and exposes
//! typed accessors for every endpoint module. It is pure delegation — no hidden
//! retries, caching, or added behavior beyond what the underlying clients
//! already provide.

use std::sync::Arc;
use std::time::Duration;

#[allow(deprecated)]
use crate::endpoints::chat::ChatClient;

use crate::endpoints::api_key::ApiKeyClient;
use crate::endpoints::batches::BatchesClient;
use crate::endpoints::documents::DocumentsClient;
use crate::endpoints::files::FilesClient;
use crate::endpoints::images::ImagesClient;
use crate::endpoints::models::ModelsClient;
use crate::endpoints::responses::ResponsesClient;
use crate::endpoints::tokenize::TokenizeClient;
use crate::endpoints::tts::TtsClient;
use crate::endpoints::videos::VideosClient;
use crate::endpoints::voice::VoiceAgentClient;
use crate::transport::auth::resolve_api_key;
use crate::transport::client::{HttpClient, HttpClientConfig};
use crate::transport::error::TransportError;
use crate::transport::policy_gate::PolicyGate;
use crate::transport::retry::RetryConfig;
use crate::transport::websocket::WsClientConfig;
use grokrs_core::AppConfig;

/// Unified client for the xAI Grok API.
///
/// Constructed from `AppConfig` with an injected policy gate. Provides typed
/// accessors for every endpoint family. The facade is pure delegation — no
/// hidden behavior, retries, or caching beyond what `HttpClient` already does.
///
/// # Session Association
///
/// A `GrokClient` can optionally be associated with a session ID for
/// trust-level-aware operations. Session association is not required — the
/// client works without one for simple scripts.
///
/// # Lifetimes
///
/// Some endpoint clients (`ChatClient`, `TokenizeClient`, `FilesClient`,
/// `ApiKeyClient`) borrow from the inner `HttpClient`. Their returned
/// references are tied to `&self`, so the `GrokClient` must outlive them.
pub struct GrokClient {
    http: Arc<HttpClient>,
    /// Optional session ID for trust-level-aware operation tracking.
    session_id: Option<String>,
    /// Base URL for the API (used to derive WebSocket URL).
    base_url: String,
    /// API key for authenticating WebSocket connections.
    api_key: crate::transport::auth::ApiKeySecret,
    /// Policy gate for network access control.
    policy_gate: Option<Arc<dyn PolicyGate>>,
}

impl GrokClient {
    /// Construct from `AppConfig`.
    ///
    /// Reads the API key from the environment variable specified in config
    /// (defaulting to `XAI_API_KEY`). The provided policy gate is evaluated
    /// before every outbound HTTP request. If `None`, `HttpClient` uses its
    /// built-in deny-by-default gate.
    ///
    /// # Errors
    ///
    /// Returns `TransportError::Auth` if the API key env var is missing or
    /// empty. Returns `TransportError::Http` if the underlying reqwest client
    /// cannot be constructed.
    pub fn from_config(
        config: &AppConfig,
        policy_gate: Option<Arc<dyn PolicyGate>>,
    ) -> Result<Self, TransportError> {
        let api_config = config.api.as_ref();

        let api_key_env = api_config
            .and_then(|a| a.api_key_env.as_deref())
            .unwrap_or("XAI_API_KEY");
        let api_key = resolve_api_key(api_key_env)?;

        let base_url = api_config
            .and_then(|a| a.base_url.clone())
            .unwrap_or_else(|| "https://api.x.ai".into());

        let timeout_secs = api_config.and_then(|a| a.timeout_secs).unwrap_or(120);
        let max_retries = api_config.and_then(|a| a.max_retries).unwrap_or(3);

        let api_key_clone = api_key.clone();
        let policy_gate_clone = policy_gate.clone();
        let base_url_clone = base_url.clone();

        let http_config = HttpClientConfig {
            base_url,
            timeout: Duration::from_secs(timeout_secs),
            retry: RetryConfig {
                max_retries,
                ..RetryConfig::default()
            },
            api_key_env: Some(api_key_env.to_owned()),
        };

        let http = HttpClient::new(http_config, api_key, policy_gate)?;
        Ok(Self {
            http: Arc::new(http),
            session_id: None,
            base_url: base_url_clone,
            api_key: api_key_clone,
            policy_gate: policy_gate_clone,
        })
    }

    /// Associate this client with a session ID for trust-level-aware tracking.
    ///
    /// Returns `self` for chaining. Session association is optional — the
    /// client works without one for simple scripts.
    #[must_use]
    pub fn with_session(mut self, session_id: impl Into<String>) -> Self {
        self.session_id = Some(session_id.into());
        self
    }

    /// Return the associated session ID, if any.
    #[must_use]
    pub fn session_id(&self) -> Option<&str> {
        self.session_id.as_deref()
    }

    /// Access the Responses API client (`POST /v1/responses`, etc.).
    #[must_use]
    pub fn responses(&self) -> ResponsesClient {
        ResponsesClient::new(Arc::clone(&self.http))
    }

    /// Access the Chat Completions API client (legacy).
    #[allow(deprecated)]
    #[must_use]
    pub fn chat(&self) -> ChatClient<'_> {
        ChatClient::new(&self.http)
    }

    /// Access the Models API client.
    #[must_use]
    pub fn models(&self) -> ModelsClient {
        ModelsClient::new(Arc::clone(&self.http))
    }

    /// Access the Images API client.
    #[must_use]
    pub fn images(&self) -> ImagesClient {
        ImagesClient::new(Arc::clone(&self.http))
    }

    /// Access the Videos API client.
    #[must_use]
    pub fn videos(&self) -> VideosClient {
        VideosClient::new(Arc::clone(&self.http))
    }

    /// Access the Text-to-Speech API client.
    #[must_use]
    pub fn tts(&self) -> TtsClient {
        TtsClient::new(Arc::clone(&self.http))
    }

    /// Access the Files API client.
    #[must_use]
    pub fn files(&self) -> FilesClient<'_> {
        FilesClient::new(&self.http)
    }

    /// Access the Batches API client.
    #[must_use]
    pub fn batches(&self) -> BatchesClient {
        BatchesClient::new(Arc::clone(&self.http))
    }

    /// Access the Tokenize API client.
    #[must_use]
    pub fn tokenize(&self) -> TokenizeClient<'_> {
        TokenizeClient::new(&self.http)
    }

    /// Access the API Key info client.
    #[must_use]
    pub fn api_key(&self) -> ApiKeyClient<'_> {
        ApiKeyClient::new(&self.http)
    }

    /// Access the Document Search API client (`POST /v1/documents/search`).
    ///
    /// Document search uses the standard inference API key — it does not
    /// depend on the Collections Management API or management key.
    #[must_use]
    pub fn documents(&self) -> DocumentsClient {
        DocumentsClient::new(Arc::clone(&self.http))
    }

    /// Create a Voice Agent API client for WebSocket-based voice conversations.
    ///
    /// The voice agent uses a separate WebSocket transport (not `HttpClient`),
    /// so this method constructs a `VoiceAgentClient` with the API key and
    /// policy gate from this `GrokClient`.
    ///
    /// An optional `WsClientConfig` can be provided to customize WebSocket
    /// behavior (heartbeat interval, reconnect policy, etc.). If `None`,
    /// defaults are used with the base URL derived from the HTTP base URL.
    #[must_use]
    pub fn voice_agent(&self, ws_config: Option<WsClientConfig>) -> VoiceAgentClient {
        let config = ws_config.unwrap_or_else(|| {
            let ws_base = if self.base_url.starts_with("https://") {
                format!("wss://{}", &self.base_url[8..])
            } else if self.base_url.starts_with("http://") {
                format!("ws://{}", &self.base_url[7..])
            } else {
                self.base_url.clone()
            };
            WsClientConfig {
                base_url: ws_base,
                ..WsClientConfig::default()
            }
        });
        VoiceAgentClient::new(config, self.api_key.clone(), self.policy_gate.clone())
    }
}

// Intentionally omits `session_id`, `base_url`, `api_key` (secret), and
// `policy_gate` (not Debug-printable) from the Debug output.
#[allow(clippy::missing_fields_in_debug)]
impl std::fmt::Debug for GrokClient {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("GrokClient")
            .field("http", &self.http)
            .finish_non_exhaustive()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use grokrs_core::{
        ApiConfig, AppConfig, ModelConfig, PolicyConfig, SessionConfig, WorkspaceConfig,
    };
    use serial_test::serial;

    /// Build a minimal `AppConfig` for testing.
    fn test_config() -> AppConfig {
        AppConfig {
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
                api_key_env: Some("GROKRS_TEST_CLIENT_KEY".into()),
                base_url: Some("https://api.x.ai".into()),
                timeout_secs: Some(60),
                max_retries: Some(2),
            }),
            management_api: None,
            store: None,
            agent: None,
            chat: None,
            mcp: None,
        }
    }

    /// Build a minimal `AppConfig` without the `[api]` section.
    fn test_config_no_api() -> AppConfig {
        AppConfig {
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
            api: None,
            management_api: None,
            store: None,
            agent: None,
            chat: None,
            mcp: None,
        }
    }

    #[test]
    #[serial]
    fn from_config_constructs_ok_with_valid_env_var() {
        let config = test_config();
        // SAFETY: `set_var`/`remove_var` are unsafe in edition 2024 because
        // concurrent writes to the same env var are UB. The `#[serial]`
        // attribute ensures no other test mutates this variable concurrently.
        unsafe {
            std::env::set_var("GROKRS_TEST_CLIENT_KEY", "test-key-for-facade");
        }
        let result = GrokClient::from_config(&config, None);
        // SAFETY: `set_var`/`remove_var` are unsafe in edition 2024 because
        // concurrent writes to the same env var are UB. The `#[serial]`
        // attribute ensures no other test mutates this variable concurrently.
        unsafe {
            std::env::remove_var("GROKRS_TEST_CLIENT_KEY");
        }
        assert!(result.is_ok(), "expected Ok, got: {result:?}");
    }

    #[test]
    #[serial]
    fn from_config_fails_when_env_var_missing() {
        // Use a unique env var name that no other test touches to avoid
        // race conditions with parallel test execution.
        let mut config = test_config();
        let unique_var = "GROKRS_TEST_CLIENT_MISSING_VAR_UNIQUE";
        config.api = Some(ApiConfig {
            api_key_env: Some(unique_var.into()),
            base_url: Some("https://api.x.ai".into()),
            timeout_secs: Some(60),
            max_retries: Some(2),
        });
        // SAFETY: `set_var`/`remove_var` are unsafe in edition 2024 because
        // concurrent writes to the same env var are UB. The `#[serial]`
        // attribute ensures no other test mutates this variable concurrently.
        unsafe {
            std::env::remove_var(unique_var);
        }
        let result = GrokClient::from_config(&config, None);
        assert!(result.is_err());
        match result.unwrap_err() {
            TransportError::Auth { message } => {
                assert!(
                    message.contains(unique_var),
                    "error should mention the env var name: {message}"
                );
            }
            other => panic!("expected Auth error, got: {other}"),
        }
    }

    #[test]
    #[serial]
    fn from_config_without_api_section_falls_back_to_defaults() {
        let config = test_config_no_api();
        // Set the default XAI_API_KEY var
        let var_name = "XAI_API_KEY";
        let had_value = std::env::var(var_name).ok();
        // SAFETY: `set_var`/`remove_var` are unsafe in edition 2024 because
        // concurrent writes to the same env var are UB. The `#[serial]`
        // attribute ensures no other test mutates this variable concurrently.
        unsafe {
            std::env::set_var(var_name, "fallback-test-key");
        }
        let result = GrokClient::from_config(&config, None);
        // Restore original state
        // SAFETY: `set_var`/`remove_var` are unsafe in edition 2024 because
        // concurrent writes to the same env var are UB. The `#[serial]`
        // attribute ensures no other test mutates this variable concurrently.
        match had_value {
            Some(v) => unsafe { std::env::set_var(var_name, v) },
            None => unsafe { std::env::remove_var(var_name) },
        }
        assert!(
            result.is_ok(),
            "expected Ok with default fallbacks, got: {result:?}"
        );
    }

    #[allow(deprecated)]
    #[test]
    #[serial]
    fn sub_client_accessors_return_correct_types() {
        use crate::endpoints::chat::ChatClient;

        let config = test_config();
        // SAFETY: `set_var`/`remove_var` are unsafe in edition 2024 because
        // concurrent writes to the same env var are UB. The `#[serial]`
        // attribute ensures no other test mutates this variable concurrently.
        unsafe {
            std::env::set_var("GROKRS_TEST_CLIENT_KEY", "accessor-test-key");
        }
        let client = GrokClient::from_config(&config, None).unwrap();
        // SAFETY: `set_var`/`remove_var` are unsafe in edition 2024 because
        // concurrent writes to the same env var are UB. The `#[serial]`
        // attribute ensures no other test mutates this variable concurrently.
        unsafe {
            std::env::remove_var("GROKRS_TEST_CLIENT_KEY");
        }

        // Verify each accessor compiles and returns the correct type.
        // We cannot call actual API methods without a server, but we can
        // verify the types are correct by binding them.
        let _responses: ResponsesClient = client.responses();
        let _chat: ChatClient<'_> = client.chat();
        let _models: ModelsClient = client.models();
        let _images: ImagesClient = client.images();
        let _videos: VideosClient = client.videos();
        let _files: FilesClient<'_> = client.files();
        let _batches: BatchesClient = client.batches();
        let _tokenize: TokenizeClient<'_> = client.tokenize();
        let _tts: TtsClient = client.tts();
        let _api_key: ApiKeyClient<'_> = client.api_key();
    }

    #[test]
    #[serial]
    fn from_config_with_policy_gate() {
        use crate::transport::policy_gate::AllowAllGate;

        let config = test_config();
        // SAFETY: `set_var`/`remove_var` are unsafe in edition 2024 because
        // concurrent writes to the same env var are UB. The `#[serial]`
        // attribute ensures no other test mutates this variable concurrently.
        unsafe {
            std::env::set_var("GROKRS_TEST_CLIENT_KEY", "gate-test-key");
        }
        let gate: Option<Arc<dyn PolicyGate>> = Some(Arc::new(AllowAllGate));
        let result = GrokClient::from_config(&config, gate);
        // SAFETY: `set_var`/`remove_var` are unsafe in edition 2024 because
        // concurrent writes to the same env var are UB. The `#[serial]`
        // attribute ensures no other test mutates this variable concurrently.
        unsafe {
            std::env::remove_var("GROKRS_TEST_CLIENT_KEY");
        }
        assert!(result.is_ok());
    }

    #[test]
    #[serial]
    fn debug_output_does_not_contain_api_key() {
        let config = test_config();
        // SAFETY: `set_var`/`remove_var` are unsafe in edition 2024 because
        // concurrent writes to the same env var are UB. The `#[serial]`
        // attribute ensures no other test mutates this variable concurrently.
        unsafe {
            std::env::set_var("GROKRS_TEST_CLIENT_KEY", "super-secret-debug-test");
        }
        let client = GrokClient::from_config(&config, None).unwrap();
        // SAFETY: `set_var`/`remove_var` are unsafe in edition 2024 because
        // concurrent writes to the same env var are UB. The `#[serial]`
        // attribute ensures no other test mutates this variable concurrently.
        unsafe {
            std::env::remove_var("GROKRS_TEST_CLIENT_KEY");
        }

        let debug = format!("{client:?}");
        assert!(
            debug.contains("[REDACTED]"),
            "debug output should redact the key"
        );
        assert!(
            !debug.contains("super-secret-debug-test"),
            "debug output must not contain the raw API key"
        );
    }

    #[test]
    #[serial]
    fn from_config_uses_custom_timeout_and_retries() {
        let mut config = test_config();
        config.api = Some(ApiConfig {
            api_key_env: Some("GROKRS_TEST_CLIENT_KEY".into()),
            base_url: Some("https://custom.example.com".into()),
            timeout_secs: Some(30),
            max_retries: Some(5),
        });
        // SAFETY: `set_var`/`remove_var` are unsafe in edition 2024 because
        // concurrent writes to the same env var are UB. The `#[serial]`
        // attribute ensures no other test mutates this variable concurrently.
        unsafe {
            std::env::set_var("GROKRS_TEST_CLIENT_KEY", "custom-config-key");
        }
        let result = GrokClient::from_config(&config, None);
        // SAFETY: `set_var`/`remove_var` are unsafe in edition 2024 because
        // concurrent writes to the same env var are UB. The `#[serial]`
        // attribute ensures no other test mutates this variable concurrently.
        unsafe {
            std::env::remove_var("GROKRS_TEST_CLIENT_KEY");
        }
        assert!(result.is_ok());
    }

    #[test]
    #[serial]
    fn from_config_partial_api_section_uses_defaults() {
        let mut config = test_config();
        // Only api_key_env set, rest are None
        config.api = Some(ApiConfig {
            api_key_env: Some("GROKRS_TEST_CLIENT_KEY".into()),
            base_url: None,
            timeout_secs: None,
            max_retries: None,
        });
        // SAFETY: `set_var`/`remove_var` are unsafe in edition 2024 because
        // concurrent writes to the same env var are UB. The `#[serial]`
        // attribute ensures no other test mutates this variable concurrently.
        unsafe {
            std::env::set_var("GROKRS_TEST_CLIENT_KEY", "partial-config-key");
        }
        let result = GrokClient::from_config(&config, None);
        // SAFETY: `set_var`/`remove_var` are unsafe in edition 2024 because
        // concurrent writes to the same env var are UB. The `#[serial]`
        // attribute ensures no other test mutates this variable concurrently.
        unsafe {
            std::env::remove_var("GROKRS_TEST_CLIENT_KEY");
        }
        assert!(result.is_ok());
    }
}
