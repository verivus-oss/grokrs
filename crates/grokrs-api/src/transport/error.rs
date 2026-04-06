use std::fmt;

use crate::types::error::ApiError;

/// Errors that can occur during HTTP transport operations.
///
/// These errors cover the full spectrum from network-level failures through
/// API-level errors, policy denials, and SSE parsing issues.
#[derive(Debug)]
pub enum TransportError {
    /// An HTTP-level error from the underlying reqwest client.
    Http {
        /// The underlying reqwest error.
        source: reqwest::Error,
    },

    /// A structured API error parsed from the response body.
    Api(ApiError),

    /// The request was denied by the policy gate.
    PolicyDenied {
        /// The host that was denied.
        host: String,
        /// The reason for the denial.
        reason: String,
    },

    /// The request requires interactive approval before proceeding.
    ApprovalRequired {
        /// The host that requires approval.
        host: String,
    },

    /// The request timed out.
    Timeout,

    /// An error occurred while parsing the SSE stream.
    Sse {
        /// A description of the SSE parsing error.
        message: String,
    },

    /// Authentication error (missing or invalid API key).
    Auth {
        /// A description of the authentication error.
        message: String,
    },

    /// Failed to serialize a request body to JSON.
    Serialization {
        /// A description of the serialization error.
        message: String,
    },

    /// Failed to deserialize a response body from JSON.
    Deserialization {
        /// A description of the deserialization error.
        message: String,
    },

    /// The base URL is not a valid URL or contains disallowed components.
    InvalidBaseUrl {
        /// The URL that failed validation.
        url: String,
        /// The reason the URL is invalid.
        reason: String,
    },

    /// A WebSocket transport error.
    WebSocket {
        /// A description of the WebSocket error.
        message: String,
    },
}

impl fmt::Display for TransportError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            TransportError::Http { source } => write!(f, "HTTP error: {source}"),
            TransportError::Api(api_err) => write!(f, "{api_err}"),
            TransportError::PolicyDenied { host, reason } => {
                write!(f, "policy denied request to {host}: {reason}")
            }
            TransportError::ApprovalRequired { host } => {
                write!(
                    f,
                    "approval required for request to {host}: interactive approval is not yet implemented"
                )
            }
            TransportError::Timeout => write!(f, "request timed out"),
            TransportError::Sse { message } => write!(f, "SSE error: {message}"),
            TransportError::Auth { message } => write!(f, "authentication error: {message}"),
            TransportError::Serialization { message } => {
                write!(f, "serialization error: {message}")
            }
            TransportError::Deserialization { message } => {
                write!(f, "deserialization error: {message}")
            }
            TransportError::InvalidBaseUrl { url, reason } => {
                write!(f, "invalid base URL {url}: {reason}")
            }
            TransportError::WebSocket { message } => {
                write!(f, "WebSocket error: {message}")
            }
        }
    }
}

impl std::error::Error for TransportError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            TransportError::Http { source } => Some(source),
            TransportError::Api(api_err) => Some(api_err),
            _ => None,
        }
    }
}

impl From<reqwest::Error> for TransportError {
    fn from(err: reqwest::Error) -> Self {
        if err.is_timeout() {
            TransportError::Timeout
        } else {
            TransportError::Http { source: err }
        }
    }
}

impl From<ApiError> for TransportError {
    fn from(err: ApiError) -> Self {
        TransportError::Api(err)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::error::ApiError;

    #[test]
    fn transport_error_display_policy_denied() {
        let err = TransportError::PolicyDenied {
            host: "evil.example.com".into(),
            reason: "network access is denied by default".into(),
        };
        let display = format!("{err}");
        assert!(display.contains("evil.example.com"));
        assert!(display.contains("denied"));
    }

    #[test]
    fn transport_error_display_approval_required() {
        let err = TransportError::ApprovalRequired {
            host: "api.x.ai".into(),
        };
        let display = format!("{err}");
        assert!(display.contains("approval required"));
        assert!(display.contains("api.x.ai"));
        assert!(
            display.contains("not yet implemented"),
            "ApprovalRequired message must state that interactive approval is not yet implemented"
        );
    }

    #[test]
    fn transport_error_display_timeout() {
        let err = TransportError::Timeout;
        assert_eq!(format!("{err}"), "request timed out");
    }

    #[test]
    fn transport_error_display_sse() {
        let err = TransportError::Sse {
            message: "unexpected EOF".into(),
        };
        let display = format!("{err}");
        assert!(display.contains("SSE error"));
        assert!(display.contains("unexpected EOF"));
    }

    #[test]
    fn transport_error_display_api() {
        let api_err = ApiError::from_status(500, "server error".into());
        let err = TransportError::Api(api_err);
        let display = format!("{err}");
        assert!(display.contains("500"));
        assert!(display.contains("server error"));
    }

    #[test]
    fn transport_error_from_api_error() {
        let api_err = ApiError::from_status(400, "bad request".into());
        let err: TransportError = api_err.into();
        match err {
            TransportError::Api(e) => assert_eq!(e.status_code, 400),
            other => panic!("expected Api variant, got: {other}"),
        }
    }

    #[test]
    fn transport_error_is_std_error() {
        let err = TransportError::Timeout;
        let _: &dyn std::error::Error = &err;
    }

    #[test]
    fn transport_error_display_auth() {
        let err = TransportError::Auth {
            message: "key not set".into(),
        };
        let display = format!("{err}");
        assert!(display.contains("authentication error"));
        assert!(display.contains("key not set"));
    }

    #[test]
    fn transport_error_display_serialization() {
        let err = TransportError::Serialization {
            message: "invalid type".into(),
        };
        let display = format!("{err}");
        assert!(display.contains("serialization error"));
        assert!(display.contains("invalid type"));
    }

    #[test]
    fn transport_error_display_deserialization() {
        let err = TransportError::Deserialization {
            message: "missing field 'id'".into(),
        };
        let display = format!("{err}");
        assert!(display.contains("deserialization error"));
        assert!(display.contains("missing field 'id'"));
    }

    #[test]
    fn transport_error_display_invalid_base_url() {
        let err = TransportError::InvalidBaseUrl {
            url: "not-a-url".into(),
            reason: "relative URL without a base".into(),
        };
        let display = format!("{err}");
        assert!(display.contains("invalid base URL not-a-url"));
        assert!(display.contains("relative URL without a base"));
    }
}
