//! The scripted model and the runtime harness the end-to-end suites share.
//!
//! Included by two test binaries, each of which uses a different part of it.
//! Rust checks dead code per binary, so anything only one suite needs looks
//! unused to the other; the alternative is two copies that drift.
#![allow(dead_code)]
//
//!
//! These drive the real actor runtime against a scripted OpenAI-compatible
//! server, so everything between "operator presses enter" and "four agents have
//! replied" is exercised: tool-call assembly, the guard, channel routing, batch
//! coalescing, and settle detection. The only thing swapped out is the model.

use std::collections::HashMap;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use axum::response::IntoResponse;
use axum::routing::post;
use axum::Router;

use guac_lib::config::{AppConfig, E2bConfig, InferenceConfig};
use guac_lib::db::Store;
use guac_lib::domain::agent::{CleanDraft, Lifecycle};
use guac_lib::domain::envelope::{Envelope, Participant};
use guac_lib::domain::group::CleanGroup;
use guac_lib::domain::ids::{AgentId, RunId};
use guac_lib::files::FileStore;
use guac_lib::llm::openrouter::LlmClient;
use guac_lib::runtime::events::{RecordingSink, UiEvent};
use guac_lib::runtime::guard::GuardLimits;
use guac_lib::runtime::Runtime;
use guac_lib::trajectory::{self, Trajectory};
use guac_lib::workspace::Workspace;

// ---- scripted model ------------------------------------------------------

/// What the stub model should do for one request.
#[derive(Debug, Clone)]
pub enum Script {
    /// Emit plain text.
    Say(String),
    /// Publish some working, then say something. What a reasoning model sends:
    /// the thinking arrives first, in frames of the same shape as the text.
    Thinking { about: String, say: String },
    /// Emit a `send_message` tool call, declared as a courtesy: what a model
    /// sends when it is being polite, and what most of these scenarios play.
    SendTo { recipients: Vec<String>, text: String },
    /// The same call, declared as work. The distinction is the only thing that
    /// lets a peer be instructed twice in one run.
    Instruct { recipients: Vec<String>, text: String },
    /// A `send_message` carrying files, named the way a model names them.
    SendFiles { recipients: Vec<String>, text: String, files: Vec<String> },
    /// An `attach_file` call: files put on the answer rather than sent to a
    /// peer. The tool name is the one a model actually emitted, so the alias
    /// path is exercised wherever a scenario asks for it.
    Attach { tool: String, files: Vec<String> },
    /// Emit a `directory` tool call.
    Directory,
    /// Emit an `update_notes` tool call.
    Notes(String),
    /// The same call under the name a model reaches for when it is asked to
    /// update its memory rather than its notes.
    Memory(String),
    /// Emit a `create_agent` tool call.
    Hire { name: String, instructions: String, notes: String },
    /// Emit a `schedule` tool call that books a repeat on the calendar.
    Book { name: String, what: String, repeat: String },
    /// A `schedule` call that retimes a routine the agent already has, which is
    /// the whole of what it should do when asked to change one.
    Retime { id: String, repeat: String },
    /// Emit a `request_permission` tool call.
    AskOperator { action: String, because: String },
    /// Answer with a 503.
    ///
    /// Stands in for every failure worth retrying: a provider having a bad
    /// minute, a request that timed out, a connection that never opened. They
    /// arrive as different `LlmError`s and all answer `is_transient`, which is
    /// the only thing the retry loop asks them.
    Unavailable,
}

pub fn frame(value: serde_json::Value) -> String {
    format!("data: {value}\n\n")
}

/// What every scripted call reports having cost.
///
/// Fixed rather than proportional to the text: the point is that a real
/// provider counts, so the runtime's accounting runs end to end and a total
/// can be asserted exactly. A stub that reported nothing left `count_tokens`,
/// the usage table and the budget's own arithmetic untested.
pub const CALL_TOKENS: (u32, u32) = (100, 20);

