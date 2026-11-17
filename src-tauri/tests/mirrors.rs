use std::collections::{BTreeMap, BTreeSet};
use std::fmt::Debug;
use std::path::PathBuf;

use serde::de::DeserializeOwned;
use serde::Deserialize;
use serde_json::{json, Value};

use guac_lib::domain::ids::AgentId;
use guac_lib::domain::plugin::PluginAccess;
use guac_lib::runtime::guard::GuardLimits;

type Check = fn(&Value);

const CHECKS: &[(&str, Check)] = &[
    ("limits", limits),
    ("plugin-access", plugin_access),
];

fn mirrors() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../src/lib/mirrors")
}

fn vectors(name: &str) -> Value {
    let path = mirrors().join(format!("{name}.json"));
    let text = std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
    serde_json::from_str(&text).unwrap_or_else(|e| panic!("{}: {e}", path.display()))
}

fn parsed<T: DeserializeOwned>(value: &Value) -> T {
    serde_json::from_value(value.clone()).unwrap_or_else(|e| panic!("{value}: {e}"))
}

fn each<I: DeserializeOwned, E: DeserializeOwned + PartialEq + Debug>(
    file: &Value,
    actual: impl Fn(I) -> E,
) {
    for case in file.as_array().expect("a vector file is a list of cases") {
        let expected: E = parsed(&case["expected"]);
        assert_eq!(actual(parsed(&case["input"])), expected, "input {}", case["input"]);
    }
}

#[test]
fn every_vector_file_has_a_check_here_and_every_check_a_file() {
    let files: BTreeSet<String> = std::fs::read_dir(mirrors())
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .filter(|path| path.extension().is_some_and(|ext| ext == "json"))
        .map(|path| path.file_stem().unwrap().to_string_lossy().into_owned())
        .collect();
    let checks: BTreeSet<String> = CHECKS.iter().map(|(name, _)| name.to_string()).collect();
    assert_eq!(files, checks);
}

#[test]
fn every_vector_holds_in_rust() {
    for (name, check) in CHECKS {
        check(&vectors(name));
    }
}

#[derive(Deserialize)]
struct Access {
    access: PluginAccess,
    agent: AgentId,
}

fn plugin_access(file: &Value) {
    each(file, |case: Access| case.access.allows(case.agent));
}

fn limits(file: &Value) {
    let low = GuardLimits {
        max_hops: 0,
        max_steps_per_run: 0,
        max_fanout_per_call: 0,
        max_sends_per_pair: 0,
        max_tool_rounds: 0,
    }
    .sanitized();
    let high = GuardLimits {
        max_hops: u16::MAX,
        max_steps_per_run: u32::MAX,
        max_fanout_per_call: usize::MAX,
        max_sends_per_pair: u32::MAX,
        max_tool_rounds: u16::MAX,
    }
    .sanitized();
    let (low, high) = (serde_json::to_value(low).unwrap(), serde_json::to_value(high).unwrap());
    let ours: BTreeMap<String, Value> = low
        .as_object()
        .unwrap()
        .keys()
        .map(|key| (key.clone(), json!({ "min": low[key], "max": high[key] })))
        .collect();
    let theirs: BTreeMap<String, Value> = file
        .as_array()
        .unwrap()
        .iter()
        .map(|case| (parsed(&case["input"]), case["expected"].clone()))
        .collect();
    assert_eq!(ours, theirs);
}
