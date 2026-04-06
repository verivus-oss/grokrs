//! Management API client.
//!
//! `ManagementClient` is the unified facade for the xAI Collections Management
//! API. It wraps `HttpClient` with the management base URL and management API
//! key. It goes through the same `PolicyGate` as `GrokClient`.

use std::sync::Arc;
use std::time::Duration;

use crate::management::collections::CollectionsClient;
use crate::transport::auth::resolve_api_key;
use crate::transport::client::{HttpClient, HttpClientConfig};
use crate::transport::error::TransportError;
use crate::transport::policy_gate::PolicyGate;
use crate::transport::retry::RetryConfig;
use grokrs_core::AppConfig;

/// Default base URL for the xAI Collections Management API.
const DEFAULT_MANAGEMENT_BASE_URL: &str = "https://management-api.x.ai";

/// Default environment variable name for the Management API key.
const DEFAULT_MANAGEMENT_KEY_ENV: &str = "XAI_MANAGEMENT_API_KEY";

/// Unified client for the xAI Collections Management API.
///
/// Constructed from `AppConfig` with an injected policy gate. The management
/// client uses a different base URL and API key than `GrokClient`, but shares
/// the same `HttpClient` infrastructure (retry, policy gate, auth pattern).
///
/// # Important
///
/// The management API key is **separate** from the inference API key. They must
/// not be confused. The management key env var defaults to `XAI_MANAGEMENT_API_KEY`.
pub struct ManagementClient {
    http: Arc<HttpClient>,
}

impl ManagementClient {
    /// Construct from `AppConfig`.
    ///
    /// Reads the management API key from the environment variable specified in
    /// the `[management_api]` config section (defaulting to `XAI_MANAGEMENT_API_KEY`).
    /// The provided policy gate is evaluated before every outbound HTTP request.
    ///
    /// # Errors
    ///
    /// Returns `TransportError::Auth` if the management key env var is missing
    /// or empty. Returns `TransportError::Http` if the underlying reqwest client
    /// cannot be constructed.
    pub fn from_config(
        config: &AppConfig,
        policy_gate: Option<Arc<dyn PolicyGate>>,
    ) -> Result<Self, TransportError> {
        let mgmt_config = config.management_api.as_ref();

        let key_env = mgmt_config
            .and_then(|m| m.management_key_env.as_deref())
            .unwrap_or(DEFAULT_MANAGEMENT_KEY_ENV);
        let api_key = resolve_api_key(key_env)?;

        let base_url = mgmt_config
            .and_then(|m| m.base_url.clone())
            .unwrap_or_else(|| DEFAULT_MANAGEMENT_BASE_URL.into());

        let timeout_secs = mgmt_config.and_then(|m| m.timeout_secs).unwrap_or(120);
        let max_retries = mgmt_config.and_then(|m| m.max_retries).unwrap_or(3);

        let http_config = HttpClientConfig {
            base_url,
            timeout: Duration::from_secs(timeout_secs),
            retry: RetryConfig {
                max_retries,
                ..RetryConfig::default()
            },
            api_key_env: Some(key_env.to_owned()),
        };

        let http = HttpClient::new(http_config, api_key, policy_gate)?;
        Ok(Self {
            http: Arc::new(http),
        })
    }

    /// Access the Collections API client.
    #[must_use]
    pub fn collections(&self) -> CollectionsClient {
        CollectionsClient::new(Arc::clone(&self.http))
    }
}

impl std::fmt::Debug for ManagementClient {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ManagementClient")
            .field("http", &self.http)
            .finish()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use grokrs_core::{
        AppConfig, ManagementApiConfig, ModelConfig, PolicyConfig, SessionConfig, WorkspaceConfig,
    };

