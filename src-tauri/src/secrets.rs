//! Process credentials. Values are never serialized; grants are resolved by the store.
use std::collections::BTreeMap;

#[derive(Default)]
pub struct Environment {
    pub names: Vec<String>,
    pub values: BTreeMap<String, String>,
}

impl Environment {
    pub fn apply(&self, command: &mut tokio::process::Command) {
        for name in &self.names {
            command.env_remove(name);
        }
        command.envs(&self.values);
    }

    pub fn description(&self) -> String {
        if self.values.is_empty() {
            return String::new();
        }
        format!("\nGuaca provides these secret environment variables: {}. Use them by name; never print their values, copy them into files, or include them in messages.\n",
            self.values.keys().map(|name| format!("${name}")).collect::<Vec<_>>().join(", "))
    }
}

/// Exact values and their common serialized forms. This limits accidental disclosure;
/// an authorized program can still transform or transmit credentials it holds.
pub fn redact(text: &str, values: &BTreeMap<String, String>) -> String {
    let (bytes, _) = Redactor::new(values).take(text.as_bytes(), true);
    String::from_utf8_lossy(&bytes).into_owned()
}

pub(crate) struct Redactor {
    patterns: Vec<Vec<u8>>,
    longest: usize,
}

impl Redactor {
    pub(crate) fn new(values: &BTreeMap<String, String>) -> Self {
        use base64::Engine;
        let mut patterns = Vec::new();
        for value in values.values().filter(|value| !value.is_empty()) {
            patterns.push(value.as_bytes().to_vec());
            if let Ok(encoded) = serde_json::to_string(value) {
                patterns.push(encoded.as_bytes()[1..encoded.len() - 1].to_vec());
            }
            patterns.push(
                percent_encoding::utf8_percent_encode(value, percent_encoding::NON_ALPHANUMERIC)
                    .to_string()
                    .into_bytes(),
            );
            patterns.push(base64::engine::general_purpose::STANDARD.encode(value).into_bytes());
        }
        patterns.sort_by(|a, b| b.len().cmp(&a.len()).then(a.cmp(b)));
        patterns.dedup();
        let longest = patterns.first().map(Vec::len).unwrap_or(1);
        Self { patterns, longest }
    }

    /// Keep incomplete matches for the next read. Scrub before any output cap.
    pub(crate) fn take(&self, input: &[u8], finished: bool) -> (Vec<u8>, usize) {
        let limit =
            if finished { input.len() } else { input.len().saturating_sub(self.longest - 1) };
        let mut output = Vec::new();
        let mut at = 0;
        while at < limit {
            if let Some(pattern) =
                self.patterns.iter().find(|pattern| input[at..].starts_with(pattern))
            {
                output.extend_from_slice(b"[REDACTED]");
                at += pattern.len();
            } else {
                output.push(input[at]);
                at += 1;
            }
        }
        (output, at)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn chunk_boundaries_and_overlapping_values_do_not_disclose_bytes() {
        let values = BTreeMap::from([
            ("LONG".into(), "秘密-token".into()),
            ("SHORT".into(), "token".into()),
        ]);
        for width in 1..20 {
            let scrubber = Redactor::new(&values);
            let mut pending = Vec::new();
            let mut output = Vec::new();
            for chunk in "before 秘密-token after token".as_bytes().chunks(width) {
                pending.extend_from_slice(chunk);
                let (safe, consumed) = scrubber.take(&pending, false);
                output.extend(safe);
                pending.drain(..consumed);
            }
            output.extend(scrubber.take(&pending, true).0);
            assert_eq!(String::from_utf8(output).unwrap(), "before [REDACTED] after [REDACTED]");
        }
    }

    #[test]
    fn values_and_common_encodings_are_scrubbed() {
        let values = BTreeMap::from([("TOKEN".into(), "abc+/123".into())]);
        assert_eq!(
            redact("abc+/123 abc%2B%2F123 YWJjKy8xMjM=", &values),
            "[REDACTED] [REDACTED] [REDACTED]"
        );
        assert!(!Environment { names: vec![], values }.description().contains("abc"));
    }
}
