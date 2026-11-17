#![allow(dead_code)]

pub mod live;

use std::collections::HashMap;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use axum::response::IntoResponse;
use axum::routing::post;
use axum::Router;

use guac_lib::config::{AppConfig, E2bConfig, InferenceConfig, KernelConfig};
use guac_lib::db::Store;
use guac_lib::domain::agent::{CleanDraft, Lifecycle};
use guac_lib::domain::envelope::{Envelope, Participant};
use guac_lib::domain::group::CleanGroup;
use guac_lib::domain::ids::{AgentId, RunId};
use guac_lib::llm::openrouter::LlmClient;
use guac_lib::runtime::events::{Activity, RecordingSink, UiEvent};
use guac_lib::runtime::guard::GuardLimits;
use guac_lib::runtime::{Drained, OnDisk, Runtime};
use guac_lib::trajectory::{self, Trajectory};

#[derive(Debug, Clone)]
pub enum Script {
    Say(String),
    Thinking { about: String, say: String },
    SendTo { recipients: Vec<String>, text: String },
    Instruct { recipients: Vec<String>, text: String },
    SendFiles { recipients: Vec<String>, text: String, files: Vec<String> },
    Attach { tool: String, files: Vec<String> },
    Write { tool: String, name: String, content: String },
    Narrate { text: String, then: Box<Script> },
    Mull { about: String, then: Box<Script> },
    Directory,
    Plugin { name: String, arguments: serde_json::Value },
    Notes(String),
    Memory(String),
    Progress(String),
    Hire { name: String, instructions: String, notes: String },
    Book { name: String, what: String, repeat: String },
    Retime { id: String, repeat: String },
    Note { title: String, starts_at: String },
    Move { action: String, id: String, starts_at: String },
    Skill(serde_json::Value),
    Settings(serde_json::Value),
    Notebook(serde_json::Value),
    AskPermission { action: String, because: String },
    AskQuestion { question: String, options: Vec<String> },
    Escalate(String),
    RunCommand(String),
    Look,
    Open(String),
    Errands(Vec<String>),
    Unavailable,
    Hang,
}

pub const HANG: Duration = Duration::from_secs(30);

pub fn frame(value: serde_json::Value) -> String {
    format!("data: {value}\n\n")
}

pub const CALL_TOKENS: (u32, u32) = (100, 20);

