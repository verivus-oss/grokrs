//! Wire types for the xAI Text-to-Speech API.
//!
//! These types map directly to the JSON request/response bodies of the
//! `/v1/tts`, `/v1/tts/voices`, and `/v1/tts/voices/{voice_id}` endpoints.
//! The TTS audio response is raw bytes (Content-Type: audio/*), not JSON,
//! so there is no response serde type for audio data.

use serde::{Deserialize, Serialize};

// ---------------------------------------------------------------------------
// Enums
// ---------------------------------------------------------------------------

/// Voice identifier for TTS generation.
///
/// Each variant serializes to the lowercase string expected by the xAI API.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum VoiceId {
    /// Eve voice.
    Eve,
    /// Ara voice.
    Ara,
    /// Rex voice.
    Rex,
    /// Sal voice.
    Sal,
    /// Leo voice.
    Leo,
}

impl std::fmt::Display for VoiceId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Eve => write!(f, "eve"),
            Self::Ara => write!(f, "ara"),
            Self::Rex => write!(f, "rex"),
            Self::Sal => write!(f, "sal"),
            Self::Leo => write!(f, "leo"),
        }
    }
}

impl std::str::FromStr for VoiceId {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_lowercase().as_str() {
            "eve" => Ok(Self::Eve),
            "ara" => Ok(Self::Ara),
            "rex" => Ok(Self::Rex),
            "sal" => Ok(Self::Sal),
            "leo" => Ok(Self::Leo),
            other => Err(format!(
                "unknown voice ID '{other}'; expected one of: eve, ara, rex, sal, leo"
            )),
        }
    }
}

/// Output audio format for TTS generation.
///
/// Each variant serializes to the exact string expected by the xAI API.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum TtsOutputFormat {
    /// MP3 audio format.
    Mp3,
    /// WAV audio format.
    Wav,
    /// Raw PCM audio format.
    Pcm,
    /// Mu-law encoded audio format.
    Mulaw,
    /// A-law encoded audio format.
    Alaw,
}

impl std::fmt::Display for TtsOutputFormat {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Mp3 => write!(f, "mp3"),
            Self::Wav => write!(f, "wav"),
            Self::Pcm => write!(f, "pcm"),
            Self::Mulaw => write!(f, "mulaw"),
            Self::Alaw => write!(f, "alaw"),
        }
    }
}

impl std::str::FromStr for TtsOutputFormat {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_lowercase().as_str() {
            "mp3" => Ok(Self::Mp3),
            "wav" => Ok(Self::Wav),
            "pcm" => Ok(Self::Pcm),
            "mulaw" => Ok(Self::Mulaw),
            "alaw" => Ok(Self::Alaw),
            other => Err(format!(
                "unknown output format '{other}'; expected one of: mp3, wav, pcm, mulaw, alaw"
            )),
        }
    }
}

// ---------------------------------------------------------------------------
// Request types
// ---------------------------------------------------------------------------

/// Maximum text length in characters for a TTS request.
pub const TTS_MAX_TEXT_LENGTH: usize = 15_000;

/// Request body for `POST /v1/tts`.
///
/// The `text` field supports up to 15,000 characters. Length validation is
/// enforced at the builder level, not the type level, so deserialization of
/// over-length text from an external source will succeed (the API server will
/// reject it).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TtsRequest {
    /// The text to synthesize into speech (max 15,000 characters).
    pub text: String,

    /// The voice to use for synthesis.
    pub voice_id: VoiceId,

    /// The audio output format. Defaults to MP3 on the server side if omitted.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub output_format: Option<TtsOutputFormat>,

    /// The audio sample rate in Hz.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sample_rate: Option<u32>,

    /// The audio bit rate in bits per second.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub bit_rate: Option<u32>,

    /// BCP-47 language tag (e.g., "en", "en-US", "de-DE"). Required by the
    /// xAI API.
    pub language: String,
}

impl TtsRequest {
    /// Create a new TTS request with required fields only.
    ///
    /// # Errors
    ///
    /// Returns an error if `text` exceeds [`TTS_MAX_TEXT_LENGTH`] characters.
    pub fn new(
        text: impl Into<String>,
        voice_id: VoiceId,
        language: impl Into<String>,
    ) -> Result<Self, TtsRequestError> {
        let text = text.into();
        if text.chars().count() > TTS_MAX_TEXT_LENGTH {
            return Err(TtsRequestError::TextTooLong {
                length: text.chars().count(),
                max: TTS_MAX_TEXT_LENGTH,
            });
        }
        Ok(Self {
            text,
            voice_id,
            output_format: None,
            sample_rate: None,
            bit_rate: None,
            language: language.into(),
        })
    }

