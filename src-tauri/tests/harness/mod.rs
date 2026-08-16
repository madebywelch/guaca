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

use guac_lib::config::{AppConfig, InferenceConfig};
use guac_lib::db::Store;
use guac_lib::domain::agent::{CleanDraft, Lifecycle};
use guac_lib::domain::envelope::Envelope;
use guac_lib::domain::group::CleanGroup;
use guac_lib::domain::ids::{AgentId, RunId};
use guac_lib::llm::openrouter::LlmClient;
use guac_lib::runtime::events::{RecordingSink, UiEvent};
use guac_lib::runtime::guard::GuardLimits;
use guac_lib::runtime::Runtime;
use guac_lib::workspace::Workspace;

// ---- scripted model ------------------------------------------------------

/// What the stub model should do for one request.
#[derive(Debug, Clone)]
pub enum Script {
    /// Emit plain text.
    Say(String),
    /// Emit a `send_message` tool call.
    SendTo { recipients: Vec<String>, text: String },
    /// Emit a `directory` tool call.
    Directory,
    /// Emit an `update_notes` tool call.
    Notes(String),
}

pub fn frame(value: serde_json::Value) -> String {
    format!("data: {value}\n\n")
}

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
        Script::SendTo { recipients, text } => {
            let args = serde_json::json!({ "to": recipients, "text": text }).to_string();
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
        Script::Notes(content) => {
            let args = serde_json::json!({ "content": content }).to_string();
            body.push_str(&frame(serde_json::json!({"choices":[{"delta":{"tool_calls":[
                {"index":0,"id":"call_notes","type":"function",
                 "function":{"name":"update_notes","arguments": args}}
            ]}}]})));
            body.push_str(&frame(
                serde_json::json!({"choices":[{"delta":{},"finish_reason":"tool_calls"}]}),
            ));
        }
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
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open(&dir.path().join("guac.db")).unwrap();

    let mut groups: HashMap<&str, guac_lib::domain::ids::GroupId> = HashMap::new();
    let mut ids = HashMap::new();
    for (name, group) in placed {
        let group_id = group.map(|label| {
            *groups.entry(label).or_insert_with(|| {
                store
                    .create_group(&CleanGroup {
                        name: (*label).to_string(),
                        base_url: None,
                        default_model: None,
                        api_key: None,
                    })
                    .expect("group name is unique in a fresh store")
                    .id
            })
        });
        let mut d = draft(name, &["testing"]);
        d.group_id = group_id;
        let card = store.create_agent(&d).unwrap();
        ids.insert(name.to_string(), card.id);
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
        e2b: Default::default(),
    };

    let sink = RecordingSink::new();
    let runtime = Runtime::new(
        store,
        LlmClient::new().unwrap(),
        config,
        Workspace::new(dir.path().join("workspace")),
        sink.clone(),
    );
    runtime.start_all().unwrap();

    Harness { runtime, sink, ids, _dir: dir }
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

    /// Polls until `check` holds, or panics on timeout.
    pub async fn wait_until(&self, what: &str, check: impl Fn(&Harness) -> bool) {
        let deadline = Instant::now() + Duration::from_secs(20);
        while !check(self) {
            assert!(Instant::now() < deadline, "timed out waiting for {what}");
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
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
    pub fn transcript(&self) -> String {
        self.sink
            .appended_messages()
            .iter()
            .map(|m| format!("  {:?} -> {:?}: {}", m.from, m.to, m.plain_text()))
            .collect::<Vec<_>>()
            .join("\n")
    }
}
