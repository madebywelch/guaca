#[derive(Debug, Default)]
pub struct SseDecoder {
    buffer: String,
}

impl SseDecoder {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn push(&mut self, chunk: &str) -> Vec<String> {
        self.buffer.push_str(chunk);
        let mut out = Vec::new();

        let Some(last_newline) = self.buffer.rfind('\n') else {
            return out;
        };

        let complete: String = self.buffer.drain(..=last_newline).collect();
        for raw_line in complete.split('\n') {
            let line = raw_line.strip_suffix('\r').unwrap_or(raw_line);

            if line.is_empty() {
                continue;
            }
            if line.starts_with(':') {
                continue;
            }
            if let Some(payload) = line.strip_prefix("data:") {
                out.push(payload.trim_start().to_string());
            }
        }

        out
    }

    pub fn remainder(&self) -> &str {
        &self.buffer
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decodes_one_event_per_line() {
        let mut d = SseDecoder::new();
        assert_eq!(d.push("data: {\"a\":1}\n\n"), vec!["{\"a\":1}"]);
    }

    #[test]
    fn buffers_a_payload_split_across_chunks() {
        let mut d = SseDecoder::new();
        assert!(d.push("data: {\"choices\":[{\"delta\":{\"cont").is_empty());
        assert!(d.push("ent\":\"hel").is_empty());
        assert_eq!(
            d.push("lo\"}}]}\n\n"),
            vec!["{\"choices\":[{\"delta\":{\"content\":\"hello\"}}]}"]
        );
    }

    #[test]
    fn handles_several_events_in_one_chunk() {
        let mut d = SseDecoder::new();
        assert_eq!(
            d.push("data: one\n\ndata: two\n\ndata: three\n\n"),
            vec!["one", "two", "three"]
        );
    }

    #[test]
    fn skips_comments_and_keepalives() {
        let mut d = SseDecoder::new();
        assert_eq!(
            d.push(": OPENROUTER PROCESSING\n\ndata: real\n\n"),
            vec!["real"],
            "a keepalive comment must not reach the JSON parser"
        );
    }

    #[test]
    fn skips_non_data_fields() {
        let mut d = SseDecoder::new();
        assert_eq!(d.push("event: message\nid: 7\nretry: 100\ndata: payload\n\n"), vec!["payload"]);
    }

    #[test]
    fn tolerates_crlf_line_endings() {
        let mut d = SseDecoder::new();
        assert_eq!(d.push("data: {\"a\":1}\r\n\r\n"), vec!["{\"a\":1}"]);
    }

    #[test]
    fn tolerates_a_missing_space_after_the_colon() {
        let mut d = SseDecoder::new();
        assert_eq!(d.push("data:{\"a\":1}\n\n"), vec!["{\"a\":1}"]);
    }

    #[test]
    fn preserves_json_containing_newline_escapes() {
        let mut d = SseDecoder::new();
        let payload = r#"{"content":"line one\nline two"}"#;
        assert_eq!(d.push(&format!("data: {payload}\n\n")), vec![payload]);
    }

    #[test]
    fn reports_an_unterminated_tail() {
        let mut d = SseDecoder::new();
        d.push("data: complete\n\ndata: truncat");
        assert_eq!(d.remainder(), "data: truncat");
    }

    #[test]
    fn a_byte_at_a_time_stream_decodes_identically() {
        let full = "data: alpha\n\ndata: beta\n\ndata: [DONE]\n\n";
        let mut one_shot = SseDecoder::new();
        let expected = one_shot.push(full);

        let mut drip = SseDecoder::new();
        let mut got = Vec::new();
        for ch in full.chars() {
            got.extend(drip.push(&ch.to_string()));
        }
        assert_eq!(got, expected);
        assert_eq!(got, vec!["alpha", "beta", "[DONE]"]);
    }
}
