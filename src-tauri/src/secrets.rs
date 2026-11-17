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

    pub(crate) fn quote(&self, text: &str, chars: usize) -> String {
        self.redact(text).chars().take(chars).collect()
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
    let input = text.as_bytes();
    let mut output = Vec::new();
    let mut at = 0;
    while at < input.len() {
        if let Some(pattern) = patterns.iter().find(|pattern| input[at..].starts_with(pattern)) {
            output.extend_from_slice(b"[REDACTED]");
            at += pattern.len();
        } else {
            output.push(input[at]);
            at += 1;
        }
    }
    String::from_utf8_lossy(&output).into_owned()
}

#[cfg(test)]
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

#[cfg(test)]
pub(crate) use assert_lacks;

#[cfg(test)]
mod tests {
    use super::*;

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
    fn overlapping_values_are_scrubbed_whole() {
        let values = BTreeMap::from([
            ("LONG".into(), "秘密-token".into()),
            ("SHORT".into(), "token".into()),
        ]);
        assert_eq!(
            redact("before 秘密-token after token", &values),
            "before [REDACTED] after [REDACTED]"
        );
    }

    #[test]
    fn a_quote_cut_across_a_secret_keeps_none_of_it() {
        let env = ProcessEnv::new(BTreeMap::from([("GITHUB_TOKEN".into(), "ghp_hunter2".into())]));
        let text = format!("{}ghp_hunter2 rejected", "x".repeat(195));
        let quoted = env.quote(&text, 200);
        assert_eq!(quoted.chars().count(), 200);
        assert!(!quoted.contains("ghp_h"), "a cut secret reached the quote: {quoted}");
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