    fn base_config() -> AppConfig {
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
            store: None,
            agent: None,
            chat: None,
            mcp: None,
            management_api: Some(ManagementApiConfig {
                management_key_env: Some("GROKRS_TEST_MGMT_KEY".into()),
                base_url: Some("https://management-api.x.ai".into()),
                timeout_secs: Some(60),
                max_retries: Some(2),
            }),
        }
    }

    #[test]
    fn from_config_constructs_ok_with_valid_env_var() {
        let config = base_config();
        // SAFETY: Test-only env manipulation; test runner serializes these tests.
        unsafe {
            std::env::set_var("GROKRS_TEST_MGMT_KEY", "test-mgmt-key");
        }
        let result = ManagementClient::from_config(&config, None);
        // SAFETY: Test-only env manipulation; test runner serializes these tests.
        unsafe {
            std::env::remove_var("GROKRS_TEST_MGMT_KEY");
        }
        assert!(result.is_ok(), "expected Ok, got: {result:?}");
    }

    #[test]
    fn from_config_fails_when_env_var_missing() {
        let config = base_config();
        let unique_var = "GROKRS_TEST_MGMT_MISSING_VAR";
        let mut cfg = config;
        cfg.management_api = Some(ManagementApiConfig {
            management_key_env: Some(unique_var.into()),
            base_url: None,
            timeout_secs: None,
            max_retries: None,
        });
        // SAFETY: Test-only env manipulation; test runner serializes these tests.
        unsafe {
            std::env::remove_var(unique_var);
        }
        let result = ManagementClient::from_config(&cfg, None);
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
    fn from_config_without_management_section_uses_defaults() {
        let mut config = base_config();
        config.management_api = None;
        // Without management_api section, it defaults to XAI_MANAGEMENT_API_KEY
        // SAFETY: Test-only env manipulation; test runner serializes these tests.
        unsafe {
            std::env::set_var("XAI_MANAGEMENT_API_KEY", "default-mgmt-key");
        }
        let result = ManagementClient::from_config(&config, None);
        // SAFETY: Test-only env manipulation; test runner serializes these tests.
        unsafe {
            std::env::remove_var("XAI_MANAGEMENT_API_KEY");
        }
        assert!(result.is_ok());
    }

    #[test]
    fn debug_output_does_not_contain_management_key() {
        let config = base_config();
        // SAFETY: Test-only env manipulation; test runner serializes these tests.
        unsafe {
            std::env::set_var("GROKRS_TEST_MGMT_KEY", "super-secret-mgmt-key");
        }
        let client = ManagementClient::from_config(&config, None).unwrap();
        // SAFETY: Test-only env manipulation; test runner serializes these tests.
        unsafe {
            std::env::remove_var("GROKRS_TEST_MGMT_KEY");
        }

        let debug = format!("{client:?}");
        assert!(
            debug.contains("[REDACTED]"),
            "debug output should redact the key"
        );
        assert!(
            !debug.contains("super-secret-mgmt-key"),
            "debug output must not contain the raw management API key"
        );
    }

    #[test]
    fn from_config_with_policy_gate() {
        use crate::transport::policy_gate::AllowAllGate;

        let config = base_config();
        // SAFETY: Test-only env manipulation; test runner serializes these tests.
        unsafe {
            std::env::set_var("GROKRS_TEST_MGMT_KEY", "gate-mgmt-key");
        }
        let gate: Option<Arc<dyn PolicyGate>> = Some(Arc::new(AllowAllGate));
        let result = ManagementClient::from_config(&config, gate);
        // SAFETY: Test-only env manipulation; test runner serializes these tests.
        unsafe {
            std::env::remove_var("GROKRS_TEST_MGMT_KEY");
        }
        assert!(result.is_ok());
    }

    #[test]
    fn collections_accessor_returns_client() {
        let config = base_config();
        // SAFETY: Test-only env manipulation; test runner serializes these tests.
        unsafe {
            std::env::set_var("GROKRS_TEST_MGMT_KEY", "accessor-mgmt-key");
        }
        let client = ManagementClient::from_config(&config, None).unwrap();
        // SAFETY: Test-only env manipulation; test runner serializes these tests.
        unsafe {
            std::env::remove_var("GROKRS_TEST_MGMT_KEY");
        }
        let _collections: CollectionsClient = client.collections();
    }
}
