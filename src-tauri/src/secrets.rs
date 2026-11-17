use std::collections::BTreeMap;

#[derive(Default)]
pub struct ProcessEnv(BTreeMap<String, String>);

impl ProcessEnv {
    pub(crate) fn new(values: BTreeMap<String, String>) -> Self {
        Self(values)
    }

    pub fn contains(&self, name: &str) -> bool {
        self.0.contains_key(name)
    }

    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    pub(crate) fn redact(&self, text: &str) -> String {
        redact(text, &self.0)
    }

    pub(crate) fn process_envs(&self) -> serde_json::Value {
        serde_json::json!(self.0)
    }

    #[cfg(test)]
    pub(crate) fn get(&self, name: &str) -> Option<&str> {
        self.0.get(name).map(String::as_str)
    }
}

impl std::fmt::Debug for ProcessEnv {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_tuple("ProcessEnv").field(&self.0.keys().collect::<Vec<_>>()).finish()
    }
}

fn redact(text: &str, values: &BTreeMap<String, String>) -> String {
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

    macro_rules! assert_lacks {
        ($kind:path, $ty:ty) => {{
            trait Ambiguous<A> {
                const HELD: () = ();
            }
            impl<T: ?Sized> Ambiguous<()> for T {}
            impl<T: ?Sized + $kind> Ambiguous<u8> for T {}
            let () = <$ty as Ambiguous<_>>::HELD;
        }};
    }

    #[test]
    fn a_process_env_cannot_be_serialized_or_displayed() {
        assert_lacks!(serde::Serialize, ProcessEnv);
        assert_lacks!(std::fmt::Display, ProcessEnv);
    }

    #[test]
    fn a_process_env_debugs_as_its_names_and_never_its_values() {
        let env = ProcessEnv::new(BTreeMap::from([("GITHUB_TOKEN".into(), "ghp_hunter2".into())]));
        let printed = format!("{env:?}");
        assert!(printed.contains("GITHUB_TOKEN"), "{printed}");
        assert!(!printed.contains("hunter2"), "a value reached Debug: {printed}");
    }

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
    }
}
