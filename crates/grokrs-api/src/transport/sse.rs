use bytes::Bytes;
use futures::stream::Stream;
use pin_project_lite::pin_project;
use std::pin::Pin;
use std::task::{Context, Poll};

use crate::transport::error::TransportError;

/// The SSE stream terminator sent by the xAI API.
const DONE_MARKER: &str = "[DONE]";

/// Maximum length of a single SSE line in bytes.
///
/// This prevents unbounded memory allocation from a malicious or buggy server.
const MAX_LINE_LENGTH: usize = 1024 * 1024; // 1 MiB

pin_project! {
    /// A stream that parses Server-Sent Events (SSE) from a byte stream.
    ///
    /// This is a thin line-protocol parser, not a full EventSource
    /// implementation. It extracts `data:` lines and yields their content
    /// as strings. The stream terminates when `data: [DONE]` is received
    /// or the underlying byte stream ends.
    ///
    /// Memory-bounded: individual lines are limited to `MAX_LINE_LENGTH`.
    /// Dropping the stream drops the underlying byte stream, which releases
    /// the HTTP connection.
    pub struct SseStream<S> {
        #[pin]
        inner: S,
        buffer: String,
        done: bool,
    }
}

impl<S> SseStream<S>
where
    S: Stream<Item = Result<Bytes, reqwest::Error>>,
{
    /// Create a new SSE stream from a byte stream (typically from
    /// `reqwest::Response::bytes_stream()`).
    pub fn new(inner: S) -> Self {
        Self {
            inner,
            buffer: String::new(),
            done: false,
        }
    }
}

impl<S> Stream for SseStream<S>
where
    S: Stream<Item = Result<Bytes, reqwest::Error>>,
{
    type Item = Result<String, TransportError>;

    fn poll_next(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        let mut this = self.project();

        if *this.done {
            // Check for residual data after EOF — a partial line means
            // the connection was truncated mid-frame.
            let residual = this.buffer.trim();
            if !residual.is_empty() {
                let msg = format!(
                    "connection closed with incomplete SSE frame ({} bytes remaining)",
                    residual.len()
                );
                this.buffer.clear();
                return Poll::Ready(Some(Err(TransportError::Sse { message: msg })));
            }
            return Poll::Ready(None);
        }

        loop {
            // Try to extract a complete line from the buffer
            if let Some(line_result) = extract_data_line(this.buffer, this.done) {
                return Poll::Ready(Some(line_result));
            }

            if *this.done {
                return Poll::Ready(None);
            }

            // Need more data from the underlying stream
            match this.inner.as_mut().poll_next(cx) {
                Poll::Ready(Some(Ok(chunk))) => {
                    let text = String::from_utf8_lossy(&chunk);
                    this.buffer.push_str(&text);

                    // Guard against unbounded buffering
                    if this.buffer.len() > MAX_LINE_LENGTH {
                        *this.done = true;
                        return Poll::Ready(Some(Err(TransportError::Sse {
                            message: format!(
                                "SSE line exceeds maximum length of {MAX_LINE_LENGTH} bytes"
                            ),
                        })));
                    }
                }
                Poll::Ready(Some(Err(err))) => {
                    *this.done = true;
                    return Poll::Ready(Some(Err(TransportError::from(err))));
                }
                Poll::Ready(None) => {
                    *this.done = true;
                    // Try to extract any remaining data from the buffer
                    if let Some(line_result) = extract_data_line(this.buffer, this.done) {
                        return Poll::Ready(Some(line_result));
                    }
                    // Check for residual partial data — truncated mid-frame.
                    let residual = this.buffer.trim();
                    if !residual.is_empty() {
                        let msg = format!(
                            "connection closed with incomplete SSE frame ({} bytes remaining)",
                            residual.len()
                        );
                        this.buffer.clear();
                        return Poll::Ready(Some(Err(TransportError::Sse { message: msg })));
                    }
                    return Poll::Ready(None);
                }
                Poll::Pending => return Poll::Pending,
            }
        }
    }
}

