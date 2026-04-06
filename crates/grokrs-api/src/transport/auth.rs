use std::env;
use std::fmt;

use crate::transport::error::TransportError;

/// A newtype wrapping an API key that redacts its value in `Debug` output.
///
/// The API key must NEVER appear in logs, error messages, or debug output.
/// This type enforces that invariant by implementing `Debug` with a
/// `[REDACTED]` placeholder.
#[derive(Clone)]
pub struct ApiKeySecret {
    inner: String,
}

impl ApiKeySecret {
    /// Create a new `ApiKeySecret` from a raw key string.
    pub fn new(key: impl Into<String>) -> Self {
        Self { inner: key.into() }
    }

    /// Access the raw key value for use in HTTP headers.
    ///
    /// Callers must ensure this value is never logged or included in
    /// error messages.
    #[must_use]
    pub fn expose(&self) -> &str {
        &self.inner
    }
}

impl fmt::Debug for ApiKeySecret {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ApiKeySecret")
            .field("inner", &"[REDACTED]")
            .finish()
    }
}

impl fmt::Display for ApiKeySecret {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "[REDACTED]")
    }
}

/// Resolve an API key from an environment variable.
///
/// Returns `TransportError::Auth` if the variable is not set or empty.
/// The key is wrapped in `ApiKeySecret` to prevent accidental logging.
///
/// # Errors
///
/// Returns [`TransportError::Auth`] if the environment variable is not set
/// or is set to an empty string.
pub fn resolve_api_key(env_var_name: &str) -> Result<ApiKeySecret, TransportError> {
    match env::var(env_var_name) {
        Ok(val) if val.is_empty() => Err(TransportError::Auth {
            message: format!("environment variable '{env_var_name}' is set but empty"),
        }),
        Ok(val) => Ok(ApiKeySecret::new(val)),
        Err(_) => Err(TransportError::Auth {
            message: format!("environment variable '{env_var_name}' is not set"),
        }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn api_key_secret_debug_is_redacted() {
        let secret = ApiKeySecret::new("xai-super-secret-key-12345");
        let debug = format!("{secret:?}");
        assert!(debug.contains("[REDACTED]"));
        assert!(!debug.contains("xai-super-secret-key-12345"));
    }

    #[test]
    fn api_key_secret_display_is_redacted() {
        let secret = ApiKeySecret::new("xai-super-secret-key-12345");
        let display = format!("{secret}");
        assert_eq!(display, "[REDACTED]");
        assert!(!display.contains("xai-super-secret-key-12345"));
    }

    #[test]
    fn api_key_secret_expose_returns_raw_value() {
        let secret = ApiKeySecret::new("xai-key-abc");
        assert_eq!(secret.expose(), "xai-key-abc");
    }

    #[test]
    fn resolve_api_key_from_env() {
        // Use a unique env var name to avoid conflicts with other tests
        let var_name = "GROKRS_TEST_API_KEY_RESOLVE";
        // SAFETY: Test-only env manipulation; test runner serializes these tests.
        unsafe { env::set_var(var_name, "test-key-value") };
        let result = resolve_api_key(var_name);
        unsafe { env::remove_var(var_name) };

        let secret = result.expect("should resolve key");
        assert_eq!(secret.expose(), "test-key-value");
    }

    #[test]
    fn resolve_api_key_missing_env_var() {
        let var_name = "GROKRS_TEST_MISSING_VAR_THAT_DEFINITELY_DOES_NOT_EXIST";
        // SAFETY: Test-only env manipulation; test runner serializes these tests.
        unsafe { env::remove_var(var_name) };
        let result = resolve_api_key(var_name);
        assert!(result.is_err());
        match result.unwrap_err() {
            TransportError::Auth { message } => {
                assert!(message.contains("is not set"));
            }
            other => panic!("expected Auth error, got: {other}"),
        }
    }

    #[test]
    fn resolve_api_key_empty_env_var() {
        let var_name = "GROKRS_TEST_EMPTY_API_KEY";
        // SAFETY: Test-only env manipulation; test runner serializes these tests.
        unsafe { env::set_var(var_name, "") };
        let result = resolve_api_key(var_name);
        unsafe { env::remove_var(var_name) };

        assert!(result.is_err());
        match result.unwrap_err() {
            TransportError::Auth { message } => {
                assert!(message.contains("empty"));
            }
            other => panic!("expected Auth error, got: {other}"),
        }
    }
}
