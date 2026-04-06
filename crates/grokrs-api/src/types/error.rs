use serde::{Deserialize, Serialize};
use std::fmt;

/// The wire-format error envelope returned by the xAI API.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ApiErrorResponse {
    /// The error body containing details.
    pub error: ApiErrorBody,
}

/// The body of an API error response.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ApiErrorBody {
    /// A human-readable error message.
    pub message: String,
    /// The error type (e.g., "`invalid_request_error`").
    #[serde(skip_serializing_if = "Option::is_none")]
    pub r#type: Option<String>,
    /// An error code (e.g., "`model_not_found`").
    #[serde(skip_serializing_if = "Option::is_none")]
    pub code: Option<String>,
}

/// A structured API error with status code and request metadata.
///
/// This is not just the serde type -- it carries the HTTP status code and
/// optional request ID for diagnostics.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ApiError {
    /// The HTTP status code from the response.
    pub status_code: u16,
    /// A human-readable error message.
    pub message: String,
    /// The error type from the API response body.
    pub error_type: Option<String>,
    /// The error code from the API response body.
    pub code: Option<String>,
    /// The request ID from response headers, if present.
    pub request_id: Option<String>,
}

impl ApiError {
    /// Create an `ApiError` from an HTTP status code and a deserialized error response.
    #[must_use]
    pub fn from_response(status_code: u16, body: ApiErrorBody, request_id: Option<String>) -> Self {
        Self {
            status_code,
            message: body.message,
            error_type: body.r#type,
            code: body.code,
            request_id,
        }
    }

    /// Create an `ApiError` when the response body could not be parsed.
    #[must_use]
    pub fn from_status(status_code: u16, fallback_message: String) -> Self {
        Self {
            status_code,
            message: fallback_message,
            error_type: None,
            code: None,
            request_id: None,
        }
    }
}

impl fmt::Display for ApiError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "API error {}: {}", self.status_code, self.message)?;
        if let Some(ref t) = self.error_type {
            write!(f, " (type: {t})")?;
        }
        if let Some(ref c) = self.code {
            write!(f, " (code: {c})")?;
        }
        if let Some(ref rid) = self.request_id {
            write!(f, " [request_id: {rid}]")?;
        }
        Ok(())
    }
}

impl std::error::Error for ApiError {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn api_error_response_round_trips() {
        let resp = ApiErrorResponse {
            error: ApiErrorBody {
                message: "Model not found".into(),
                r#type: Some("invalid_request_error".into()),
                code: Some("model_not_found".into()),
            },
        };
        let json = serde_json::to_string(&resp).unwrap();
        let back: ApiErrorResponse = serde_json::from_str(&json).unwrap();
        assert_eq!(resp, back);
    }

    #[test]
    fn api_error_body_skips_none_fields() {
        let body = ApiErrorBody {
            message: "bad request".into(),
            r#type: None,
            code: None,
        };
        let json = serde_json::to_string(&body).unwrap();
        assert!(!json.contains("\"type\""));
        assert!(!json.contains("\"code\""));
    }

    #[test]
    fn api_error_display_full() {
        let err = ApiError {
            status_code: 400,
            message: "Invalid model".into(),
            error_type: Some("invalid_request_error".into()),
            code: Some("model_not_found".into()),
            request_id: Some("req_abc123".into()),
        };
        let display = format!("{err}");
        assert!(display.contains("400"));
        assert!(display.contains("Invalid model"));
        assert!(display.contains("invalid_request_error"));
        assert!(display.contains("model_not_found"));
        assert!(display.contains("req_abc123"));
    }

    #[test]
    fn api_error_display_minimal() {
        let err = ApiError::from_status(500, "Internal server error".into());
        let display = format!("{err}");
        assert!(display.contains("500"));
        assert!(display.contains("Internal server error"));
        assert!(!display.contains("type:"));
        assert!(!display.contains("code:"));
    }

    #[test]
    fn api_error_from_response() {
        let body = ApiErrorBody {
            message: "Rate limited".into(),
            r#type: Some("rate_limit_error".into()),
            code: None,
        };
        let err = ApiError::from_response(429, body, Some("req_xyz".into()));
        assert_eq!(err.status_code, 429);
        assert_eq!(err.message, "Rate limited");
        assert_eq!(err.error_type.as_deref(), Some("rate_limit_error"));
        assert_eq!(err.request_id.as_deref(), Some("req_xyz"));
    }

    #[test]
    fn api_error_is_std_error() {
        let err = ApiError::from_status(400, "bad".into());
        let _: &dyn std::error::Error = &err;
    }

    #[test]
    fn api_error_response_deserializes_with_unknown_fields() {
        let json = r#"{"error":{"message":"oops","param":"model","extra":true}}"#;
        let resp: ApiErrorResponse = serde_json::from_str(json).unwrap();
        assert_eq!(resp.error.message, "oops");
    }
}