    /// Set the output audio format.
    #[must_use]
    pub fn with_output_format(mut self, format: TtsOutputFormat) -> Self {
        self.output_format = Some(format);
        self
    }

    /// Set the sample rate in Hz.
    #[must_use]
    pub fn with_sample_rate(mut self, rate: u32) -> Self {
        self.sample_rate = Some(rate);
        self
    }

    /// Set the bit rate in bits per second.
    #[must_use]
    pub fn with_bit_rate(mut self, rate: u32) -> Self {
        self.bit_rate = Some(rate);
        self
    }
}

/// Errors that can occur when constructing a [`TtsRequest`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TtsRequestError {
    /// The input text exceeds the maximum allowed length.
    TextTooLong {
        /// The actual length of the text in characters.
        length: usize,
        /// The maximum allowed length.
        max: usize,
    },
}

impl std::fmt::Display for TtsRequestError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::TextTooLong { length, max } => {
                write!(
                    f,
                    "TTS text too long: {length} characters exceeds maximum of {max}"
                )
            }
        }
    }
}

impl std::error::Error for TtsRequestError {}

// ---------------------------------------------------------------------------
// Response types (JSON — for voice listing, not audio)
// ---------------------------------------------------------------------------

/// A single TTS voice descriptor returned by the voices endpoint.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TtsVoice {
    /// The voice identifier string (e.g., "eve").
    pub voice_id: String,

    /// Human-readable name of the voice.
    pub name: String,

    /// BCP-47 language tag for the voice, if available.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub language: Option<String>,
}