pub fn render(script: &Script) -> String {
    let mut body = String::new();
    match script {
        Script::Say(text) => {
            // Split into fragments so the streaming path is exercised, not
            // just a single-chunk happy case.
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
            // Arguments arrive in pieces, as they really do.
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
        Script::AskOperator { action, because } => {
            let args = serde_json::json!({ "action": action, "because": because }).to_string();
            body.push_str(&frame(serde_json::json!({"choices":[{"delta":{"tool_calls":[
                {"index":0,"id":"call_ask","type":"function",
                 "function":{"name":"request_permission","arguments": args}}
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
        // Never rendered: the server answers it with a status, not a body.
        Script::Unavailable => {}
        Script::Directory => {
            body.push_str(&frame(serde_json::json!({"choices":[{"delta":{"tool_calls":[
                {"index":0,"id":"call_dir","type":"function",
                 "function":{"name":"directory","arguments":"{}"}}
            ]}}]})));
            body.push_str(&frame(
                serde_json::json!({"choices":[{"delta":{},"finish_reason":"tool_calls"}]}),
            ));
        }
    }
    // As a provider sends it: alone, after the content, in a frame carrying no
    // choices at all.
    body.push_str(&frame(serde_json::json!({
        "choices": [],
        "usage": {"prompt_tokens": CALL_TOKENS.0, "completion_tokens": CALL_TOKENS.1},
    })));
    body.push_str("data: [DONE]\n\n");
    body
}

/// Extracts `You are <Name>,` from the system prompt so the stub can answer as
/// whichever agent is asking.
pub fn speaker(body: &serde_json::Value) -> String {
    let system = body["messages"][0]["content"].as_str().unwrap_or_default();
    system
        .strip_prefix("You are ")
        .and_then(|rest| rest.split(',').next())
        .unwrap_or("unknown")
        .to_string()
}

/// The id of the first routine this agent's own prompt says it has standing.
///
/// Read out of the prompt rather than handed to the stub, because that is the
/// thing under test: an agent asked to change something it keeps has to be able
/// to find it without going to look, or it writes a second one instead.
pub fn standing_id(body: &serde_json::Value) -> Option<String> {
    let system = body["messages"][0]["content"].as_str().unwrap_or_default();
    let schedule = system.split("## Your schedule").nth(1)?.split("\n## ").next()?;
    schedule
        .lines()
        .find_map(|line| line.strip_prefix("- "))
        .and_then(|line| line.split_whitespace().next())
        .map(str::to_string)
}

/// True when the newest user turn carries peer messages, meaning this agent is
/// reading replies rather than being given a fresh instruction. A real model
/// would summarize here rather than broadcasting again.
pub fn reading_peer_replies(body: &serde_json::Value) -> bool {
    body["messages"]
        .as_array()
        .and_then(|m| m.last())
        .map(|m| {
            m["role"] == "user" && m["content"].as_str().unwrap_or_default().contains("[AGENT")
        })
        .unwrap_or(false)
}

/// True once this conversation already contains a tool result, meaning the
/// agent has already acted and should now speak.
pub fn has_tool_result(body: &serde_json::Value) -> bool {
    body["messages"].as_array().map(|m| m.iter().any(|msg| msg["role"] == "tool")).unwrap_or(false)
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
    let calls = Arc::new(AtomicUsize::new(0));
    let transcript = Arc::new(parking_lot::Mutex::new(Vec::new()));
    let call_counter = calls.clone();
    let recorder = transcript.clone();

    let app = Router::new().route(
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

// ---- harness -------------------------------------------------------------

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
        system_prompt: format!("You are the {name}."),
        skills: skills.iter().map(|s| s.to_string()).collect(),
    }
}

/// One agent in a scripted crew.
///
/// Skills are here because who a piece of work belongs to is not a question a
/// crew of identically-described agents can be asked: the roster is what a
/// coordinator chooses from, so a scenario about choosing has to build one.
#[derive(Debug, Clone, Copy)]
pub struct Member<'a> {
    pub name: &'a str,
    pub skills: &'a [&'a str],
    /// `None` is the default group, where everybody can reach everybody.
    pub group: Option<&'a str>,
    /// The card's own standing instructions. Empty takes the default.
    pub prompt: &'a str,
}

impl<'a> Member<'a> {
    pub fn new(name: &'a str, skills: &'a [&'a str]) -> Self {
        Member { name, skills, group: None, prompt: "" }
    }

    /// The same, carrying the standing instruction an operator would type.
    pub fn told(name: &'a str, skills: &'a [&'a str], prompt: &'a str) -> Self {
        Member { name, skills, group: None, prompt }
    }
}

pub fn harness(stub: &Stub, names: &[&str], limits: GuardLimits) -> Harness {
    // No group named, so every agent lands in the default one and can reach
    // every other. This is the control for the isolation tests below.
    let placed: Vec<(&str, Option<&str>)> = names.iter().map(|n| (*n, None)).collect();
    harness_in_groups(stub, &placed, limits)
}

/// Places each agent in the named group, creating groups on demand. `None`
/// means the default group.
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

/// A crew whose agents differ from each other, which is what a scenario about
/// delegation needs.
pub fn harness_of(stub: &Stub, crew: &[Member], limits: GuardLimits) -> Harness {
    build(stub, crew, limits, E2bConfig::default())
}

/// The same crew, in a workspace that has a computer provider configured.
///
/// The scenarios about acting in the operator's name need one. A permission
/// request from an agent with no computer and no browser is refused before the
/// operator is asked, because nothing such an agent can call reaches outside
/// the workspace, so a scenario about sending mail has to be a workspace where
/// mail could be sent. Nothing here calls E2B: a key set is what decides which
/// tools an agent is offered, and these tests script the model, not the machine.
pub fn harness_with_computer(stub: &Stub, names: &[&str], limits: GuardLimits) -> Harness {
    let crew: Vec<Member> = names.iter().map(|name| Member::new(name, &["testing"])).collect();
    build(stub, &crew, limits, E2bConfig { api_key: "e2b-test".into(), ..Default::default() })
}

fn build(stub: &Stub, crew: &[Member], limits: GuardLimits, e2b: E2bConfig) -> Harness {
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
        kernel: Default::default(),
    };

    let sink = RecordingSink::new();
    let runtime = Runtime::new(
        store,
        LlmClient::new().unwrap(),
        config,
        Workspace::new(dir.path().join("workspace")),
        FileStore::new(dir.path().join("files")),
        sink.clone(),
    );
    runtime.start_all().unwrap();

    Harness { runtime, sink, ids, _dir: dir }
}

/// Fails naming the anomaly and printing the whole ledger.
///
/// The ledger is the point. "one anomaly" is unactionable; the sequence that
/// produced it is what a person reads to find the defect.
pub fn expect_normal(trajectory: &Trajectory, scenario: &str) {
    let anomalies = trajectory.anomalies();
    assert!(
        anomalies.is_empty(),
        "{scenario}\n\n{}\nwhat went wrong:\n{}",
        trajectory.ledger,
        anomalies.iter().map(|a| format!("  - {}", a.explain())).collect::<Vec<_>>().join("\n")
    );
}

/// Every `role: "tool"` message the stub was sent, in order. This is how a test
/// sees what the model was actually told, which for a refusal is the assertion
/// that matters: a silently dropped message and a reported one look identical
/// from the outside.
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

fn who(participant: Participant) -> String {
    match participant {
        Participant::Human => "operator".to_string(),
        Participant::System => "Guaca".to_string(),
        Participant::Agent { id } => id.short(),
    }
}

/// One message rendered so that nothing in it is invisible.
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
            Part::Routine { name, what, .. } => format!("[routine {name:?}] {what}"),
            Part::Json { name, .. } => format!("[{name}]"),
        })
        .collect::<Vec<_>>()
        .join(" ")
}

/// The system prompt each agent was actually sent, by name.
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

/// How many model calls each agent made, by name.
///
/// The unit a crew costs its operator. An agent that was never involved in a
/// task should not appear here at all, which is the whole argument for
/// delegating to one peer rather than to everybody.
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

    /// Which peers were sent something, by name, with a count each.
    ///
    /// Who a message went to is the question a delegation scenario asks, and
    /// counting traffic cannot answer it: one message to the right agent and
    /// one to the wrong agent are the same number.
    pub fn messaged(&self) -> Vec<(String, usize)> {
        self.tally(self.feed())
    }

    /// The same, for what one agent sent.
    ///
    /// A coordinator's delegations and the answers coming back to it are both
    /// traffic at that agent, and only the first is the decision under test.
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

    /// Everything one agent said to a peer, wherever it was filed.
    ///
    /// Not `channel_texts`: a send is filed under the recipient and an answer
    /// under the sender, so an agent's own channel holds what it was asked and
    /// not always what it answered.
    pub fn said_to_peers(&self, name: &str) -> Vec<String> {
        let sender = Participant::Agent { id: self.id(name) };
        self.feed()
            .into_iter()
            .filter(|e| e.from == sender)
            .map(|e| e.plain_text())
            .filter(|t| !t.is_empty())
            .collect()
    }

    /// Peer traffic only, which is what most of these tests reason about.
    pub fn feed(&self) -> Vec<Envelope> {
        self.runtime
            .store()
            .conversation_flow(400)
            .unwrap()
            .into_iter()
            .filter(|e| e.from.is_agent() && e.to.is_agent())
            .collect()
    }

    /// What the run's machinery did, read from the events the UI is drawn
    /// from. See `guac_lib::trajectory`.
    pub fn trajectory(&self, run: RunId) -> Trajectory {
        let names: HashMap<AgentId, String> =
            self.ids.iter().map(|(name, id)| (*id, name.clone())).collect();
        // Agents hired mid-run are not in the harness's table, so the store is
        // the fallback: an anomaly about "?" is one nobody can act on.
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

    /// The same, asserted. Returns it so a scenario can go on to ask what it
    /// cost.
    pub fn expect_normal(&self, run: RunId, scenario: &str) -> Trajectory {
        let trajectory = self.trajectory(run);
        expect_normal(&trajectory, scenario);
        trajectory
    }

    /// Polls until `check` holds, or panics on timeout.
    pub async fn wait_until(&self, what: &str, check: impl Fn(&Harness) -> bool) {
        let deadline = Instant::now() + Duration::from_secs(20);
        while !check(self) {
            assert!(Instant::now() < deadline, "timed out waiting for {what}");
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    }

    /// Waits for an agent to park on a permission request, and returns it.
    ///
    /// Polling the event sink rather than the store because the request the
    /// test wants to answer is the one the UI would be drawing, and a turn is
    /// holding its line open until somebody does.
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

    pub fn resume(&self, name: &str) {
        self.runtime.store().set_lifecycle(self.id(name), Lifecycle::Active).unwrap();
        self.runtime.resume_agent(self.id(name));
    }

    /// Waits for the run to settle, or panics with what actually happened.
    /// Waits for a run to go quiet. Twenty seconds is generous for a stub.
    pub async fn settle(&self, run: RunId) {
        self.settle_within(run, 20).await
    }

    /// The same, for runs whose model is real.
    ///
    /// A scripted turn answers instantly; a real one takes seconds, and a
    /// delegation is several in sequence. The stub timeout was reporting live
    /// runs that were merely thinking as runs that had hung.
    pub async fn settle_within(&self, run: RunId, secs: u64) {
        if !self.settled_within(run, secs).await {
            panic!("run did not settle. messages so far:\n{}", self.transcript());
        }
    }

    /// The same wait, reporting rather than panicking.
    ///
    /// A live run holds real machines, and a panic here skips whatever the
    /// caller meant to release: the timeouts are exactly the runs that leave
    /// the most behind, because a run that overruns is a run whose agents are
    /// all still working. A caller that owns something has to be able to clean
    /// up before it fails.
    pub async fn settled_within(&self, run: RunId, secs: u64) -> bool {
        let deadline = Instant::now() + Duration::from_secs(secs);
        loop {
            let settled = self
                .sink
                .count_of(|e| matches!(e, UiEvent::RunSettled { run_id, .. } if *run_id == run));
            if settled > 0 {
                // Let any final persistence land before assertions read it.
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

    /// Everything said so far, for a failure that has to be actionable.
    ///
    /// Every part, not `plain_text`: a guard refusal and a failed call are
    /// `Notice` parts with no text in them, so a run that stopped because of one
    /// used to print a blank line where its reason was. The two failures worth
    /// having a transcript for are exactly those.
    pub fn transcript(&self) -> String {
        self.sink
            .appended_messages()
            .iter()
            .map(|m| format!("  {} -> {}: {}", who(m.from), who(m.to), parts_of(m)))
            .collect::<Vec<_>>()
            .join("\n")
    }
}
