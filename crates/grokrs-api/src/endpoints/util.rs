/// Percent-encode a single path segment so that characters like `/`, `?`, `#`,
/// and spaces do not break the URL structure.
///
/// Uses a minimal manual implementation covering the characters that would
/// corrupt a URL path segment. Unreserved characters (RFC 3986 section 2.3)
/// plus sub-delimiters safe in a path segment are passed through verbatim.
pub(crate) fn encode_path_segment(segment: &str) -> String {
    let mut encoded = String::with_capacity(segment.len());
    for byte in segment.bytes() {
        match byte {
            // Unreserved characters (RFC 3986 section 2.3) plus sub-delimiters
            // that are safe in a path segment. We keep alphanumerics, `-`, `.`,
            // `_`, `~`, and common model-id punctuation like `:` unencoded.
            b'A'..=b'Z'
            | b'a'..=b'z'
            | b'0'..=b'9'
            | b'-'
            | b'.'
            | b'_'
            | b'~'
            | b':'
            | b'@'
            | b'!' => {
                encoded.push(byte as char);
            }
            _ => {
                encoded.push('%');
                encoded.push(char::from(HEX_UPPER[(byte >> 4) as usize]));
                encoded.push(char::from(HEX_UPPER[(byte & 0x0F) as usize]));
            }
        }
    }
    encoded
}

/// Upper-case hex digits for percent-encoding.
const HEX_UPPER: [u8; 16] = *b"0123456789ABCDEF";

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn preserves_typical_ids() {
        assert_eq!(encode_path_segment("grok-4"), "grok-4");
        assert_eq!(encode_path_segment("resp_abc123"), "resp_abc123");
        assert_eq!(encode_path_segment("file-abc-123_XYZ"), "file-abc-123_XYZ");
        assert_eq!(encode_path_segment("req_done"), "req_done");
    }

    #[test]
    fn encodes_slash() {
        assert_eq!(encode_path_segment("org/grok-4"), "org%2Fgrok-4");
    }

    #[test]
    fn encodes_space() {
        assert_eq!(encode_path_segment("my model"), "my%20model");
    }

    #[test]
    fn encodes_query_chars() {
        assert_eq!(encode_path_segment("id?v=1"), "id%3Fv%3D1");
    }

    #[test]
    fn encodes_hash() {
        assert_eq!(encode_path_segment("id#frag"), "id%23frag");
    }

    #[test]
    fn handles_empty_string() {
        assert_eq!(encode_path_segment(""), "");
    }

    #[test]
    fn handles_unicode() {
        let encoded = encode_path_segment("id-ü");
        assert!(encoded.starts_with("id-"));
        assert!(encoded.contains('%'));
        assert!(!encoded.contains('ü'));
    }

    #[test]
    fn preserves_tilde_underscore_colon() {
        assert_eq!(encode_path_segment("a_b~c:d"), "a_b~c:d");
    }
}