/// Extract the next `data:` line from the buffer.
///
/// Returns `Some(Ok(data))` if a data line was found, `Some(Err(...))` on
/// parse error, or `None` if no complete line is available yet.
///
/// Modifies the buffer by removing consumed lines.
fn extract_data_line(
    buffer: &mut String,
    done: &mut bool,
) -> Option<Result<String, TransportError>> {
    loop {
        let newline_pos = buffer.find('\n')?;
        let line = buffer[..newline_pos].trim_end_matches('\r').to_string();
        buffer.drain(..=newline_pos);

        // Skip empty lines (SSE event boundaries)
        if line.is_empty() {
            continue;
        }

        // Skip comment lines
        if line.starts_with(':') {
            continue;
        }

        // Handle `data:` lines
        if let Some(data) = line.strip_prefix("data:") {
            let data = data.trim_start();

            // Check for stream terminator
            if data == DONE_MARKER {
                *done = true;
                return None;
            }

            return Some(Ok(data.to_string()));
        }

        // Skip other SSE fields (event:, id:, retry:) — we only care about data
        // This is intentional: we are a thin data-line parser, not a full
        // EventSource implementation.
    }
}

/// Parse SSE data lines from raw text for testing purposes.
///
/// Takes a complete SSE text payload and returns all `data:` lines as strings,
/// stopping at `data: [DONE]`.
pub fn parse_sse_lines(text: &str) -> Vec<Result<String, TransportError>> {
    let mut buffer = text.to_string();
    // Ensure buffer ends with newline for proper parsing
    if !buffer.ends_with('\n') {
        buffer.push('\n');
    }

    let mut results = Vec::new();
    let mut done = false;

    while !done {
        match extract_data_line(&mut buffer, &mut done) {
            Some(result) => results.push(result),
            None => break,
        }
    }

    results
}

#[cfg(test)]
mod tests {
    use super::*;
    use futures::StreamExt;
    use futures::stream;

    #[test]
    fn parse_sse_lines_extracts_data() {
        let text = "data: hello\ndata: world\n";
        let results = parse_sse_lines(text);
        assert_eq!(results.len(), 2);
        assert_eq!(results[0].as_ref().unwrap(), "hello");
        assert_eq!(results[1].as_ref().unwrap(), "world");
    }

    #[test]
    fn parse_sse_lines_terminates_on_done() {
        let text = "data: first\ndata: second\ndata: [DONE]\ndata: should_not_appear\n";
        let results = parse_sse_lines(text);
        assert_eq!(results.len(), 2);
        assert_eq!(results[0].as_ref().unwrap(), "first");
        assert_eq!(results[1].as_ref().unwrap(), "second");
    }

    #[test]
    fn parse_sse_lines_skips_empty_lines() {
        let text = "data: first\n\n\ndata: second\n";
        let results = parse_sse_lines(text);
        assert_eq!(results.len(), 2);
    }

