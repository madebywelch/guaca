use std::collections::{BTreeMap, BTreeSet};
use std::fmt::Debug;
use std::path::PathBuf;

use serde::de::DeserializeOwned;
use serde::Deserialize;
use serde_json::{json, Value};

use chrono::{Local, NaiveDate, TimeZone};
use guac_lib::config::{AppConfig, Provider};
use guac_lib::db::Store;
use guac_lib::domain::agent::{AgentDraft, CleanDraft, MAX_NAME_LEN};
use guac_lib::domain::attachment::Attachment;
use guac_lib::domain::connector::CleanConnector;
use guac_lib::domain::envelope::Envelope;
use guac_lib::domain::group::{GroupInference, InferenceOverrides};
use guac_lib::domain::ids::{AgentId, GroupId};
use guac_lib::domain::occasion::{parse_when, When};
use guac_lib::domain::plugin::{PluginAccess, PluginKind, STDIO};
use guac_lib::domain::routine::{human_gap, Cadence, EventTrigger, Trigger};
use guac_lib::domain::view::AWAY_AFTER_MS;
use guac_lib::llm::modality::Modalities;
use guac_lib::llm::openrouter::{LlmClient, ToolCall};
use guac_lib::llm::tools::{self, Surfaces, ToolInvocation};
use guac_lib::menubar::{crew_named, Crew};
use guac_lib::runtime::events::RecordingSink;
use guac_lib::runtime::guard::GuardLimits;
use guac_lib::runtime::{OnDisk, Runtime};
use guac_lib::secrets::Secret;

type Check = fn(&Value);

const CHECKS: &[(&str, Check)] = &[
    ("builds", builds),
    ("compact", compact),
    ("constants", constants),
    ("crew-name", crew_name),
    ("event-spec", event_spec),
    ("hireable", hireable),
    ("human-gap", gap),
    ("inference", inference),
    ("limits", limits),
    ("money", money),
    ("occasion-field", occasion_field),
    ("plain-text", plain_text),
    ("plugin-access", plugin_access),
    ("plugin-call", plugin_call),
    ("sizes", sizes),
    ("tool-names", tool_names),
    ("titled", titled),
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

fn hireable(file: &Value) {
    #[derive(Deserialize)]
    #[serde(rename_all = "camelCase")]
    struct Preset {
        id: String,
        name: String,
        avatar: String,
        color: String,
        system_prompt: String,
        skills: Vec<String>,
    }
    for case in file.as_array().expect("the catalog is a list of presets") {
        let preset: Preset = parsed(case);
        let draft = AgentDraft {
            group_id: None,
            name: preset.name,
            avatar: preset.avatar,
            color: preset.color,
            model: String::new(),
            subscription_model: None,
            reasoning_effort: None,
            system_prompt: preset.system_prompt,
            skills: preset.skills,
        };
        let clean =
            draft.validate().unwrap_or_else(|e| panic!("{} cannot be hired: {e}", preset.id));
        assert_eq!(
            (clean.name, clean.avatar, clean.color, clean.system_prompt, clean.skills),
            (draft.name, draft.avatar, draft.color, draft.system_prompt, draft.skills),
            "{} is stored differently from how the cafeteria shows it",
            preset.id
        );
    }
}

fn titled(file: &Value) {
    each(file, |service: String| guac_lib::domain::routine::titled(&service));
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

fn crew_name(file: &Value) {
    #[derive(Deserialize)]
    struct Named {
        groups: Vec<Crew>,
        group: GroupId,
    }
    each(file, |case: Named| crew_named(&case.groups, case.group).map(|crew| crew.name.clone()));
}

fn compact(file: &Value) {
    each(file, guac_lib::menubar::compact);
}

fn money(file: &Value) {
    each(file, guac_lib::menubar::money);
}

fn sizes(file: &Value) {
    each(file, |bytes: u64| {
        Attachment { digest: String::new(), name: String::new(), mime: String::new(), bytes }.size()
    });
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Paying {
    provider: Option<Provider>,
    api_key_set: bool,
}

#[derive(Deserialize)]
struct Choices {
    group: Option<Paying>,
    workspace: Paying,
}

fn key(set: bool) -> Option<Secret> {
    set.then(|| "sk-vector".into())
}

fn inference(file: &Value) {
    each(file, |choices: Choices| {
        let mut base = AppConfig::default().inference;
        base.provider = choices.workspace.provider.expect("a workspace always has a provider");
        base.api_key = key(choices.workspace.api_key_set).unwrap_or_default();
        let group = choices.group.map_or_else(GroupInference::default, |group| GroupInference {
            overrides: InferenceOverrides { provider: group.provider, ..Default::default() },
            api_key: key(group.api_key_set),
        });
        let paid = group.apply(&base);
        json!({ "provider": paid.provider, "keyed": !paid.api_key.expose().trim().is_empty() })
    });
}

#[derive(Deserialize)]
struct Wall {
    date: (i32, u32, u32),
    time: Option<(u32, u32)>,
}

fn occasion_field(file: &Value) {
    for case in file.as_array().expect("a vector file is a list of cases") {
        let wall: Wall = parsed(&case["input"]);
        let field: String = parsed(&case["expected"]);
        let (year, month, day) = wall.date;
        let (hour, minute) = wall.time.unwrap_or((0, 0));
        let naive = NaiveDate::from_ymd_opt(year, month, day)
            .and_then(|date| date.and_hms_opt(hour, minute, 0))
            .expect("a vector names a real wall time");
        let starts_at = Local
            .from_local_datetime(&naive)
            .single()
            .expect("a vector names an unambiguous wall time")
            .timestamp_millis();
        assert_eq!(
            parse_when(&field).unwrap_or_else(|e| panic!("{field}: {e}")),
            When { starts_at, all_day: wall.time.is_none() },
            "field {field}"
        );
    }
}

fn constants(file: &Value) {
    let mut ours = json!({
        "stdio": STDIO,
        "maxAgentNameLength": MAX_NAME_LEN,
        "awayAfterMs": AWAY_AFTER_MS,
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