pub fn render(script: &Script) -> String {
    let mut body = String::new();
    match script {
        Script::Say(text) => {
            for piece in text.as_bytes().chunks(7) {
                let piece = String::from_utf8_lossy(piece).to_string();
                body.push_str(&frame(
                    serde_json::json!({"choices":[{"delta":{"content": piece}}]}),
                ));
            }
            body.push_str(&frame(
                serde_json::json!({"choices":[{"delta":{},"finish_reason":"stop"}]}),
            ));
        }
        Script::Thinking { about, say } => {
            for piece in about.as_bytes().chunks(9) {
                let piece = String::from_utf8_lossy(piece).to_string();
                body.push_str(&frame(
                    serde_json::json!({"choices":[{"delta":{"reasoning": piece}}]}),
                ));
            }
            for piece in say.as_bytes().chunks(7) {
                let piece = String::from_utf8_lossy(piece).to_string();
                body.push_str(&frame(
                    serde_json::json!({"choices":[{"delta":{"content": piece}}]}),
                ));
            }
            body.push_str(&frame(
                serde_json::json!({"choices":[{"delta":{},"finish_reason":"stop"}]}),
            ));
        }
        Script::SendTo { recipients, text }
        | Script::Instruct { recipients, text }
        | Script::SendFiles { recipients, text, .. } => {
            let intent = if matches!(script, Script::SendTo { .. }) { "courtesy" } else { "work" };
            let mut args = serde_json::json!({
                "to": recipients,
                "text": text,
                "intent": intent,
            });
            if let Script::SendFiles { files, .. } = script {
                args["files"] = serde_json::json!(files);
            }
            let args = args.to_string();
            body.push_str(&frame(serde_json::json!({"choices":[{"delta":{"tool_calls":[
                {"index":0,"id":"call_send","type":"function",
                 "function":{"name":"send_message","arguments":""}}
            ]}}]})));
            for piece in args.as_bytes().chunks(11) {
                let piece = String::from_utf8_lossy(piece).to_string();
                body.push_str(&frame(serde_json::json!({"choices":[{"delta":{"tool_calls":[
                    {"index":0,"function":{"arguments": piece}}
                ]}}]})));
            }
            body.push_str(&frame(
                serde_json::json!({"choices":[{"delta":{},"finish_reason":"tool_calls"}]}),
            ));
        }
        Script::Attach { tool, files } => {
            let args = serde_json::json!({ "files": files }).to_string();
            body.push_str(&frame(serde_json::json!({"choices":[{"delta":{"tool_calls":[
                {"index":0,"id":"call_attach","type":"function",
                 "function":{"name": tool,"arguments": args}}
            ]}}]})));
            body.push_str(&frame(
                serde_json::json!({"choices":[{"delta":{},"finish_reason":"tool_calls"}]}),
            ));
        }
        Script::Write { tool, name, content } => {
            let args = serde_json::json!({ "name": name, "content": content }).to_string();
            body.push_str(&frame(serde_json::json!({"choices":[{"delta":{"tool_calls":[
                {"index":0,"id":"call_write","type":"function",
                 "function":{"name": tool,"arguments": args}}
            ]}}]})));
            body.push_str(&frame(
                serde_json::json!({"choices":[{"delta":{},"finish_reason":"tool_calls"}]}),
            ));
        }
        Script::Narrate { text, then } => {
            for piece in text.as_bytes().chunks(7) {
                let piece = String::from_utf8_lossy(piece).to_string();
                body.push_str(&frame(
                    serde_json::json!({"choices":[{"delta":{"content": piece}}]}),
                ));
            }
            body.push_str(&render(then));
        }
        Script::Mull { about, then } => {
            for piece in about.as_bytes().chunks(9) {
                let piece = String::from_utf8_lossy(piece).to_string();
                body.push_str(&frame(
                    serde_json::json!({"choices":[{"delta":{"reasoning": piece}}]}),
                ));
            }
            body.push_str(&render(then));
        }
        Script::Notes(content) | Script::Memory(content) => {
            let tool =
                if matches!(script, Script::Memory(_)) { "update_memory" } else { "update_notes" };
            let args = serde_json::json!({ "content": content }).to_string();
            body.push_str(&frame(serde_json::json!({"choices":[{"delta":{"tool_calls":[
                {"index":0,"id":"call_notes","type":"function",
                 "function":{"name": tool,"arguments": args}}
            ]}}]})));
            body.push_str(&frame(
                serde_json::json!({"choices":[{"delta":{},"finish_reason":"tool_calls"}]}),
            ));
        }
        Script::Progress(note) => {
            let args = serde_json::json!({ "note": note }).to_string();
            body.push_str(&frame(serde_json::json!({"choices":[{"delta":{"tool_calls":[
                {"index":0,"id":"call_progress","type":"function",
                 "function":{"name":"note_progress","arguments": args}}
            ]}}]})));
            body.push_str(&frame(
                serde_json::json!({"choices":[{"delta":{},"finish_reason":"tool_calls"}]}),
            ));
        }
        Script::AskPermission { action, because } => {
            let args = serde_json::json!({ "action": action, "because": because }).to_string();
            body.push_str(&frame(serde_json::json!({"choices":[{"delta":{"tool_calls":[
                {"index":0,"id":"call_ask","type":"function",
                 "function":{"name":"request_permission","arguments": args}}
            ]}}]})));
            body.push_str(&frame(
                serde_json::json!({"choices":[{"delta":{},"finish_reason":"tool_calls"}]}),
            ));
        }
        Script::AskQuestion { question, options } => {
            let mut args = serde_json::json!({ "question": question });
            if !options.is_empty() {
                args["options"] = serde_json::json!(options);
            }
            let args = args.to_string();
            body.push_str(&frame(serde_json::json!({"choices":[{"delta":{"tool_calls":[
                {"index":0,"id":"call_question","type":"function",
                 "function":{"name":"ask_operator","arguments": args}}
            ]}}]})));
            body.push_str(&frame(
                serde_json::json!({"choices":[{"delta":{},"finish_reason":"tool_calls"}]}),
            ));
        }
        Script::Escalate(summary) => {
            let args = serde_json::json!({ "summary": summary }).to_string();
            body.push_str(&frame(serde_json::json!({"choices":[{"delta":{"tool_calls":[
                {"index":0,"id":"call_escalate","type":"function",
                 "function":{"name":"escalate","arguments": args}}
            ]}}]})));
            body.push_str(&frame(
                serde_json::json!({"choices":[{"delta":{},"finish_reason":"tool_calls"}]}),
            ));
        }
        Script::RunCommand(command) => {
            let args = serde_json::json!({ "command": command }).to_string();
            body.push_str(&frame(serde_json::json!({"choices":[{"delta":{"tool_calls":[
                {"index":0,"id":"call_run_command","type":"function",
                 "function":{"name":"run_command","arguments": args}}
            ]}}]})));
            body.push_str(&frame(
                serde_json::json!({"choices":[{"delta":{},"finish_reason":"tool_calls"}]}),
            ));
        }
        Script::Look => {
            let args = serde_json::json!({ "action": "look" }).to_string();
            body.push_str(&frame(serde_json::json!({"choices":[{"delta":{"tool_calls":[
                {"index":0,"id":"call_screen","type":"function",
                 "function":{"name":"use_screen","arguments": args}}
            ]}}]})));
            body.push_str(&frame(
                serde_json::json!({"choices":[{"delta":{},"finish_reason":"tool_calls"}]}),
            ));
        }
        Script::Open(url) => {
            let args = serde_json::json!({ "action": "open", "url": url }).to_string();
            body.push_str(&frame(serde_json::json!({"choices":[{"delta":{"tool_calls":[
                {"index":0,"id":"call_browse","type":"function",
                 "function":{"name":"browse","arguments": args}}
            ]}}]})));
            body.push_str(&frame(
                serde_json::json!({"choices":[{"delta":{},"finish_reason":"tool_calls"}]}),
            ));
        }
        Script::Book { name, what, repeat } => {
            let args =
                serde_json::json!({ "action": "add", "name": name, "what": what, "repeat": repeat })
                    .to_string();
            body.push_str(&frame(serde_json::json!({"choices":[{"delta":{"tool_calls":[
                {"index":0,"id":"call_book","type":"function",
                 "function":{"name":"schedule","arguments": args}}
            ]}}]})));
            body.push_str(&frame(
                serde_json::json!({"choices":[{"delta":{},"finish_reason":"tool_calls"}]}),
            ));
        }
        Script::Retime { id, repeat } => {
            let args =
                serde_json::json!({ "action": "update", "id": id, "repeat": repeat }).to_string();
            body.push_str(&frame(serde_json::json!({"choices":[{"delta":{"tool_calls":[
                {"index":0,"id":"call_retime","type":"function",
                 "function":{"name":"schedule","arguments": args}}
            ]}}]})));
            body.push_str(&frame(
                serde_json::json!({"choices":[{"delta":{},"finish_reason":"tool_calls"}]}),
            ));
        }
        Script::Note { title, starts_at } => {
            let args =
                serde_json::json!({ "action": "add", "title": title, "starts_at": starts_at })
                    .to_string();
            body.push_str(&frame(serde_json::json!({"choices":[{"delta":{"tool_calls":[
                {"index":0,"id":"call_note","type":"function",
                 "function":{"name":"calendar","arguments": args}}
            ]}}]})));
            body.push_str(&frame(
                serde_json::json!({"choices":[{"delta":{},"finish_reason":"tool_calls"}]}),
            ));
        }
        Script::Move { action, id, starts_at } => {
            let args = serde_json::json!({ "action": action, "id": id, "starts_at": starts_at })
                .to_string();
            body.push_str(&frame(serde_json::json!({"choices":[{"delta":{"tool_calls":[
                {"index":0,"id":"call_move","type":"function",
                 "function":{"name":"calendar","arguments": args}}
            ]}}]})));
            body.push_str(&frame(
                serde_json::json!({"choices":[{"delta":{},"finish_reason":"tool_calls"}]}),
            ));
        }
        Script::Hire { name, instructions, notes } => {
            let args =
                serde_json::json!({ "name": name, "instructions": instructions, "notes": notes })
                    .to_string();
            body.push_str(&frame(serde_json::json!({"choices":[{"delta":{"tool_calls":[
                {"index":0,"id":"call_hire","type":"function",
                 "function":{"name":"create_agent","arguments": args}}
            ]}}]})));
            body.push_str(&frame(
                serde_json::json!({"choices":[{"delta":{},"finish_reason":"tool_calls"}]}),
            ));
        }
        Script::Unavailable | Script::Hang => {}
        Script::Directory => {
            body.push_str(&frame(serde_json::json!({"choices":[{"delta":{"tool_calls":[
                {"index":0,"id":"call_dir","type":"function",
                 "function":{"name":"directory","arguments":"{}"}}
            ]}}]})));
            body.push_str(&frame(
                serde_json::json!({"choices":[{"delta":{},"finish_reason":"tool_calls"}]}),
            ));
        }
        Script::Plugin { name, arguments } => {
            body.push_str(&frame(serde_json::json!({"choices":[{"delta":{"tool_calls":[
                {"index":0,"id":"call_plugin","type":"function",
                 "function":{"name":name,"arguments":arguments.to_string()}}
            ]}}]})));
            body.push_str(&frame(
                serde_json::json!({"choices":[{"delta":{},"finish_reason":"tool_calls"}]}),
            ));
        }
        Script::Errands(briefs) => {
            body.push_str(&frame(serde_json::json!({"choices":[{"delta":{"tool_calls":[
                {"index":0,"id":"call_errand","type":"function",
                 "function":{"name":"errand",
                             "arguments":serde_json::json!({"briefs": briefs}).to_string()}}
            ]}}]})));
            body.push_str(&frame(
                serde_json::json!({"choices":[{"delta":{},"finish_reason":"tool_calls"}]}),
            ));
        }
        Script::Skill(arguments) | Script::Settings(arguments) | Script::Notebook(arguments) => {
            let name = match script {
                Script::Skill(_) => "skill",
                Script::Settings(_) => "settings",
                _ => "notebook",
            };
            body.push_str(&frame(serde_json::json!({"choices":[{"delta":{"tool_calls":[
                {"index":0,"id":"call_skill","type":"function",
                 "function":{"name":name,"arguments":arguments.to_string()}}
            ]}}]})));
            body.push_str(&frame(
                serde_json::json!({"choices":[{"delta":{},"finish_reason":"tool_calls"}]}),
            ));
        }
    }
    body.push_str(&frame(serde_json::json!({
        "choices": [],
        "usage": {"prompt_tokens": CALL_TOKENS.0, "completion_tokens": CALL_TOKENS.1},
    })));
    body.push_str("data: [DONE]\n\n");
    body
}

