//! Typed stream parsers for Chat Completions and Responses API SSE streams.
//!
//! These parsers consume the raw `String` items produced by
//! [`crate::transport::sse::SseStream`] and deserialize them into typed
//! event structs. They are memory-bounded — no internal buffering beyond
//! the current event — and cancellation-safe (dropping the returned stream
//! drops the underlying SSE stream, releasing the HTTP connection).

use std::pin::Pin;

use futures::stream::{Stream, StreamExt};

use crate::transport::error::TransportError;
use crate::types::stream::{ChatStreamChunk, ResponseStreamEvent, StreamError};

/// Parse a raw SSE data stream into typed [`ChatStreamChunk`] values.
///
/// Each `Ok(String)` item from `raw_stream` is expected to be a single
/// JSON-encoded `ChatStreamChunk` (i.e. one `data:` line from the SSE
/// stream, with the `data:` prefix and `[DONE]` sentinel already stripped
/// by the SSE layer).
///
/// # Error handling
///
/// - JSON parse failures become [`StreamError::Parse`].
/// - Transport errors from the underlying stream are wrapped in
///   [`StreamError::Transport`].
/// - If the underlying stream ends unexpectedly (without `[DONE]`), the
///   returned stream simply ends — no synthetic error is emitted, because
///   the SSE layer already handles `[DONE]` termination and the caller
///   can detect early termination by the absence of a final usage chunk.
///
/// # Memory safety
///
/// No internal buffering: each raw line is deserialized and yielded
/// immediately. Dropping the returned stream drops `raw_stream`.
pub fn parse_chat_stream<S>(
    raw_stream: S,
) -> Pin<Box<dyn Stream<Item = Result<ChatStreamChunk, StreamError>> + Send>>
where
    S: Stream<Item = Result<String, TransportError>> + Send + 'static,
{
    Box::pin(raw_stream.map(|item| match item {
        Ok(data) => {
            serde_json::from_str::<ChatStreamChunk>(&data).map_err(|e| StreamError::Parse {
                message: format!("failed to deserialize ChatStreamChunk: {e}"),
            })
        }
        Err(transport_err) => Err(StreamError::Transport(transport_err)),
    }))
}

