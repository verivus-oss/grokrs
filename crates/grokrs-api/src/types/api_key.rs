use serde::{Deserialize, Serialize};

/// Information about the authenticated API key.
///
/// Returned by `GET /v1/api-key`. All fields are optional because the API
/// may return a subset of fields depending on the key's permissions, and
/// future fields should be accepted without breaking deserialization.
///
/// This type must NOT be cached — always fetch fresh from the API to ensure
/// the key status, ACLs, and blocked/disabled state are current.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ApiKeyInfo {
    /// The human-readable name assigned to this API key.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,

    /// The current status of the API key (e.g., "active", "revoked").
    #[serde(skip_serializing_if = "Option::is_none")]
    pub status: Option<String>,

    /// Access control list entries for this key.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub acls: Option<Vec<String>>,

    /// The team identifier this key belongs to.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub team_id: Option<String>,

    /// Whether the key has been blocked.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub blocked: Option<bool>,

    /// Whether the key has been disabled.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub disabled: Option<bool>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn api_key_info_full_round_trips() {
        let info = ApiKeyInfo {
            name: Some("my-key".into()),
            status: Some("active".into()),
            acls: Some(vec!["chat".into(), "files".into()]),
            team_id: Some("team-abc".into()),
            blocked: Some(false),
            disabled: Some(false),
        };
        let json = serde_json::to_string(&info).unwrap();
        let back: ApiKeyInfo = serde_json::from_str(&json).unwrap();
        assert_eq!(info, back);
    }

    #[test]
    fn api_key_info_minimal_round_trips() {
        let info = ApiKeyInfo {
            name: None,
            status: None,
            acls: None,
            team_id: None,
            blocked: None,
            disabled: None,
        };
        let json = serde_json::to_string(&info).unwrap();
        // All fields omitted
        assert_eq!(json, "{}");
        let back: ApiKeyInfo = serde_json::from_str(&json).unwrap();
        assert_eq!(info, back);
    }

    #[test]
    fn api_key_info_deserializes_with_unknown_fields() {
        let json = r#"{
            "name": "test-key",
            "status": "active",
            "some_future_field": "should be ignored",
            "another_new_field": 42,
            "nested_unknown": {"a": 1}
        }"#;
        let info: ApiKeyInfo = serde_json::from_str(json).unwrap();
        assert_eq!(info.name.as_deref(), Some("test-key"));
        assert_eq!(info.status.as_deref(), Some("active"));
        assert!(info.acls.is_none());
    }

    #[test]
    fn api_key_info_blocked_and_disabled_flags() {
        let json = r#"{"blocked": true, "disabled": true}"#;
        let info: ApiKeyInfo = serde_json::from_str(json).unwrap();
        assert_eq!(info.blocked, Some(true));
        assert_eq!(info.disabled, Some(true));
    }

    #[test]
    fn api_key_info_empty_acls() {
        let json = r#"{"acls": []}"#;
        let info: ApiKeyInfo = serde_json::from_str(json).unwrap();
        assert_eq!(info.acls.as_deref(), Some(&[][..]));
    }

    #[test]
    fn api_key_info_skips_none_fields_on_serialize() {
        let info = ApiKeyInfo {
            name: Some("k".into()),
            status: None,
            acls: None,
            team_id: None,
            blocked: None,
            disabled: None,
        };
        let json = serde_json::to_string(&info).unwrap();
        assert!(json.contains("\"name\":\"k\""));
        assert!(!json.contains("status"));
        assert!(!json.contains("acls"));
        assert!(!json.contains("team_id"));
        assert!(!json.contains("blocked"));
        assert!(!json.contains("disabled"));
    }

    #[test]
    fn api_key_info_deserializes_from_wire_format() {
        let json = r#"{
            "name": "production-key",
            "status": "active",
            "acls": ["chat:completions", "files:read", "files:write"],
            "team_id": "team-prod-001",
            "blocked": false,
            "disabled": false
        }"#;
        let info: ApiKeyInfo = serde_json::from_str(json).unwrap();
        assert_eq!(info.name.as_deref(), Some("production-key"));
        assert_eq!(info.status.as_deref(), Some("active"));
        let acls = info.acls.unwrap();
        assert_eq!(acls.len(), 3);
        assert_eq!(acls[0], "chat:completions");
        assert_eq!(info.team_id.as_deref(), Some("team-prod-001"));
        assert_eq!(info.blocked, Some(false));
        assert_eq!(info.disabled, Some(false));
    }
}
