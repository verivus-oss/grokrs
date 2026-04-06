//! Text-to-Speech API endpoint client.
//!
//! This module provides `TtsClient`, which wraps `HttpClient` to send
//! requests to the xAI `/v1/tts` (generate audio), `/v1/tts/voices` (list
//! voices), and `/v1/tts/voices/{voice_id}` (get voice) endpoints.
//!
//! The generate endpoint returns raw audio bytes (Content-Type: audio/*),
//! not JSON, so `send_json_raw` is used instead of `send_json`.

use std::sync::Arc;

use reqwest::Method;

use super::util::encode_path_segment;
use crate::transport::client::HttpClient;
use crate::transport::error::TransportError;
use crate::types::tts::{TtsRequest, TtsVoice, TtsVoiceList};

/// The path for the TTS generation endpoint.
const TTS_GENERATE_PATH: &str = "/v1/tts";

/// The path for the TTS voices listing endpoint.
const TTS_VOICES_PATH: &str = "/v1/tts/voices";

/// The path prefix for fetching a single voice by ID.
const TTS_VOICE_PATH_PREFIX: &str = "/v1/tts/voices/";

/// A client for the xAI Text-to-Speech API.
///
/// Holds a shared reference to an `HttpClient` and provides typed methods for
/// generating audio, listing voices, and retrieving individual voice metadata.
#[derive(Debug, Clone)]
pub struct TtsClient {
    http: Arc<HttpClient>,
}

impl TtsClient {
    /// Create a new `TtsClient` from a shared `HttpClient`.
    #[must_use]
    pub fn new(http: Arc<HttpClient>) -> Self {
        Self { http }
    }

    /// Generate audio from text.
    ///
    /// Issues `POST /v1/tts` with the given request body. The response is raw
    /// audio bytes (not JSON), returned as `Vec<u8>`. The caller decides how
    /// to handle the audio (write to file, pipe to stdout, etc.).
    pub async fn generate(&self, request: &TtsRequest) -> Result<Vec<u8>, TransportError> {
        self.http
            .send_json_raw(Method::POST, TTS_GENERATE_PATH, request)
            .await
    }

    /// List all available TTS voices.
    ///
    /// Issues `GET /v1/tts/voices` and returns the parsed voice list.
    pub async fn list_voices(&self) -> Result<TtsVoiceList, TransportError> {
        self.http.send_no_body(Method::GET, TTS_VOICES_PATH).await
    }