/// Parse a raw SSE data stream into typed [`ResponseStreamEvent`] values.
///
/// Each `Ok(String)` item from `raw_stream` is expected to be a single
/// JSON-encoded event object with a `"type"` field that determines which
/// [`ResponseStreamEvent`] variant to deserialize into.
///
/// # Error handling
///
/// Same semantics as [`parse_chat_stream`]: parse failures become
/// [`StreamError::Parse`], transport errors are wrapped, and early stream
/// termination is surfaced as the stream ending.
///
/// # Memory safety
///
/// No internal buffering: each raw line is deserialized and yielded
/// immediately. Dropping the returned stream drops `raw_stream`.
pub fn parse_response_stream<S>(
    raw_stream: S,
) -> Pin<Box<dyn Stream<Item = Result<ResponseStreamEvent, StreamError>> + Send>>
where
    S: Stream<Item = Result<String, TransportError>> + Send + 'static,
{
    Box::pin(raw_stream.map(|item| match item {
        Ok(data) => {
            serde_json::from_str::<ResponseStreamEvent>(&data).map_err(|e| StreamError::Parse {
                message: format!("failed to deserialize ResponseStreamEvent: {e}"),
            })
        }
        Err(transport_err) => Err(StreamError::Transport(transport_err)),
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use futures::stream;

    // -- parse_chat_stream tests --

    #[tokio::test]
    async fn parse_chat_stream_yields_typed_chunks() {
        let lines: Vec<Result<String, TransportError>> = vec![
            Ok(r#"{"id":"chatcmpl-1","choices":[{"index":0,"delta":{"content":"Hi"}}]}"#.into()),
            Ok(r#"{"id":"chatcmpl-1","choices":[{"index":0,"delta":{"content":"!"},"finish_reason":"stop"}]}"#.into()),
        ];
        let raw = stream::iter(lines);
        let mut parsed = parse_chat_stream(raw);

        let first = parsed.next().await.unwrap().unwrap();
        assert_eq!(first.id, "chatcmpl-1");
        assert_eq!(first.choices[0].delta.content.as_deref(), Some("Hi"));
        assert!(first.choices[0].finish_reason.is_none());

        let second = parsed.next().await.unwrap().unwrap();
        assert_eq!(
            second.choices[0].finish_reason,
            Some(crate::types::stream::FinishReason::Stop)
        );
        assert_eq!(second.choices[0].delta.content.as_deref(), Some("!"));

        // Stream ends
        assert!(parsed.next().await.is_none());
    }

    #[tokio::test]
    async fn parse_chat_stream_parse_error_on_malformed_json() {
        let lines: Vec<Result<String, TransportError>> = vec![Ok("this is not json".into())];
        let raw = stream::iter(lines);
        let mut parsed = parse_chat_stream(raw);

        let result = parsed.next().await.unwrap();
        assert!(result.is_err());
        match result.unwrap_err() {
            StreamError::Parse { message } => {
                assert!(message.contains("ChatStreamChunk"));
            }
            other => panic!("expected Parse error, got: {other}"),
        }
    }

    #[tokio::test]
    async fn parse_chat_stream_transport_error_propagated() {
        let lines: Vec<Result<String, TransportError>> = vec![Err(TransportError::Timeout)];
        let raw = stream::iter(lines);
        let mut parsed = parse_chat_stream(raw);

        let result = parsed.next().await.unwrap();
        assert!(result.is_err());
        match result.unwrap_err() {
            StreamError::Transport(TransportError::Timeout) => {}
            other => panic!("expected Transport(Timeout), got: {other}"),
        }
    }

    #[tokio::test]
    async fn parse_chat_stream_empty_stream() {
        let lines: Vec<Result<String, TransportError>> = vec![];
        let raw = stream::iter(lines);
        let mut parsed = parse_chat_stream(raw);
        assert!(parsed.next().await.is_none());
    }

    #[tokio::test]
    async fn parse_chat_stream_done_already_handled_by_sse() {
        // The SSE layer strips [DONE], so the typed parser should never see it.
        // If the raw stream is empty (because SSE consumed [DONE]), we get nothing.
        let lines: Vec<Result<String, TransportError>> = vec![];
        let raw = stream::iter(lines);
        let mut parsed = parse_chat_stream(raw);
        assert!(parsed.next().await.is_none());
    }

    #[tokio::test]
    async fn parse_chat_stream_with_usage_chunk() {
        let json = r#"{"id":"chatcmpl-1","choices":[],"usage":{"prompt_tokens":10,"completion_tokens":5,"total_tokens":15}}"#;
        let lines: Vec<Result<String, TransportError>> = vec![Ok(json.into())];
        let raw = stream::iter(lines);
        let mut parsed = parse_chat_stream(raw);

        let chunk = parsed.next().await.unwrap().unwrap();
        let usage = chunk.usage.unwrap();
        assert_eq!(usage.prompt_tokens, 10);
        assert_eq!(usage.completion_tokens, 5);
    }

    // -- parse_response_stream tests --

    #[tokio::test]
    async fn parse_response_stream_yields_typed_events() {
        let lines: Vec<Result<String, TransportError>> = vec![
            Ok(r#"{"type":"response.created","response":{"id":"resp_1","status":"in_progress"}}"#.into()),
            Ok(r#"{"type":"response.content_part.delta","output_index":0,"content_index":0,"delta":{"type":"text","text":"Hi"}}"#.into()),
            Ok(r#"{"type":"response.completed","response":{"id":"resp_1","status":"completed"}}"#.into()),
        ];
        let raw = stream::iter(lines);
        let mut parsed = parse_response_stream(raw);

        let first = parsed.next().await.unwrap().unwrap();
        assert!(matches!(first, ResponseStreamEvent::ResponseCreated { .. }));

        let second = parsed.next().await.unwrap().unwrap();
        match &second {
            ResponseStreamEvent::ContentDelta { delta, .. } => {
                assert_eq!(delta.text.as_deref(), Some("Hi"));
            }
            other => panic!("expected ContentDelta, got: {other:?}"),
        }

        let third = parsed.next().await.unwrap().unwrap();
        assert!(matches!(
            third,
            ResponseStreamEvent::ResponseCompleted { .. }
        ));

        assert!(parsed.next().await.is_none());
    }

    #[tokio::test]
    async fn parse_response_stream_parse_error() {
        let lines: Vec<Result<String, TransportError>> = vec![Ok("{bad json}".into())];
        let raw = stream::iter(lines);
        let mut parsed = parse_response_stream(raw);

        let result = parsed.next().await.unwrap();
        assert!(result.is_err());
        match result.unwrap_err() {
            StreamError::Parse { message } => {
                assert!(message.contains("ResponseStreamEvent"));
            }
            other => panic!("expected Parse error, got: {other}"),
        }
    }

    #[tokio::test]
    async fn parse_response_stream_empty() {
        let lines: Vec<Result<String, TransportError>> = vec![];
        let raw = stream::iter(lines);
        let mut parsed = parse_response_stream(raw);
        assert!(parsed.next().await.is_none());
    }

    #[tokio::test]
    async fn parse_response_stream_transport_error() {
        let lines: Vec<Result<String, TransportError>> = vec![
            Ok(r#"{"type":"response.created","response":{"id":"resp_1"}}"#.into()),
            Err(TransportError::Sse {
                message: "connection reset".into(),
            }),
        ];
        let raw = stream::iter(lines);
        let mut parsed = parse_response_stream(raw);

        // First event succeeds
        let first = parsed.next().await.unwrap();
        assert!(first.is_ok());

        // Second is a transport error
        let second = parsed.next().await.unwrap();
        assert!(second.is_err());
        match second.unwrap_err() {
            StreamError::Transport(TransportError::Sse { message }) => {
                assert!(message.contains("connection reset"));
            }
            other => panic!("expected Transport(Sse), got: {other}"),
        }
    }

    #[tokio::test]
    async fn parse_response_stream_function_call_events() {
        let lines: Vec<Result<String, TransportError>> = vec![
            Ok(r#"{"type":"response.function_call_arguments.delta","output_index":0,"delta":"{\"q\":"}"#.into()),
            Ok(r#"{"type":"response.function_call_arguments.done","output_index":0,"arguments":"{\"q\":\"rust\"}"}"#.into()),
        ];
        let raw = stream::iter(lines);
        let mut parsed = parse_response_stream(raw);

        let delta = parsed.next().await.unwrap().unwrap();
        match &delta {
            ResponseStreamEvent::FunctionCallArgumentsDelta {
                output_index,
                delta,
                ..
            } => {
                assert_eq!(*output_index, 0);
                assert_eq!(delta, "{\"q\":");
            }
            other => panic!("expected FunctionCallArgumentsDelta, got: {other:?}"),
        }

        let done = parsed.next().await.unwrap().unwrap();
        match &done {
            ResponseStreamEvent::FunctionCallArgumentsDone { arguments, .. } => {
                assert_eq!(arguments, "{\"q\":\"rust\"}");
            }
            other => panic!("expected FunctionCallArgumentsDone, got: {other:?}"),
        }
    }
}