pub fn speaker(body: &serde_json::Value) -> String {
    let system = body["messages"][0]["content"].as_str().unwrap_or_default();
    system
        .strip_prefix("You are ")
        .and_then(|rest| rest.split(',').next())
        .unwrap_or("unknown")
        .to_string()
}

pub fn standing_id(body: &serde_json::Value) -> Option<String> {
    let system = body["messages"][0]["content"].as_str().unwrap_or_default();
    let schedule = system.split("## Your schedule").nth(1)?.split("\n## ").next()?;
    schedule
        .lines()
        .find_map(|line| line.strip_prefix("- "))
        .and_then(|line| line.split_whitespace().next())
        .map(str::to_string)
}

pub fn reading_peer_replies(body: &serde_json::Value) -> bool {
    body["messages"]
        .as_array()
        .and_then(|m| m.last())
        .map(|m| {
            m["role"] == "user" && m["content"].as_str().unwrap_or_default().contains("[AGENT")
        })
        .unwrap_or(false)
}

pub fn has_tool_result(body: &serde_json::Value) -> bool {
    body["messages"].as_array().map(|m| m.iter().any(|msg| msg["role"] == "tool")).unwrap_or(false)
}

pub fn anyone_said(body: &serde_json::Value, needle: &str) -> bool {
    body["messages"]
        .as_array()
        .map(|messages| {
            messages
                .iter()
                .filter(|msg| msg["role"] != "system")
                .filter_map(|msg| msg["content"].as_str())
                .any(|content| content.contains(needle))
        })
        .unwrap_or(false)
}

