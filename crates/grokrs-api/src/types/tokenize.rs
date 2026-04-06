use serde::{Deserialize, Serialize};

/// Request body for the tokenize-text endpoint.
///
/// Sent as `POST /v1/tokenize-text`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TokenizeRequest {
    /// The text to tokenize.
    pub text: String,

    /// The model whose tokenizer should be used.
    pub model: String,
}

/// Response from the tokenize-text endpoint.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TokenizeResponse {
    /// The list of tokens produced by the tokenizer.
    pub token_ids: Vec<TokenInfo>,
}

/// A single token from the tokenizer output.
///
/// `token_bytes` is `Vec<u8>` because tokenizers may produce byte sequences
/// that are not valid UTF-8 (e.g., partial multi-byte characters).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TokenInfo {
    /// The numeric token identifier.
    pub token_id: u32,

    /// The string representation of the token, if available.
    pub string_token: String,

    /// The raw byte representation of the token.
    ///
    /// Deserialized from a JSON array of numbers (e.g., `[72, 101, 108]`).
    /// May contain bytes that do not form valid UTF-8.
    #[serde(default)]
    pub token_bytes: Vec<u8>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tokenize_request_serializes_correctly() {
        let req = TokenizeRequest {
            text: "Hello, world!".into(),
            model: "grok-4".into(),
        };
        let json = serde_json::to_string(&req).unwrap();
        assert!(json.contains("\"text\":\"Hello, world!\""));
        assert!(json.contains("\"model\":\"grok-4\""));
    }

    #[test]
    fn tokenize_request_round_trips() {
        let req = TokenizeRequest {
            text: "test input".into(),
            model: "grok-4-mini".into(),
        };
        let json = serde_json::to_string(&req).unwrap();
        let back: TokenizeRequest = serde_json::from_str(&json).unwrap();
        assert_eq!(req, back);
    }

    #[test]
    fn tokenize_response_deserializes_with_token_ids() {
        let json = r#"{
            "token_ids": [
                {
                    "token_id": 9906,
                    "string_token": "Hello",
                    "token_bytes": [72, 101, 108, 108, 111]
                },
                {
                    "token_id": 11,
                    "string_token": ",",
                    "token_bytes": [44]
                }
            ]
        }"#;
        let resp: TokenizeResponse = serde_json::from_str(json).unwrap();
        assert_eq!(resp.token_ids.len(), 2);
        assert_eq!(resp.token_ids[0].token_id, 9906);
        assert_eq!(resp.token_ids[0].string_token, "Hello");
        assert_eq!(resp.token_ids[0].token_bytes, vec![72, 101, 108, 108, 111]);
        assert_eq!(resp.token_ids[1].token_id, 11);
        assert_eq!(resp.token_ids[1].string_token, ",");
        assert_eq!(resp.token_ids[1].token_bytes, vec![44]);
    }

    #[test]
    fn tokenize_response_round_trips() {
        let resp = TokenizeResponse {
            token_ids: vec![
                TokenInfo {
                    token_id: 100,
                    string_token: "foo".into(),
                    token_bytes: vec![102, 111, 111],
                },
                TokenInfo {
                    token_id: 200,
                    string_token: "bar".into(),
                    token_bytes: vec![98, 97, 114],
                },
            ],
        };
        let json = serde_json::to_string(&resp).unwrap();
        let back: TokenizeResponse = serde_json::from_str(&json).unwrap();
        assert_eq!(resp, back);
    }

    #[test]
    fn token_bytes_handles_non_utf8() {
        // Bytes 0xFF, 0xFE are not valid UTF-8 start bytes
        let json = r#"{
            "token_id": 42,
            "string_token": "\ufffd",
            "token_bytes": [255, 254, 128]
        }"#;
        let token: TokenInfo = serde_json::from_str(json).unwrap();
        assert_eq!(token.token_id, 42);
        assert_eq!(token.token_bytes, vec![255, 254, 128]);
        // Confirm these bytes are NOT valid UTF-8
        assert!(std::str::from_utf8(&token.token_bytes).is_err());
    }

    #[test]
    fn token_bytes_defaults_to_empty_when_missing() {
        let json = r#"{
            "token_id": 1,
            "string_token": "a"
        }"#;
        let token: TokenInfo = serde_json::from_str(json).unwrap();
        assert_eq!(token.token_id, 1);
        assert_eq!(token.string_token, "a");
        assert!(token.token_bytes.is_empty());
    }

    #[test]
    fn token_bytes_empty_array() {
        let json = r#"{
            "token_id": 0,
            "string_token": "",
            "token_bytes": []
        }"#;
        let token: TokenInfo = serde_json::from_str(json).unwrap();
        assert!(token.token_bytes.is_empty());
    }

    #[test]
    fn tokenize_response_empty_token_ids() {
        let json = r#"{"token_ids": []}"#;
        let resp: TokenizeResponse = serde_json::from_str(json).unwrap();
        assert!(resp.token_ids.is_empty());
    }
}