    #[test]
    fn parse_sse_lines_skips_comments() {
        let text = ": this is a comment\ndata: actual_data\n";
        let results = parse_sse_lines(text);
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].as_ref().unwrap(), "actual_data");
    }

    #[test]
    fn parse_sse_lines_skips_non_data_fields() {
        let text = "event: message\ndata: payload\nid: 123\nretry: 5000\n";
        let results = parse_sse_lines(text);
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].as_ref().unwrap(), "payload");
    }

    #[test]
    fn parse_sse_lines_handles_carriage_return() {
        let text = "data: hello\r\ndata: world\r\n";
        let results = parse_sse_lines(text);
        assert_eq!(results.len(), 2);
        assert_eq!(results[0].as_ref().unwrap(), "hello");
        assert_eq!(results[1].as_ref().unwrap(), "world");
    }

    #[test]
    fn parse_sse_lines_trims_data_prefix_space() {
        // "data: value" and "data:value" should both work
        let text = "data: with_space\ndata:without_space\n";
        let results = parse_sse_lines(text);
        assert_eq!(results.len(), 2);
        assert_eq!(results[0].as_ref().unwrap(), "with_space");
        assert_eq!(results[1].as_ref().unwrap(), "without_space");
    }

    #[test]
    fn parse_sse_lines_empty_input() {
        let results = parse_sse_lines("");
        assert!(results.is_empty());
    }

    #[test]
    fn parse_sse_lines_json_payloads() {
        let text = r#"data: {"id":"chatcmpl-1","choices":[{"delta":{"content":"Hi"}}]}
data: {"id":"chatcmpl-1","choices":[{"delta":{"content":"!"}}]}
data: [DONE]
"#;
        let results = parse_sse_lines(text);
        assert_eq!(results.len(), 2);
        // Verify the JSON is intact
        let first: serde_json::Value = serde_json::from_str(results[0].as_ref().unwrap()).unwrap();
        assert_eq!(first["id"], "chatcmpl-1");
    }

    #[tokio::test]
    async fn sse_stream_yields_data_lines() {
        let chunks = vec![
            Ok(Bytes::from("data: first\n")),
            Ok(Bytes::from("data: second\n")),
            Ok(Bytes::from("data: [DONE]\n")),
        ];
        let byte_stream = stream::iter(chunks);
        let mut sse = SseStream::new(byte_stream);

        let first = sse.next().await.unwrap().unwrap();
        assert_eq!(first, "first");
        let second = sse.next().await.unwrap().unwrap();
        assert_eq!(second, "second");
        // After [DONE], stream should end
        assert!(sse.next().await.is_none());
    }

    #[tokio::test]
    async fn sse_stream_handles_split_chunks() {
        // Data split across multiple byte chunks
        let chunks = vec![
            Ok(Bytes::from("dat")),
            Ok(Bytes::from("a: hel")),
            Ok(Bytes::from("lo\ndata: [DONE]\n")),
        ];
        let byte_stream = stream::iter(chunks);
        let mut sse = SseStream::new(byte_stream);

        let first = sse.next().await.unwrap().unwrap();
        assert_eq!(first, "hello");
        assert!(sse.next().await.is_none());
    }

    #[tokio::test]
    async fn sse_stream_terminates_on_stream_end() {
        let chunks = vec![
            Ok(Bytes::from("data: only\n")),
            // Stream ends without [DONE]
        ];
        let byte_stream = stream::iter(chunks);
        let mut sse = SseStream::new(byte_stream);

        let first = sse.next().await.unwrap().unwrap();
        assert_eq!(first, "only");
        assert!(sse.next().await.is_none());
    }

    #[tokio::test]
    async fn sse_stream_errors_on_truncated_frame_at_eof() {
        // Complete line followed by a partial line with no trailing newline.
        let chunks = vec![Ok(Bytes::from("data: ok\ndata: trunc"))];
        let byte_stream = stream::iter(chunks);
        let mut sse = SseStream::new(byte_stream);

        // First poll should yield the complete line.
        let first = sse.next().await.unwrap().unwrap();
        assert_eq!(first, "ok");

        // Second poll should produce a TransportError::Sse for the residual.
        let err = sse.next().await.unwrap().unwrap_err();
        match err {
            TransportError::Sse { message } => {
                assert!(
                    message.contains("incomplete SSE frame"),
                    "unexpected message: {message}"
                );
            }
            other => panic!("expected TransportError::Sse, got: {other:?}"),
        }

        // After error is emitted and buffer cleared, stream ends.
        assert!(sse.next().await.is_none());
    }

    #[tokio::test]
    async fn sse_stream_errors_on_pure_truncation() {
        // No newline at all — entire payload is a partial line.
        let chunks = vec![Ok(Bytes::from("data: trunc"))];
        let byte_stream = stream::iter(chunks);
        let mut sse = SseStream::new(byte_stream);

        let err = sse.next().await.unwrap().unwrap_err();
        match err {
            TransportError::Sse { message } => {
                assert!(
                    message.contains("incomplete SSE frame"),
                    "unexpected message: {message}"
                );
            }
            other => panic!("expected TransportError::Sse, got: {other:?}"),
        }

        assert!(sse.next().await.is_none());
    }

    #[tokio::test]
    async fn sse_stream_clean_eof() {
        // Complete line with trailing newline — no residual.
        let chunks = vec![Ok(Bytes::from("data: ok\n"))];
        let byte_stream = stream::iter(chunks);
        let mut sse = SseStream::new(byte_stream);

        let first = sse.next().await.unwrap().unwrap();
        assert_eq!(first, "ok");
        assert!(sse.next().await.is_none());
    }

    #[tokio::test]
    async fn sse_stream_whitespace_only_residual_is_clean() {
        // Trailing whitespace (empty lines) after data — not a truncation.
        let chunks = vec![Ok(Bytes::from("data: ok\n\n"))];
        let byte_stream = stream::iter(chunks);
        let mut sse = SseStream::new(byte_stream);

        let first = sse.next().await.unwrap().unwrap();
        assert_eq!(first, "ok");
        assert!(sse.next().await.is_none());
    }
}