pub struct Stub {
    pub base_url: String,
    pub calls: Arc<AtomicUsize>,
    pub transcript: Arc<parking_lot::Mutex<Vec<serde_json::Value>>>,
}

pub async fn serve<F>(decide: F) -> Stub
where
    F: Fn(&serde_json::Value) -> Script + Clone + Send + Sync + 'static,
{
    serve_publishing(None, decide).await
}

pub async fn serve_publishing<F>(listing: Option<serde_json::Value>, decide: F) -> Stub
where
    F: Fn(&serde_json::Value) -> Script + Clone + Send + Sync + 'static,
{
    let calls = Arc::new(AtomicUsize::new(0));
    let transcript = Arc::new(parking_lot::Mutex::new(Vec::new()));
    let call_counter = calls.clone();
    let recorder = transcript.clone();

    let mut app = Router::new();
    if let Some(listing) = listing {
        app = app.route(
            "/v1/models",
            axum::routing::get(move || {
                let listing = listing.clone();
                async move { axum::Json(listing) }
            }),
        );
    }
    let app = app.route(
        "/v1/chat/completions",
        post(move |body: axum::extract::Json<serde_json::Value>| {
            let decide = decide.clone();
            let call_counter = call_counter.clone();
            let recorder = recorder.clone();
            async move {
                call_counter.fetch_add(1, Ordering::SeqCst);
                recorder.lock().push(body.0.clone());
                let script = decide(&body.0);
                if matches!(script, Script::Unavailable) {
                    return (
                        axum::http::StatusCode::SERVICE_UNAVAILABLE,
                        "the model provider is having a moment",
                    )
                        .into_response();
                }
                if matches!(script, Script::Hang) {
                    tokio::time::sleep(HANG).await;
                }
                ([("content-type", "text/event-stream")], render(&script)).into_response()
            }
        }),
    );

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });

    Stub { base_url: format!("http://{addr}/v1"), calls, transcript }
}