/// Response body from `GET /v1/tts/voices`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TtsVoiceList {
    /// The available TTS voices.
    pub voices: Vec<TtsVoice>,
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    // -- VoiceId serde --

    #[test]
    fn voice_id_serializes_eve() {
        let json = serde_json::to_string(&VoiceId::Eve).unwrap();
        assert_eq!(json, "\"eve\"");
    }

    #[test]
    fn voice_id_serializes_ara() {
        let json = serde_json::to_string(&VoiceId::Ara).unwrap();
        assert_eq!(json, "\"ara\"");
    }

    #[test]
    fn voice_id_serializes_rex() {
        let json = serde_json::to_string(&VoiceId::Rex).unwrap();
        assert_eq!(json, "\"rex\"");
    }

    #[test]
    fn voice_id_serializes_sal() {
        let json = serde_json::to_string(&VoiceId::Sal).unwrap();
        assert_eq!(json, "\"sal\"");
    }

    #[test]
    fn voice_id_serializes_leo() {
        let json = serde_json::to_string(&VoiceId::Leo).unwrap();
        assert_eq!(json, "\"leo\"");
    }

    #[test]
    fn voice_id_deserializes_from_lowercase() {
        let v: VoiceId = serde_json::from_str("\"rex\"").unwrap();
        assert_eq!(v, VoiceId::Rex);
    }

    #[test]
    fn voice_id_from_str_case_insensitive() {
        assert_eq!("Eve".parse::<VoiceId>().unwrap(), VoiceId::Eve);
        assert_eq!("ARA".parse::<VoiceId>().unwrap(), VoiceId::Ara);
        assert_eq!("leo".parse::<VoiceId>().unwrap(), VoiceId::Leo);
    }

    #[test]
    fn voice_id_from_str_rejects_unknown() {
        let err = "unknown".parse::<VoiceId>().unwrap_err();
        assert!(err.contains("unknown voice ID"));
    }

    #[test]
    fn voice_id_display() {
        assert_eq!(VoiceId::Eve.to_string(), "eve");
        assert_eq!(VoiceId::Sal.to_string(), "sal");
    }

    // -- TtsOutputFormat serde --

    #[test]
    fn output_format_serializes_mp3() {
        let json = serde_json::to_string(&TtsOutputFormat::Mp3).unwrap();
        assert_eq!(json, "\"mp3\"");
    }

    #[test]
    fn output_format_serializes_wav() {
        let json = serde_json::to_string(&TtsOutputFormat::Wav).unwrap();
        assert_eq!(json, "\"wav\"");
    }

    #[test]
    fn output_format_serializes_pcm() {
        let json = serde_json::to_string(&TtsOutputFormat::Pcm).unwrap();
        assert_eq!(json, "\"pcm\"");
    }

    #[test]
    fn output_format_serializes_mulaw() {
        let json = serde_json::to_string(&TtsOutputFormat::Mulaw).unwrap();
        assert_eq!(json, "\"mulaw\"");
    }

    #[test]
    fn output_format_serializes_alaw() {
        let json = serde_json::to_string(&TtsOutputFormat::Alaw).unwrap();
        assert_eq!(json, "\"alaw\"");
    }

    #[test]
    fn output_format_deserializes_from_string() {
        let fmt: TtsOutputFormat = serde_json::from_str("\"wav\"").unwrap();
        assert_eq!(fmt, TtsOutputFormat::Wav);
    }

    #[test]
    fn output_format_from_str_case_insensitive() {
        assert_eq!(
            "MP3".parse::<TtsOutputFormat>().unwrap(),
            TtsOutputFormat::Mp3
        );
        assert_eq!(
            "Wav".parse::<TtsOutputFormat>().unwrap(),
            TtsOutputFormat::Wav
        );
        assert_eq!(
            "MULAW".parse::<TtsOutputFormat>().unwrap(),
            TtsOutputFormat::Mulaw
        );
    }

    #[test]
    fn output_format_from_str_rejects_unknown() {
        let err = "ogg".parse::<TtsOutputFormat>().unwrap_err();
        assert!(err.contains("unknown output format"));
    }

    #[test]
    fn output_format_display() {
        assert_eq!(TtsOutputFormat::Mp3.to_string(), "mp3");
        assert_eq!(TtsOutputFormat::Alaw.to_string(), "alaw");
    }

    // -- TtsRequest serde round-trip --

    #[test]
    fn tts_request_round_trips_full() {
        let req = TtsRequest {
            text: "Hello, world!".to_string(),
            voice_id: VoiceId::Eve,
            output_format: Some(TtsOutputFormat::Mp3),
            sample_rate: Some(24000),
            bit_rate: Some(128000),
            language: "en-US".to_string(),
        };
        let json = serde_json::to_string(&req).unwrap();
        let deserialized: TtsRequest = serde_json::from_str(&json).unwrap();
        assert_eq!(req, deserialized);
    }

    #[test]
    fn tts_request_round_trips_minimal() {
        let req = TtsRequest {
            text: "Hi".to_string(),
            voice_id: VoiceId::Rex,
            output_format: None,
            sample_rate: None,
            bit_rate: None,
            language: "en".to_string(),
        };
        let json = serde_json::to_string(&req).unwrap();
        let deserialized: TtsRequest = serde_json::from_str(&json).unwrap();
        assert_eq!(req, deserialized);
    }

    #[test]
    fn tts_request_skips_none_fields() {
        let req = TtsRequest {
            text: "Test".to_string(),
            voice_id: VoiceId::Ara,
            output_format: None,
            sample_rate: None,
            bit_rate: None,
            language: "en".to_string(),
        };
        let json = serde_json::to_string(&req).unwrap();
        assert!(!json.contains("\"output_format\""));
        assert!(!json.contains("\"sample_rate\""));
        assert!(!json.contains("\"bit_rate\""));
        // language is now always serialized (required field)
        assert!(json.contains("\"language\""));
    }

    #[test]
    fn tts_request_serializes_voice_id_lowercase() {
        let req = TtsRequest::new("Hello", VoiceId::Leo, "en").unwrap();
        let json = serde_json::to_string(&req).unwrap();
        assert!(json.contains("\"voice_id\":\"leo\""));
    }

    // -- TtsRequest builder --

    #[test]
    fn tts_request_new_succeeds_within_limit() {
        let text = "a".repeat(TTS_MAX_TEXT_LENGTH);
        let req = TtsRequest::new(text.clone(), VoiceId::Eve, "en").unwrap();
        assert_eq!(req.text, text);
        assert_eq!(req.voice_id, VoiceId::Eve);
        assert_eq!(req.language, "en");
    }

    #[test]
    fn tts_request_new_rejects_over_limit() {
        let text = "a".repeat(TTS_MAX_TEXT_LENGTH + 1);
        let err = TtsRequest::new(text, VoiceId::Eve, "en").unwrap_err();
        assert_eq!(
            err,
            TtsRequestError::TextTooLong {
                length: TTS_MAX_TEXT_LENGTH + 1,
                max: TTS_MAX_TEXT_LENGTH,
            }
        );
    }

    #[test]
    fn tts_request_new_counts_characters_not_bytes() {
        // Multi-byte characters: each is 4 bytes but 1 character.
        // 15,000 emoji = 15,000 characters (within limit) but 60,000 bytes.
        let text = "\u{1F600}".repeat(TTS_MAX_TEXT_LENGTH);
        assert!(text.len() > TTS_MAX_TEXT_LENGTH); // bytes exceed limit
        let req = TtsRequest::new(text, VoiceId::Eve, "en");
        assert!(req.is_ok(), "should count characters, not bytes");

        // 15,001 emoji = over the character limit.
        let over = "\u{1F600}".repeat(TTS_MAX_TEXT_LENGTH + 1);
        let err = TtsRequest::new(over, VoiceId::Eve, "en").unwrap_err();
        assert_eq!(
            err,
            TtsRequestError::TextTooLong {
                length: TTS_MAX_TEXT_LENGTH + 1,
                max: TTS_MAX_TEXT_LENGTH,
            }
        );
    }

    #[test]
    fn tts_request_error_display() {
        let err = TtsRequestError::TextTooLong {
            length: 20000,
            max: 15000,
        };
        let msg = err.to_string();
        assert!(msg.contains("20000"));
        assert!(msg.contains("15000"));
    }

    #[test]
    fn tts_request_builder_chain() {
        let req = TtsRequest::new("Hello", VoiceId::Sal, "de-DE")
            .unwrap()
            .with_output_format(TtsOutputFormat::Wav)
            .with_sample_rate(44100)
            .with_bit_rate(256000);

        assert_eq!(req.output_format, Some(TtsOutputFormat::Wav));
        assert_eq!(req.sample_rate, Some(44100));
        assert_eq!(req.bit_rate, Some(256000));
        assert_eq!(req.language, "de-DE");
    }

    // -- TtsVoice / TtsVoiceList serde --

    #[test]
    fn tts_voice_round_trips() {
        let voice = TtsVoice {
            voice_id: "eve".to_string(),
            name: "Eve".to_string(),
            language: Some("en-US".to_string()),
        };
        let json = serde_json::to_string(&voice).unwrap();
        let deserialized: TtsVoice = serde_json::from_str(&json).unwrap();
        assert_eq!(voice, deserialized);
    }

    #[test]
    fn tts_voice_without_language_round_trips() {
        let voice = TtsVoice {
            voice_id: "rex".to_string(),
            name: "Rex".to_string(),
            language: None,
        };
        let json = serde_json::to_string(&voice).unwrap();
        let deserialized: TtsVoice = serde_json::from_str(&json).unwrap();
        assert_eq!(voice, deserialized);
        assert!(!json.contains("\"language\""));
    }

    #[test]
    fn tts_voice_list_round_trips() {
        let list = TtsVoiceList {
            voices: vec![
                TtsVoice {
                    voice_id: "eve".to_string(),
                    name: "Eve".to_string(),
                    language: Some("en-US".to_string()),
                },
                TtsVoice {
                    voice_id: "ara".to_string(),
                    name: "Ara".to_string(),
                    language: None,
                },
            ],
        };
        let json = serde_json::to_string(&list).unwrap();
        let deserialized: TtsVoiceList = serde_json::from_str(&json).unwrap();
        assert_eq!(list, deserialized);
    }

    #[test]
    fn tts_voice_list_empty_round_trips() {
        let list = TtsVoiceList { voices: vec![] };
        let json = serde_json::to_string(&list).unwrap();
        let deserialized: TtsVoiceList = serde_json::from_str(&json).unwrap();
        assert_eq!(list, deserialized);
    }

    #[test]
    fn tts_voice_deserializes_from_api_json() {
        let json = r#"{
            "voice_id": "sal",
            "name": "Sal",
            "language": "en-GB"
        }"#;
        let voice: TtsVoice = serde_json::from_str(json).unwrap();
        assert_eq!(voice.voice_id, "sal");
        assert_eq!(voice.name, "Sal");
        assert_eq!(voice.language.as_deref(), Some("en-GB"));
    }

    #[test]
    fn tts_voice_list_deserializes_from_api_json() {
        let json = r#"{
            "voices": [
                {"voice_id": "eve", "name": "Eve", "language": "en-US"},
                {"voice_id": "leo", "name": "Leo"}
            ]
        }"#;
        let list: TtsVoiceList = serde_json::from_str(json).unwrap();
        assert_eq!(list.voices.len(), 2);
        assert_eq!(list.voices[0].voice_id, "eve");
        assert_eq!(list.voices[1].voice_id, "leo");
        assert!(list.voices[1].language.is_none());
    }
}
