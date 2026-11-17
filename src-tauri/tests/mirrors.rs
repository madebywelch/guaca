use std::collections::{BTreeMap, BTreeSet};
use std::fmt::Debug;
use std::path::PathBuf;

use serde::de::DeserializeOwned;
use serde::Deserialize;
use serde_json::{json, Value};

use guac_lib::config::AppConfig;
use guac_lib::db::Store;
use guac_lib::domain::agent::{CleanDraft, MAX_NAME_LEN};
use guac_lib::domain::connector::CleanConnector;
use guac_lib::domain::envelope::Envelope;
use guac_lib::domain::ids::AgentId;
use guac_lib::domain::plugin::{PluginAccess, PluginKind, STDIO};
use guac_lib::domain::routine::{human_gap, Cadence, EventTrigger, Trigger};
use guac_lib::llm::modality::Modalities;
use guac_lib::llm::openrouter::{LlmClient, ToolCall};
use guac_lib::llm::tools::{self, Surfaces, ToolInvocation};
use guac_lib::runtime::events::RecordingSink;
use guac_lib::runtime::guard::GuardLimits;
use guac_lib::runtime::{OnDisk, Runtime};

type Check = fn(&Value);

const CHECKS: &[(&str, Check)] = &[
    ("builds", builds),
    ("constants", constants),
    ("event-spec", event_spec),
    ("human-gap", gap),
    ("limits", limits),
    ("money", money),
    ("plain-text", plain_text),
    ("plugin-access", plugin_access),
    ("plugin-call", plugin_call),
    ("tool-names", tool_names),
    ("tool-surfaces", tool_surfaces),
    ("trigger", trigger),
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
    let mut checks: BTreeSet<String> = CHECKS.iter().map(|(name, _)| name.to_string()).collect();
    checks.insert("credentials".to_string());
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

fn shape(trigger: &Trigger) -> Value {
    match trigger {
        Trigger::Clock(Cadence::Once) => json!({ "kind": "once" }),
        Trigger::Clock(Cadence::Every(secs)) => json!({ "kind": "gap", "secs": secs }),
        Trigger::Clock(calendar) => json!({ "kind": "calendar", "repeat": calendar.as_str() }),
        Trigger::Event(event) => {
            json!({ "kind": "event", "service": event.service, "topic": event.topic })
        }
    }
}

fn trigger(file: &Value) {
    each(file, |spec: String| {
        Trigger::parse(&spec)
            .map(|trigger| json!({ "trigger": shape(&trigger), "repeats": trigger.repeats() }))
    });
}

fn event_spec(file: &Value) {
    #[derive(Deserialize)]
    struct Spec {
        service: String,
        topic: String,
    }
    each(file, |spec: Spec| EventTrigger { service: spec.service, topic: spec.topic }.as_str());
}

fn gap(file: &Value) {
    each(file, human_gap);
}

fn tool_names(file: &Value) {
    #[derive(Deserialize)]
    struct Call {
        tool: String,
        args: Value,
    }
    each(file, |call: Call| {
        let call =
            ToolCall { id: "call".into(), name: call.tool, arguments: call.args.to_string() };
        match tools::parse(&call, &[]) {
            Ok(ToolInvocation::AttachFile { files }) => files,
            Ok(ToolInvocation::SendMessage { to, .. }) => to,
            Ok(other) => panic!("{} read as {other:?}", call.name),
            Err(_) => Vec::new(),
        }
    });
}

fn tool_surfaces(file: &Value) {
    each(file, |name: String| tools::surface_of(&name));
    let named: BTreeSet<String> =
        file.as_array().unwrap().iter().map(|case| parsed(&case["input"])).collect();
    let missing: Vec<String> = tools::specs(Surfaces::both(), Modalities::seeing())
        .into_iter()
        .map(|spec| spec.name)
        .filter(|name| !named.contains(name))
        .collect();
    assert_eq!(missing, Vec::<String>::new(), "tools with no surface vector");
}

fn plugin_call(file: &Value) {
    each(file, |name: String| {
        let connected: Vec<PluginKind> = name
            .split_once(tools::PLUGIN_SEPARATOR)
            .filter(|(slug, _)| !slug.is_empty())
            .map(|(slug, _)| PluginKind::Custom {
                slug: slug.to_string(),
                endpoint: "https://mcp.example.com".into(),
            })
            .into_iter()
            .collect();
        tools::split_plugin_tool(&name, &connected)
            .map(|(kind, tool)| json!({ "server": kind.slug(), "tool": tool }))
    });
}

fn plain_text(file: &Value) {
    for case in file.as_array().expect("a vector file is a list of cases") {
        let shared = case.get("expected");
        let (rust, web) = (case.get("rust"), case.get("web"));
        assert!(
            shared.is_some() != (rust.is_some() && web.is_some()),
            "a case states one text for both sides, or a deliberate difference with one for each: {case}"
        );
        let expected: String = parsed(shared.or(rust).unwrap());
        let envelope: Envelope = parsed(&json!({
            "id": "00000000-0000-4000-8000-000000000010",
            "runId": "00000000-0000-4000-8000-000000000011",
            "channelId": "00000000-0000-4000-8000-000000000001",
            "from": { "kind": "human" },
            "to": { "kind": "agent", "id": "00000000-0000-4000-8000-000000000001" },
            "parts": case["input"],
            "trust": "operator",
            "hop": 0,
            "expectsReply": true,
            "intent": "work",
            "cause": null,
            "createdAt": 0
        }));
        assert_eq!(envelope.plain_text(), expected, "input {}", case["input"]);
    }
}

fn builds(file: &Value) {
    each(file, |(a, b): (String, String)| guac_lib::ipc::different_builds(&a, &b));
}

fn money(file: &Value) {
    each(file, guac_lib::menubar::money);
}

fn constants(file: &Value) {
    let mut ours = json!({
        "stdio": STDIO,
        "maxAgentNameLength": MAX_NAME_LEN,
        "catalog": PluginKind::ALL.iter().map(PluginKind::slug).collect::<Vec<_>>(),
    });
    #[cfg(feature = "desktop")]
    let clicked = json!(guac_lib::notifications::CLICKED);
    #[cfg(not(feature = "desktop"))]
    let clicked = file["notificationClicked"].clone();
    ours["notificationClicked"] = clicked;
    assert_eq!(&ours, file);
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Credential {
    service: String,
    env_var: String,
}

#[derive(Deserialize)]
struct Command {
    connectors: Vec<Credential>,
    command: String,
    summary: String,
}

#[derive(Deserialize)]
struct Written {
    written: String,
}

#[tokio::test]
async fn every_credential_vector_is_the_wording_the_runtime_writes() {
    for case in vectors("credentials").as_array().unwrap() {
        let command: Command = parsed(&case["input"]);
        let expected: Written = parsed(&case["expected"]);
        let dir = tempfile::tempdir().unwrap();
        let store = Store::open(&dir.path().join("guac.db")).unwrap();
        let card = store
            .create_agent(&CleanDraft {
                group_id: None,
                name: "Ada".into(),
                avatar: "orb".into(),
                color: "#7fb069".into(),
                model: "test/model".into(),
                subscription_model: None,
                reasoning_effort: None,
                system_prompt: String::new(),
                skills: vec![],
            })
            .unwrap();
        for credential in command.connectors {
            store
                .create_connector(&CleanConnector {
                    group_id: card.group_id,
                    service: credential.service,
                    account: String::new(),
                    env_var: credential.env_var,
                    note: String::new(),
                    secret: "not-a-real-secret".into(),
                    agents: vec![card.id],
                })
                .unwrap();
        }
        let config = AppConfig {
            version: guac_lib::config::CURRENT_VERSION,
            operator_name: String::new(),
            inference: Default::default(),
            limits: Default::default(),
            e2b: Default::default(),
            kernel: Default::default(),
            webhook: Default::default(),
            quick_actions: vec![],
            widgets: vec![],
            attention: Default::default(),
            delivery: Default::default(),
        };
        let runtime = Runtime::new(
            store,
            LlmClient::new().unwrap(),
            config,
            OnDisk::under(dir.path()),
            RecordingSink::new(),
        );
        let written = runtime.credentials_named_in(&card, &command.command) + &command.summary;
        assert_eq!(written, expected.written, "input {}", case["input"]);
    }
}