    /// Get metadata for a single TTS voice.
    ///
    /// Issues `GET /v1/tts/voices/{voice_id}` and returns the parsed voice
    /// descriptor. The `voice_id` is percent-encoded to prevent URL corruption.
    pub async fn get_voice(&self, voice_id: &str) -> Result<TtsVoice, TransportError> {
        let path = format!("{}{}", TTS_VOICE_PATH_PREFIX, encode_path_segment(voice_id));
        self.http.send_no_body(Method::GET, &path).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::transport::auth::ApiKeySecret;
    use crate::transport::client::HttpClientConfig;
    use crate::transport::policy_gate::AllowAllGate;
    use crate::types::tts::{TtsOutputFormat, VoiceId};
    use wiremock::matchers::{body_partial_json, header, method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    /// Create an `HttpClient` pointed at the mock server.
    fn mock_http_client(base_url: &str) -> Arc<HttpClient> {
        let config = HttpClientConfig {
            base_url: base_url.to_string(),
            ..Default::default()
        };
        Arc::new(
            HttpClient::new(
                config,
                ApiKeySecret::new("test-key"),
                Some(Arc::new(AllowAllGate)),
            )
            .unwrap(),
        )
    }

    // -- Path construction ---------------------------------------------------

    #[test]
    fn generate_path_is_correct() {
        assert_eq!(TTS_GENERATE_PATH, "/v1/tts");
    }

    #[test]
    fn voices_path_is_correct() {
        assert_eq!(TTS_VOICES_PATH, "/v1/tts/voices");
    }

    #[test]
    fn voice_path_encodes_normal_id() {
        let voice_id = "eve";
        let path = format!("{}{}", TTS_VOICE_PATH_PREFIX, encode_path_segment(voice_id));
        assert_eq!(path, "/v1/tts/voices/eve");
    }

    #[test]
    fn voice_path_encodes_slash() {
        let voice_id = "custom/voice";
        let path = format!("{}{}", TTS_VOICE_PATH_PREFIX, encode_path_segment(voice_id));
        assert_eq!(path, "/v1/tts/voices/custom%2Fvoice");
    }

    // -- Endpoint integration tests ------------------------------------------

    #[tokio::test]
    async fn list_voices_sends_get_and_deserializes() {
        let server = MockServer::start().await;
        let response_body = serde_json::json!({
            "voices": [
                {
                    "voice_id": "eve",
                    "name": "Eve",
                    "language": "en-US"
                },
                {
                    "voice_id": "rex",
                    "name": "Rex"
                }
            ]
        });

        Mock::given(method("GET"))
            .and(path("/v1/tts/voices"))
            .and(header("authorization", "Bearer test-key"))
            .respond_with(ResponseTemplate::new(200).set_body_json(&response_body))
            .expect(1)
            .mount(&server)
            .await;

        let http = mock_http_client(&server.uri());
        let client = TtsClient::new(http);

        let list = client.list_voices().await.unwrap();
        assert_eq!(list.voices.len(), 2);
        assert_eq!(list.voices[0].voice_id, "eve");
        assert_eq!(list.voices[0].name, "Eve");
        assert_eq!(list.voices[0].language.as_deref(), Some("en-US"));
        assert_eq!(list.voices[1].voice_id, "rex");
        assert!(list.voices[1].language.is_none());
    }

    #[tokio::test]
    async fn get_voice_sends_get_and_deserializes() {
        let server = MockServer::start().await;
        let response_body = serde_json::json!({
            "voice_id": "sal",
            "name": "Sal",
            "language": "en-GB"
        });

        Mock::given(method("GET"))
            .and(path("/v1/tts/voices/sal"))
            .and(header("authorization", "Bearer test-key"))
            .respond_with(ResponseTemplate::new(200).set_body_json(&response_body))
            .expect(1)
            .mount(&server)
            .await;

        let http = mock_http_client(&server.uri());
        let client = TtsClient::new(http);

        let voice = client.get_voice("sal").await.unwrap();
        assert_eq!(voice.voice_id, "sal");
        assert_eq!(voice.name, "Sal");
        assert_eq!(voice.language.as_deref(), Some("en-GB"));
    }

    #[tokio::test]
    async fn generate_sends_post_and_returns_raw_bytes() {
        let server = MockServer::start().await;

        // Simulate raw audio bytes (not JSON).
        let fake_audio: Vec<u8> = vec![0xFF, 0xFB, 0x90, 0x00, 0x01, 0x02, 0x03];

        Mock::given(method("POST"))
            .and(path("/v1/tts"))
            .and(header("authorization", "Bearer test-key"))
            .and(body_partial_json(serde_json::json!({
                "text": "Hello, world!",
                "voice_id": "eve"
            })))
            .respond_with(ResponseTemplate::new(200).set_body_raw(fake_audio.clone(), "audio/mpeg"))
            .expect(1)
            .mount(&server)
            .await;

        let http = mock_http_client(&server.uri());
        let client = TtsClient::new(http);

        let request = TtsRequest::new("Hello, world!", VoiceId::Eve, "en").unwrap();
        let audio = client.generate(&request).await.unwrap();
        assert_eq!(audio, fake_audio);
    }

    #[tokio::test]
    async fn generate_with_all_options_sends_correct_body() {
        let server = MockServer::start().await;
        let fake_audio: Vec<u8> = vec![0x00, 0x01];

        Mock::given(method("POST"))
            .and(path("/v1/tts"))
            .and(body_partial_json(serde_json::json!({
                "text": "Hallo",
                "voice_id": "ara",
                "output_format": "wav",
                "sample_rate": 44100,
                "bit_rate": 256000,
                "language": "de-DE"
            })))
            .respond_with(ResponseTemplate::new(200).set_body_raw(fake_audio.clone(), "audio/wav"))
            .expect(1)
            .mount(&server)
            .await;

        let http = mock_http_client(&server.uri());
        let client = TtsClient::new(http);

        let request = TtsRequest::new("Hallo", VoiceId::Ara, "de-DE")
            .unwrap()
            .with_output_format(TtsOutputFormat::Wav)
            .with_sample_rate(44100)
            .with_bit_rate(256000);

        let audio = client.generate(&request).await.unwrap();
        assert_eq!(audio, fake_audio);
    }

    #[tokio::test]
    async fn generate_returns_error_on_4xx() {
        let server = MockServer::start().await;
        let error_body = serde_json::json!({
            "error": {
                "message": "Text too long",
                "type": "invalid_request_error"
            }
        });

        Mock::given(method("POST"))
            .and(path("/v1/tts"))
            .respond_with(ResponseTemplate::new(400).set_body_json(&error_body))
            .expect(1)
            .mount(&server)
            .await;

        let http = mock_http_client(&server.uri());
        let client = TtsClient::new(http);

        let request = TtsRequest::new("Test", VoiceId::Eve, "en").unwrap();
        let err = client.generate(&request).await.unwrap_err();
        let display = format!("{err}");
        assert!(display.contains("Text too long"));
    }

    #[tokio::test]
    async fn list_voices_returns_error_on_4xx() {
        let server = MockServer::start().await;
        let error_body = serde_json::json!({
            "error": {
                "message": "Unauthorized",
                "type": "authentication_error"
            }
        });

        Mock::given(method("GET"))
            .and(path("/v1/tts/voices"))
            .respond_with(ResponseTemplate::new(401).set_body_json(&error_body))
            .expect(1)
            .mount(&server)
            .await;

        let http = mock_http_client(&server.uri());
        let client = TtsClient::new(http);

        let err = client.list_voices().await.unwrap_err();
        let display = format!("{err}");
        assert!(display.contains("Unauthorized"));
    }
}