pub struct Harness {
    pub runtime: Runtime,
    pub sink: Arc<RecordingSink>,
    pub ids: HashMap<String, AgentId>,
    pub _dir: tempfile::TempDir,
}

pub fn draft(name: &str, skills: &[&str]) -> CleanDraft {
    CleanDraft {
        group_id: None,
        name: name.into(),
        avatar: "avocado".into(),
        color: "#7fb069".into(),
        model: "test/model".into(),
        subscription_model: None,
        reasoning_effort: None,
        system_prompt: format!("You are the {name}."),
        skills: skills.iter().map(|s| s.to_string()).collect(),
    }
}

#[derive(Debug, Clone, Copy)]
pub struct Member<'a> {
    pub name: &'a str,
    pub skills: &'a [&'a str],
    pub group: Option<&'a str>,
    pub prompt: &'a str,
}

impl<'a> Member<'a> {
    pub fn new(name: &'a str, skills: &'a [&'a str]) -> Self {
        Member { name, skills, group: None, prompt: "" }
    }

    pub fn told(name: &'a str, skills: &'a [&'a str], prompt: &'a str) -> Self {
        Member { name, skills, group: None, prompt }
    }
}

pub fn harness(stub: &Stub, names: &[&str], limits: GuardLimits) -> Harness {
    let placed: Vec<(&str, Option<&str>)> = names.iter().map(|n| (*n, None)).collect();
    harness_in_groups(stub, &placed, limits)
}

pub fn harness_in_groups(
    stub: &Stub,
    placed: &[(&str, Option<&str>)],
    limits: GuardLimits,
) -> Harness {
    let crew: Vec<Member> = placed
        .iter()
        .map(|(name, group)| Member { name, skills: &["testing"], group: *group, prompt: "" })
        .collect();
    harness_of(stub, &crew, limits)
}

pub fn harness_of(stub: &Stub, crew: &[Member], limits: GuardLimits) -> Harness {
    build(stub, crew, limits, E2bConfig::default(), KernelConfig::default())
}

pub fn harness_with_computer(stub: &Stub, names: &[&str], limits: GuardLimits) -> Harness {
    let crew: Vec<Member> = names.iter().map(|name| Member::new(name, &["testing"])).collect();
    build(
        stub,
        &crew,
        limits,
        E2bConfig { api_key: "e2b-test".into(), ..Default::default() },
        KernelConfig::default(),
    )
}

pub fn harness_with_browser(stub: &Stub, names: &[&str], limits: GuardLimits) -> Harness {
    let crew: Vec<Member> = names.iter().map(|name| Member::new(name, &["testing"])).collect();
    build(
        stub,
        &crew,
        limits,
        E2bConfig::default(),
        KernelConfig { api_key: "kernel-test".into(), ..Default::default() },
    )
}

fn build(
    stub: &Stub,
    crew: &[Member],
    limits: GuardLimits,
    e2b: E2bConfig,
    kernel: KernelConfig,
) -> Harness {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open(&dir.path().join("guac.db")).unwrap();

    let mut groups: HashMap<&str, guac_lib::domain::ids::GroupId> = HashMap::new();
    let mut ids = HashMap::new();
    for member in crew {
        let group_id = member.group.map(|label| {
            *groups.entry(label).or_insert_with(|| {
                store
                    .create_group(&CleanGroup { name: label.to_string(), ..Default::default() })
                    .expect("group name is unique in a fresh store")
                    .id
            })
        });
        let mut d = draft(member.name, member.skills);
        d.group_id = group_id;
        if !member.prompt.is_empty() {
            d.system_prompt = member.prompt.to_string();
        }
        let card = store.create_agent(&d).unwrap();
        if !e2b.api_key.trim().is_empty() {
            store.set_has_computer(card.id, true).unwrap();
        }
        if !kernel.api_key.trim().is_empty() {
            store.set_has_browser(card.id, true).unwrap();
        }
        ids.insert(member.name.to_string(), card.id);
    }

    let config = AppConfig {
        version: guac_lib::config::CURRENT_VERSION,
        operator_name: String::new(),
        inference: InferenceConfig {
            base_url: stub.base_url.clone(),
            api_key: "sk-test".into(),
            default_model: "test/model".into(),
            request_timeout_secs: 10,
            ..Default::default()
        },
        limits,
        e2b,
        kernel,
        webhook: Default::default(),
        quick_actions: Vec::new(),
        widgets: Vec::new(),
        attention: Default::default(),
        delivery: Default::default(),
    };

    let sink = RecordingSink::new();
    let runtime = Runtime::new(
        store,
        LlmClient::new().unwrap(),
        config,
        OnDisk::under(dir.path()),
        sink.clone(),
    );
    runtime.start_all().unwrap();

    Harness { runtime, sink, ids, _dir: dir }
}

pub fn expect_normal(trajectory: &Trajectory, scenario: &str) {
    let anomalies = trajectory.anomalies();
    assert!(
        anomalies.is_empty(),
        "{scenario}\n\n{}\nwhat went wrong:\n{}",
        trajectory.ledger,
        anomalies.iter().map(|a| format!("  - {}", a.explain())).collect::<Vec<_>>().join("\n")
    );
}

pub fn tool_results(stub: &Stub) -> Vec<String> {
    stub.transcript
        .lock()
        .iter()
        .flat_map(|body| {
            body["messages"]
                .as_array()
                .cloned()
                .unwrap_or_default()
                .into_iter()
                .filter(|m| m["role"] == "tool")
                .map(|m| m["content"].as_str().unwrap_or_default().to_string())
                .collect::<Vec<_>>()
        })
        .collect()
}

pub fn last_tool_results(stub: &Stub) -> Vec<String> {
    stub.transcript
        .lock()
        .last()
        .and_then(|body| body["messages"].as_array().cloned())
        .unwrap_or_default()
        .into_iter()
        .filter(|m| m["role"] == "tool")
        .map(|m| m["content"].as_str().unwrap_or_default().to_string())
        .collect()
}

fn who(participant: Participant) -> String {
    match participant {
        Participant::Human => "operator".to_string(),
        Participant::System => "Guaca".to_string(),
        Participant::Agent { id } => id.short(),
    }
}

fn parts_of(envelope: &Envelope) -> String {
    use guac_lib::domain::envelope::Part;
    envelope
        .parts
        .iter()
        .map(|part| match part {
            Part::Text { text } => text.clone(),
            Part::Notice { kind, text } => format!("[{kind:?}] {text}"),
            Part::ToolCall { name, outcome, .. } => format!("[{name} -> {outcome:?}]"),
            Part::File(file) => format!("[file {}]", file.name),
            Part::Approval { summary, .. } => format!("[asks: {summary}]"),
            Part::Question { question, options, .. } => {
                format!("[asks the operator: {question} ({})]", options.join(" | "))
            }
            Part::Routine { name, what, .. } => format!("[routine {name:?}] {what}"),
            Part::Decision { request, answer, resumed, .. } => format!(
                "[{} {:?}] {answer}",
                if *resumed { "resumes decision" } else { "answers decision" },
                request.question
            ),
            Part::Json { name, .. } => format!("[{name}]"),
            Part::Tidy { reported, .. } => format!("[tidy of {} lines]", reported.len()),
        })
        .collect::<Vec<_>>()
        .join(" ")
}

pub fn prompts_by_agent(stub: &Stub) -> HashMap<String, String> {
    let mut out = HashMap::new();
    for body in stub.transcript.lock().iter() {
        out.insert(
            speaker(body),
            body["messages"][0]["content"].as_str().unwrap_or_default().to_string(),
        );
    }
    out
}

pub fn tools_by_agent(stub: &Stub) -> HashMap<String, Vec<String>> {
    let mut out = HashMap::new();
    for body in stub.transcript.lock().iter() {
        let names: Vec<String> = body["tools"]
            .as_array()
            .cloned()
            .unwrap_or_default()
            .iter()
            .filter_map(|tool| tool["function"]["name"].as_str().map(str::to_string))
            .collect();
        out.insert(speaker(body), names);
    }
    out
}

pub fn calls_by_agent(stub: &Stub) -> HashMap<String, usize> {
    let mut out: HashMap<String, usize> = HashMap::new();
    for body in stub.transcript.lock().iter() {
        *out.entry(speaker(body)).or_default() += 1;
    }
    out
}

impl Harness {
    pub fn id(&self, name: &str) -> AgentId {
        *self.ids.get(name).unwrap_or_else(|| panic!("no agent named {name}"))
    }

    pub fn channel_texts(&self, name: &str) -> Vec<String> {
        self.runtime
            .store()
            .channel_messages(self.id(name), 200)
            .unwrap()
            .iter()
            .map(Envelope::plain_text)
            .filter(|t| !t.is_empty())
            .collect()
    }

    pub fn messaged(&self) -> Vec<(String, usize)> {
        self.tally(self.feed())
    }

    pub fn messaged_by(&self, from: &str) -> Vec<(String, usize)> {
        let sender = Participant::Agent { id: self.id(from) };
        self.tally(self.feed().into_iter().filter(|e| e.from == sender).collect::<Vec<_>>())
    }

    fn tally(&self, envelopes: Vec<Envelope>) -> Vec<(String, usize)> {
        let mut counts: HashMap<String, usize> = HashMap::new();
        for envelope in envelopes {
            if let Some(id) = envelope.to.agent_id() {
                if let Some((name, _)) = self.ids.iter().find(|(_, known)| **known == id) {
                    *counts.entry(name.clone()).or_default() += 1;
                }
            }
        }
        let mut out: Vec<(String, usize)> = counts.into_iter().collect();
        out.sort();
        out
    }

    pub fn said_to_peers(&self, name: &str) -> Vec<String> {
        let sender = Participant::Agent { id: self.id(name) };
        self.feed()
            .into_iter()
            .filter(|e| e.from == sender)
            .map(|e| e.plain_text())
            .filter(|t| !t.is_empty())
            .collect()
    }

    pub fn envelopes(&self, names: &[&str]) -> Vec<Envelope> {
        let mut messages: Vec<Envelope> = Vec::new();
        let mut seen = std::collections::HashSet::new();
        for name in names {
            for envelope in self.runtime.store().channel_messages(self.id(name), 400).unwrap() {
                if seen.insert(envelope.id) {
                    messages.push(envelope);
                }
            }
        }
        messages.sort_by_key(|e| (e.created_at, e.id));
        messages
    }

    pub fn feed(&self) -> Vec<Envelope> {
        let store = self.runtime.store();
        let mut messages: Vec<Envelope> = store
            .list_groups()
            .unwrap()
            .into_iter()
            .flat_map(|group| store.conversation_flow(group.id, 400).unwrap())
            .filter(|e| e.from.is_agent() && e.to.is_agent())
            .collect();
        messages.sort_by_key(|e| (e.created_at, e.id));
        messages
    }

    pub fn trajectory(&self, run: RunId) -> Trajectory {
        let names: HashMap<AgentId, String> =
            self.ids.iter().map(|(name, id)| (*id, name.clone())).collect();
        let store_names: HashMap<AgentId, String> = self
            .runtime
            .store()
            .list_agents()
            .unwrap_or_default()
            .into_iter()
            .map(|card| (card.id, card.name))
            .collect();
        trajectory::read(&self.sink.snapshot(), run, &move |id| {
            names.get(&id).or_else(|| store_names.get(&id)).cloned().unwrap_or_else(|| "?".into())
        })
    }

    pub fn expect_normal(&self, run: RunId, scenario: &str) -> Trajectory {
        let trajectory = self.trajectory(run);
        expect_normal(&trajectory, scenario);
        trajectory
    }

    pub async fn wait_until(&self, what: &str, check: impl Fn(&Harness) -> bool) {
        let deadline = Instant::now() + Duration::from_secs(20);
        while !check(self) {
            assert!(Instant::now() < deadline, "timed out waiting for {what}");
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    }

    pub async fn awaited_request(&self) -> guac_lib::domain::ids::ApprovalId {
        let deadline = Instant::now() + Duration::from_secs(20);
        loop {
            let asked = self.sink.snapshot().into_iter().find_map(|event| match event {
                UiEvent::ApprovalRequested { approval_id, .. } => Some(approval_id),
                _ => None,
            });
            if let Some(id) = asked {
                return id;
            }
            assert!(Instant::now() < deadline, "no permission request arrived");
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    }

    pub fn agent_named(&self, name: &str) -> Option<guac_lib::domain::agent::AgentCard> {
        self.runtime
            .store()
            .list_agents()
            .unwrap()
            .into_iter()
            .find(|card| card.name == name && card.lifecycle != Lifecycle::Terminated)
    }

    pub fn pause(&self, name: &str) {
        self.runtime.store().set_lifecycle(self.id(name), Lifecycle::Paused).unwrap();
        self.runtime.pause_agent(self.id(name));
    }

    pub async fn park(&self, name: &str, text: &str) -> RunId {
        let id = self.id(name);
        let checkpoint = self.sink.snapshot().len();
        self.runtime.store().set_lifecycle(id, Lifecycle::Paused).unwrap();
        let run = self.runtime.send_from_human(id, text).unwrap();
        self.wait_until(&format!("{name} to park holding a message"), |h| {
            h.sink.snapshot().iter().skip(checkpoint).any(|event| {
                matches!(event, UiEvent::ActivityChanged { agent_id, activity: Activity::Paused }
                    if *agent_id == id)
            })
        })
        .await;
        run
    }

    pub fn resume(&self, name: &str) {
        self.runtime.store().set_lifecycle(self.id(name), Lifecycle::Active).unwrap();
        self.runtime.resume_agent(self.id(name));
    }

    pub async fn restart(self, within: Duration) -> (Harness, Drained) {
        let drained = self.runtime.drain(within).await;
        for id in self.ids.values() {
            self.runtime.stop_agent(*id);
        }
        let store = Store::open(&self._dir.path().join("guac.db")).unwrap();
        let recovered = guac_lib::boot::recover(&store).unwrap();
        let sink = RecordingSink::new();
        let runtime = Runtime::new(
            store,
            LlmClient::new().unwrap(),
            self.runtime.config(),
            OnDisk::under(self._dir.path()),
            sink.clone(),
        );
        runtime.start_all().unwrap();
        runtime.pick_up(recovered);
        (Harness { runtime, sink, ids: self.ids, _dir: self._dir }, drained)
    }

    pub async fn settle(&self, run: RunId) {
        self.settle_within(run, 20).await
    }

    pub async fn settle_within(&self, run: RunId, secs: u64) {
        if !self.settled_within(run, secs).await {
            panic!("run did not settle. messages so far:\n{}", self.transcript());
        }
    }

    pub async fn settled_within(&self, run: RunId, secs: u64) -> bool {
        let deadline = Instant::now() + Duration::from_secs(secs);
        loop {
            let settled = self
                .sink
                .count_of(|e| matches!(e, UiEvent::RunSettled { run_id, .. } if *run_id == run));
            if settled > 0 {
                tokio::time::sleep(Duration::from_millis(50)).await;
                assert_eq!(settled, 1, "RunSettled must fire exactly once per run");
                return true;
            }
            if Instant::now() > deadline {
                return false;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    }

    pub fn transcript(&self) -> String {
        self.sink
            .appended_messages()
            .iter()
            .map(|m| format!("  {} -> {}: {}", who(m.from), who(m.to), parts_of(m)))
            .collect::<Vec<_>>()
            .join("\n")
    }
}
