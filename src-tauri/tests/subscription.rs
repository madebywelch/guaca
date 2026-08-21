//! End-to-end runtime tests for the ChatGPT subscription path.
//!
//! A sibling of `cascade.rs`, and deliberately not part of it. Those tests drive
//! the runtime against a scripted OpenAI-compatible server, which is the only
//! endpoint shape the app spoke until subscriptions existed. These drive the same
//! runtime against a scripted *Responses* server, because that is a different
//! request body, a different item model and a different set of stream events for
//! the same conversation.
//!
//! What is being tested is the seam rather than the translation: `llm/codex.rs`
//! has unit tests for the body it assembles and the stream it reads, and those
//! would all pass with the provider never dispatched, the credential never
//! reached and the model resolved from the wrong field. Everything here goes
//! through `Runtime`, so a turn only works if the dispatch, the sign-in store,
//! the model resolution, the tool loop and the accounting all agree.
//!
//! The stub asserts the parts of the protocol that were learned the hard way
//! against the real endpoint: no `temperature`, `instructions` present, tools
//! flat. A regression in any of those is a 400 in production and a silent pass
//! in a test that only checked the reply.

mod harness;

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

use axum::response::IntoResponse;
use axum::routing::post;
use axum::Router;
use parking_lot::Mutex;

use guac_lib::config::{AppConfig, InferenceConfig, Provider};
use guac_lib::db::Store;
use guac_lib::domain::ids::AgentId;
use guac_lib::files::FileStore;
use guac_lib::llm::openrouter::LlmClient;
use guac_lib::runtime::events::RecordingSink;
use guac_lib::runtime::guard::GuardLimits;
use guac_lib::runtime::Runtime;
use guac_lib::subscription::Subscription;
use guac_lib::workspace::Workspace;

use harness::{draft, frame};

/// What every scripted call reports having cost, in this protocol's spelling.
const CALL_TOKENS: (u32, u32) = (120, 30);

/// What the stub should do for one request.
#[derive(Debug, Clone)]
enum Say {
    Text(String),
    /// Working first, then an answer. What a reasoning model sends.
    Thinking {
        about: String,
        then: String,
    },
    /// A `send_message` call, as the Responses API delivers one.
    SendTo {
        to: Vec<String>,
        text: String,
    },
}

/// Renders a turn as the Codex backend really streams one.
///
/// The event names and their nesting are the protocol, and the shape here is
/// what a live call was observed to send: text arrives as `output_text.delta`,
/// a tool call is announced, streamed as argument fragments and then repeated
/// whole in `output_item.done`, and the usage rides inside `response.completed`
/// rather than in a frame of its own. Nothing says `[DONE]`.
fn render(say: &Say) -> String {
    let mut body = String::new();
    body.push_str(&frame(serde_json::json!({
        "type": "response.created",
        "response": { "status": "in_progress" },
    })));

    match say {
        Say::Text(text) => {
            push_text(&mut body, text);
        }
        Say::Thinking { about, then } => {
            for piece in about.as_bytes().chunks(9) {
                body.push_str(&frame(serde_json::json!({
                    "type": "response.reasoning_summary_text.delta",
                    "delta": String::from_utf8_lossy(piece),
                })));
            }
            push_text(&mut body, then);
        }
        Say::SendTo { to, text } => {
            let args =
                serde_json::json!({ "to": to, "text": text, "intent": "courtesy" }).to_string();
            body.push_str(&frame(serde_json::json!({
                "type": "response.output_item.added",
                "item": {
                    "type": "function_call", "call_id": "call_send",
                    "name": "send_message", "arguments": "",
                },
            })));
            // In fragments, as they really arrive. The runtime must not read
            // these as a second call on top of the finished item below.
            for piece in args.as_bytes().chunks(11) {
                body.push_str(&frame(serde_json::json!({
                    "type": "response.function_call_arguments.delta",
                    "item_id": "call_send",
                    "delta": String::from_utf8_lossy(piece),
                })));
            }
            body.push_str(&frame(serde_json::json!({
                "type": "response.output_item.done",
                "item": {
                    "type": "function_call", "call_id": "call_send",
                    "name": "send_message", "arguments": args,
                },
            })));
        }
    }

    body.push_str(&frame(serde_json::json!({
        "type": "response.completed",
        "response": {
            "status": "completed",
            "usage": {
                "input_tokens": CALL_TOKENS.0,
                "output_tokens": CALL_TOKENS.1,
                "output_tokens_details": { "reasoning_tokens": 0 },
            },
        },
    })));
    body
}

fn push_text(body: &mut String, text: &str) {
    for piece in text.as_bytes().chunks(7) {
        body.push_str(&frame(serde_json::json!({
            "type": "response.output_text.delta",
            "delta": String::from_utf8_lossy(piece),
        })));
    }
    body.push_str(&frame(serde_json::json!({
        "type": "response.output_text.done", "text": text,
    })));
}

/// A scripted Codex backend, and what it saw.
struct Stub {
    backend: String,
    seen: Arc<Mutex<Vec<serde_json::Value>>>,
    headers: Arc<Mutex<Vec<(String, String)>>>,
    calls: Arc<AtomicUsize>,
}

impl Stub {
    /// Every request body it was sent, in order.
    fn bodies(&self) -> Vec<serde_json::Value> {
        self.seen.lock().clone()
    }

    fn header(&self, name: &str) -> Option<String> {
        self.headers
            .lock()
            .iter()
            .find(|(key, _)| key.eq_ignore_ascii_case(name))
            .map(|(_, value)| value.clone())
    }
}

/// Which agent is asking, read out of `instructions` rather than out of a
/// message. That relocation is the single biggest difference between the two
/// protocols, so a stub that could not find the name here would not have been
/// testing the thing most likely to be wrong.
fn speaker(body: &serde_json::Value) -> String {
    let instructions = body["instructions"].as_str().unwrap_or_default();
    instructions
        .split_once("You are ")
        .and_then(|(_, rest)| rest.split([',', '.', '\n']).next())
        .unwrap_or_default()
        .trim()
        .to_string()
}

/// Whether this call is the model being handed its own tool result back.
fn has_tool_result(body: &serde_json::Value) -> bool {
    body["input"]
        .as_array()
        .map(|items| items.iter().any(|i| i["type"] == "function_call_output"))
        .unwrap_or(false)
}

async fn serve<F>(decide: F) -> Stub
where
    F: Fn(&serde_json::Value) -> Say + Clone + Send + Sync + 'static,
{
    let seen = Arc::new(Mutex::new(Vec::new()));
    let headers = Arc::new(Mutex::new(Vec::new()));
    let calls = Arc::new(AtomicUsize::new(0));

    let app = Router::new().route(
        "/responses",
        post({
            let seen = seen.clone();
            let headers = headers.clone();
            let calls = calls.clone();
            move |request: axum::extract::Request| {
                let (decide, seen, headers, calls) =
                    (decide.clone(), seen.clone(), headers.clone(), calls.clone());
                async move {
                    let (parts, body) = request.into_parts();
                    for (name, value) in parts.headers.iter() {
                        headers.lock().push((
                            name.as_str().to_string(),
                            value.to_str().unwrap_or_default().to_string(),
                        ));
                    }
                    let bytes = axum::body::to_bytes(body, 8 * 1024 * 1024).await.unwrap();
                    let parsed: serde_json::Value = serde_json::from_slice(&bytes).unwrap();

                    // The three things a live call refused, asserted here so a
                    // regression is a failing test rather than a 400 in
                    // production. `temperature` in particular: the endpoint
                    // rejects the parameter outright, and the runtime's own
                    // probe sets one.
                    assert!(
                        parsed.get("temperature").is_none(),
                        "the endpoint rejects temperature outright: {parsed}"
                    );
                    assert!(
                        parsed["instructions"].as_str().is_some_and(|i| !i.is_empty()),
                        "instructions are required and 400 without: {parsed}"
                    );
                    for tool in parsed["tools"].as_array().unwrap_or(&Vec::new()) {
                        assert!(
                            tool.get("function").is_none(),
                            "a nested tool is accepted and then never offered: {tool}"
                        );
                        assert!(
                            tool["name"].as_str().is_some(),
                            "a tool needs a flat name: {tool}"
                        );
                    }

                    calls.fetch_add(1, Ordering::SeqCst);
                    let script = decide(&parsed);
                    seen.lock().push(parsed);
                    ([("content-type", "text/event-stream")], render(&script)).into_response()
                }
            }
        }),
    );

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    Stub { backend: format!("http://{addr}"), seen, headers, calls }
}

struct Signed {
    runtime: Runtime,
    sink: Arc<RecordingSink>,
    ids: std::collections::HashMap<String, AgentId>,
    _dir: tempfile::TempDir,
}

impl Signed {
    fn id(&self, name: &str) -> AgentId {
        *self.ids.get(name).unwrap_or_else(|| panic!("no agent named {name}"))
    }

    fn ask(&self, name: &str, text: &str) -> guac_lib::domain::ids::RunId {
        self.runtime.send_from_human(self.id(name), text).unwrap()
    }

    /// Everything in one agent's channel that has words in it.
    fn texts(&self, name: &str) -> Vec<String> {
        self.runtime
            .store()
            .channel_messages(self.id(name), 50)
            .unwrap()
            .iter()
            .map(guac_lib::domain::envelope::Envelope::plain_text)
            .filter(|t| !t.is_empty())
            .collect()
    }

    /// Every part of every message, including the ones with no text.
    ///
    /// A guard refusal and a failed call are notices rather than prose, so the
    /// two failures worth reading a transcript for are exactly the ones
    /// `plain_text` returns nothing for.
    fn everything(&self, name: &str) -> String {
        self.runtime
            .store()
            .channel_messages(self.id(name), 50)
            .unwrap()
            .iter()
            .map(|m| format!("{:?}", m.parts))
            .collect::<Vec<_>>()
            .join("\n")
    }

    /// Waits for a run to go quiet, and insists it reports so exactly once.
    async fn settle(&self, run: guac_lib::domain::ids::RunId) {
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(20);
        loop {
            let settled = self.sink.count_of(
                |e| matches!(e, guac_lib::runtime::events::UiEvent::RunSettled { run_id, .. } if *run_id == run),
            );
            if settled > 0 {
                // Let the last write land before anything reads the store.
                tokio::time::sleep(std::time::Duration::from_millis(60)).await;
                assert_eq!(settled, 1, "RunSettled must fire exactly once per run");
                return;
            }
            assert!(std::time::Instant::now() < deadline, "run never settled");
            tokio::time::sleep(std::time::Duration::from_millis(20)).await;
        }
    }
}

/// A runtime already signed in to a subscription, pointed at the stub.
///
/// The sign-in is written as the file the store reads rather than performed,
/// because the flow that produces that file has its own tests and repeating it
/// here would make every one of these depend on a second stub.
fn signed_in(stub: &Stub, names: &[&str], plan: &str) -> Signed {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open(&dir.path().join("guac.db")).unwrap();

    let mut ids = std::collections::HashMap::new();
    for name in names {
        // A blank model, which is how an agent says "whatever the app is using".
        // The point of the test is that what it inherits is the subscription's
        // model and not the endpoint's.
        let mut d = draft(name, &["testing"]);
        d.model = String::new();
        ids.insert((*name).to_string(), store.create_agent(&d).unwrap().id);
    }

    let path = dir.path().join("subscription.json");
    std::fs::write(
        &path,
        serde_json::json!({
            "access_token": "access-token-for-tests",
            "refresh_token": "refresh-token-for-tests",
            "id_token": jwt(plan),
            "account_id": "acct-test",
            // Far enough out that nothing tries to refresh mid-test, which
            // would reach a sign-in service this stub does not run.
            "expires_at": chrono::Utc::now().timestamp() + 86_400,
            "email": "operator@example.com",
            "plan": plan,
        })
        .to_string(),
    )
    .unwrap();

    let subscription =
        Arc::new(Subscription::open_at(path, "http://127.0.0.1:1", stub.backend.clone()));
    assert!(subscription.is_signed_in(), "the written sign-in has to be readable");

    let config = AppConfig {
        version: guac_lib::config::CURRENT_VERSION,
        operator_name: String::new(),
        inference: InferenceConfig {
            provider: Provider::Chatgpt,
            // Deliberately left pointing at a URL and a model that would fail if
            // either were used. Nothing in a subscription turn may read them.
            base_url: "http://127.0.0.1:1/v1".into(),
            api_key: String::new(),
            default_model: "endpoint/model-that-would-fail".into(),
            subscription_model: "gpt-5.6-luna".into(),
            request_timeout_secs: 10,
            ..Default::default()
        },
        limits: GuardLimits::default(),
        e2b: Default::default(),
        kernel: Default::default(),
    };

    let sink = RecordingSink::new();
    let runtime = Runtime::new(
        store,
        LlmClient::new().unwrap().with_subscription(subscription),
        config,
        Workspace::new(dir.path().join("workspace")),
        FileStore::new(dir.path().join("files")),
        sink.clone(),
    );
    runtime.start_all().unwrap();

    Signed { runtime, sink, ids, _dir: dir }
}

/// An unsigned id token carrying the claims the store reads.
fn jwt(plan: &str) -> String {
    let claims = serde_json::json!({
        "email": "operator@example.com",
        "https://api.openai.com/auth": {
            "chatgpt_plan_type": plan,
            "chatgpt_account_id": "acct-test",
        },
    });
    let payload = guac_lib::e2b::encode(claims.to_string().as_bytes())
        .replace('+', "-")
        .replace('/', "_")
        .replace('=', "");
    format!("header.{payload}.signature")
}

// ---- tests ---------------------------------------------------------------

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_turn_runs_on_the_subscription_and_bills_what_it_spent() {
    let stub = serve(|_| Say::Text("Reporting in.".into())).await;
    let app = signed_in(&stub, &["Manager"], "pro");

    let run = app.ask("Manager", "Say hello.");
    app.settle(run).await;

    let texts = app.texts("Manager");
    assert!(texts.iter().any(|t| t.contains("Reporting in")), "got {texts:?}");

    // The tokens are counted and the call carries no price. Zero would read as a
    // free call in the usage view; a subscription call is unpriced, not free.
    let usage = app.runtime.store().usage_by_run(&[run]).unwrap();
    let tokens = *usage.get(&run).expect("the run was billed");
    assert_eq!(tokens.calls, 1);
    assert_eq!(tokens.prompt, u64::from(CALL_TOKENS.0));
    assert_eq!(tokens.completion, u64::from(CALL_TOKENS.1));
    assert_eq!(tokens.cost, None, "a subscription reports no price");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn the_call_carries_the_subscription_model_not_the_endpoint_one() {
    let stub = serve(|_| Say::Text("Done.".into())).await;
    let app = signed_in(&stub, &["Manager"], "pro");

    let run = app.ask("Manager", "Go.");
    app.settle(run).await;

    let body = stub.bodies().first().cloned().expect("the endpoint was called");
    // The app holds a model for each provider and everything downstream reads
    // one field. Resolving the wrong one is a turn the backend refuses by name.
    assert_eq!(body["model"], "gpt-5.6-luna");
    assert_eq!(body["store"], false, "a conversation lives in Guaca, not upstream");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn the_call_is_authorized_by_the_sign_in_and_billed_to_its_account() {
    let stub = serve(|_| Say::Text("Done.".into())).await;
    let app = signed_in(&stub, &["Manager"], "pro");

    let run = app.ask("Manager", "Go.");
    app.settle(run).await;

    assert_eq!(
        stub.header("authorization").as_deref(),
        Some("Bearer access-token-for-tests"),
        "the stored token has to reach the request"
    );
    // Dropping this header is a 403 from the backend, which reads as a rejected
    // sign-in rather than as a missing header.
    assert_eq!(stub.header("chatgpt-account-id").as_deref(), Some("acct-test"));
    assert_eq!(stub.header("originator").as_deref(), Some("codex_cli_rs"));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_tool_call_and_its_result_survive_the_round_trip() {
    // The whole loop: the model asks for a tool, the runtime runs it, and the
    // model is handed the result back in the shape this protocol wants. This is
    // the one that fails if `function_call_output` is filed under the wrong id.
    let stub = serve(|body| {
        if speaker(body) == "Manager" && !has_tool_result(body) {
            Say::SendTo { to: vec!["Chef".into()], text: "Please prep service.".into() }
        } else if speaker(body) == "Manager" {
            Say::Text("Chef has been told.".into())
        } else {
            Say::Text("On it.".into())
        }
    })
    .await;
    let app = signed_in(&stub, &["Manager", "Chef"], "pro");

    let run = app.ask("Manager", "Get service ready.");
    app.settle(run).await;

    // Chef heard about it, which only happens if the tool call was assembled
    // from the finished item and dispatched.
    let chef = app.texts("Chef");
    assert!(chef.iter().any(|t| t.contains("prep service")), "Chef never heard: {chef:?}");

    // And the second call to the Manager carried the result as an item rather
    // than as a `role: "tool"` message, matched to the call it answers.
    let manager_second = stub
        .bodies()
        .into_iter()
        .find(|b| speaker(b) == "Manager" && has_tool_result(b))
        .expect("the model was never handed its tool result back");
    let items = manager_second["input"].as_array().unwrap();

    let call = items.iter().find(|i| i["type"] == "function_call").expect("no call in the history");
    let output = items
        .iter()
        .find(|i| i["type"] == "function_call_output")
        .expect("no result in the history");
    assert_eq!(call["call_id"], output["call_id"], "a result filed elsewhere answers nothing");
    assert_eq!(call["name"], "send_message");
    // Not a role. Chat completions send `role: "tool"`; this protocol does not
    // have one, and sending it is a 400 about the role.
    assert!(output.get("role").is_none(), "{output}");

    assert!(stub.calls.load(Ordering::SeqCst) >= 3, "manager twice and chef once, at least");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn what_an_agent_already_said_goes_back_as_output_text() {
    let stub = serve(|body| {
        if has_tool_result(body) {
            Say::Text("Noted.".into())
        } else {
            Say::SendTo { to: vec!["Chef".into()], text: "Prep service.".into() }
        }
    })
    .await;
    let app = signed_in(&stub, &["Manager", "Chef"], "pro");

    let run = app.ask("Manager", "Go.");
    app.settle(run).await;

    // Chef's own turn replays the operator's message and its own answer. Text a
    // model wrote is `output_text` and text going to it is `input_text`, and
    // getting them the wrong way round is a 400 about the content type.
    let assistant_turns: Vec<serde_json::Value> = stub
        .bodies()
        .into_iter()
        .flat_map(|b| b["input"].as_array().cloned().unwrap_or_default())
        .filter(|i| i["role"] == "assistant")
        .collect();
    assert!(!assistant_turns.is_empty(), "no agent ever read its own turn back");
    for turn in assistant_turns {
        assert_eq!(turn["content"][0]["type"], "output_text", "{turn}");
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn reasoning_is_shown_and_never_written_down() {
    let stub = serve(|_| Say::Thinking {
        about: "The operator wants a status line.".into(),
        then: "All quiet.".into(),
    })
    .await;
    let app = signed_in(&stub, &["Manager"], "pro");

    let run = app.ask("Manager", "Status?");
    app.settle(run).await;

    let texts = app.texts("Manager");
    let all = texts.join("\n");
    assert!(all.contains("All quiet"), "the answer is missing: {texts:?}");
    // The summary is streamed to the operator and dropped. Persisting it is the
    // one thing `Token::Reasoning` exists to prevent.
    assert!(!all.contains("status line"), "reasoning reached the transcript: {texts:?}");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_finished_run_leaves_nothing_open() {
    // The accounting is provider-agnostic and has to stay that way: a new
    // transport that leaks a placeholder or double-reports a run fails here
    // rather than in the field. `settle` already insists RunSettled fired
    // exactly once, which is the other half of the same question.
    let stub = serve(|_| Say::Text("Working.".into())).await;
    let app = signed_in(&stub, &["Manager"], "pro");

    let run = app.ask("Manager", "Go.");
    app.settle(run).await;

    // The call really went to the subscription. Without this the rest of the
    // test passes just as well on a turn that never reached the backend at all.
    assert_eq!(stub.calls.load(Ordering::SeqCst), 1, "the turn did not reach the backend");
    assert_eq!(
        app.runtime.store().usage_by_run(&[run]).unwrap().get(&run).map(|t| t.calls),
        Some(1),
        "one call, billed once"
    );

    // Stopping a run that has already finished changes nothing, and asking is
    // how a leaked placeholder would show.
    assert!(!app.runtime.stop_run(run), "a settled run has nothing left to stop");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn testing_the_connection_reaches_the_subscription() {
    // The Test connection button, which is the first thing an operator presses
    // after signing in. It is also the one path that sets a temperature, and
    // this endpoint rejects that parameter outright rather than ignoring it, so
    // a regression here is a working sign-in that reports itself broken. The
    // stub asserts the absence; this is what makes it run.
    let stub = serve(|_| Say::Text("ok".into())).await;
    let app = signed_in(&stub, &["Manager"], "pro");

    let reported = app.runtime.probe(&app.runtime.config()).await.expect("the probe must succeed");

    assert_eq!(stub.calls.load(Ordering::SeqCst), 1);
    // What it reports has to be where the call went, not the endpoint field it
    // never touched: a URL the request never used reads as a misconfiguration.
    assert!(reported.contains("chatgpt.com/backend-api/codex"), "got {reported}");
    assert!(reported.contains("gpt-5.6-luna"), "got {reported}");
    assert!(!reported.contains("127.0.0.1"), "the unused endpoint leaked in: {reported}");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_turn_that_is_refused_says_to_sign_in_again() {
    // A 401 here is a sign-in to repeat, not a key to check. An operator sent to
    // the API key field they have never used does not get back on their feet.
    let app = signed_in(&refusing(401).await, &["Manager"], "pro");

    let run = app.ask("Manager", "Go.");
    app.settle(run).await;

    let all = app.everything("Manager");
    assert!(all.to_lowercase().contains("sign in"), "the way out has to be in the message: {all}");
    // And specifically not the other way out. A 401 on a pasted key means the
    // key is wrong; a 401 here means a sign-in has to be repeated, and sending
    // the operator to a field they have never used strands them.
    assert!(
        !all.to_lowercase().contains("paste"),
        "a refused sign-in must not read as a bad API key: {all}"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_model_the_plan_cannot_run_is_named_in_the_refusal() {
    let app = signed_in(&refusing(400).await, &["Manager"], "pro");

    let run = app.ask("Manager", "Go.");
    app.settle(run).await;

    let all = app.everything("Manager");
    // The backend's own sentence is the most useful thing an operator can read,
    // so it is passed through rather than replaced with a generic upstream error.
    assert!(all.contains("gpt-5.6-luna"), "which model has to survive: {all}");
}

/// A backend that refuses everything, in the two shapes this one refuses in.
async fn refusing(status: u16) -> Stub {
    let seen = Arc::new(Mutex::new(Vec::new()));
    let headers = Arc::new(Mutex::new(Vec::new()));
    let calls = Arc::new(AtomicUsize::new(0));

    let body = if status == 401 {
        r#"{"detail":"invalid or expired token"}"#.to_string()
    } else {
        serde_json::json!({
            "detail": "The 'gpt-5.6-luna' model is not supported when using Codex with a ChatGPT account."
        })
        .to_string()
    };

    let app = Router::new().route(
        "/responses",
        post({
            let calls = calls.clone();
            move || {
                let (calls, body) = (calls.clone(), body.clone());
                async move {
                    calls.fetch_add(1, Ordering::SeqCst);
                    (axum::http::StatusCode::from_u16(status).unwrap(), body).into_response()
                }
            }
        }),
    );

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    Stub { backend: format!("http://{addr}"), seen, headers, calls }
}

// ---- against the real endpoint -------------------------------------------

/// A real turn, on the operator's own subscription.
///
/// `#[ignore]`d for the reason the live evals are: it costs quota and cannot be
/// deterministic. It exists because everything above this line is a stub
/// agreeing with the shape this app believes the protocol has, and the failure
/// mode worth catching is that belief going stale. OpenAI can rename an event,
/// require a field, or retire a model slug, and every test above would still
/// pass while no agent could speak.
///
/// Run it after touching `llm/codex.rs`, or when a working sign-in stops
/// working:
///
/// ```sh
/// ./scripts/subscription.sh
/// ```
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "makes a real model call against the operator's ChatGPT plan"]
async fn a_real_subscription_answers_a_real_turn() {
    let Some(credentials) = live_credentials() else {
        panic!(
            "no ChatGPT sign-in to test with. Sign in from Guaca's Settings, or point \
             GUAC_SUBSCRIPTION_JSON at a credential file."
        );
    };

    let dir = tempfile::tempdir().unwrap();
    let store = Store::open(&dir.path().join("guac.db")).unwrap();
    let mut d = draft("Manager", &["testing"]);
    d.model = String::new();
    let manager = store.create_agent(&d).unwrap().id;

    // A copy, so a refresh performed here cannot rotate the token out from under
    // the running app.
    let path = dir.path().join("subscription.json");
    std::fs::copy(&credentials, &path).unwrap();

    let subscription = Arc::new(Subscription::open(path));
    assert!(subscription.is_signed_in(), "the credential file was not readable");
    let status = subscription.status();
    eprintln!("signed in as {} on a {} plan", status.email, status.plan);
    assert!(status.includes_codex, "a {} plan cannot call Codex", status.plan);

    let config = AppConfig {
        version: guac_lib::config::CURRENT_VERSION,
        operator_name: String::new(),
        inference: InferenceConfig {
            provider: Provider::Chatgpt,
            request_timeout_secs: 120,
            ..Default::default()
        },
        limits: GuardLimits::default(),
        e2b: Default::default(),
        kernel: Default::default(),
    };

    let sink = RecordingSink::new();
    let runtime = Runtime::new(
        store,
        LlmClient::new().unwrap().with_subscription(subscription),
        config,
        Workspace::new(dir.path().join("workspace")),
        FileStore::new(dir.path().join("files")),
        sink.clone(),
    );
    runtime.start_all().unwrap();

    // The probe first, because it is the one path that sets a temperature and
    // this endpoint refuses the parameter outright. It is also the button an
    // operator presses immediately after signing in.
    let reported = runtime
        .probe(&runtime.config())
        .await
        .unwrap_or_else(|err| panic!("Test connection failed against the real endpoint: {err}"));
    eprintln!("probe: {reported}");

    let app = Signed {
        runtime,
        sink,
        ids: [("Manager".to_string(), manager)].into_iter().collect(),
        _dir: dir,
    };

    let run = app.ask("Manager", "Reply with exactly: LIVE_OK");
    app.settle(run).await;

    let texts = app.texts("Manager");
    eprintln!("said: {texts:?}");
    assert!(
        texts.iter().any(|t| t.contains("LIVE_OK")),
        "the model did not answer through the subscription: {texts:?}"
    );

    // And the plan was billed, which is the half a reply alone does not prove.
    let usage = app.runtime.store().usage_by_run(&[run]).unwrap();
    let tokens = *usage.get(&run).expect("the run was billed");
    eprintln!("spent {} in, {} out", tokens.prompt, tokens.completion);
    assert!(tokens.prompt > 0 && tokens.completion > 0, "the provider reported no usage");
    assert_eq!(tokens.cost, None, "a subscription call must carry no price");
}

/// The credential file to test against, if there is one.
///
/// The app's own by default, so what is measured is what the operator is
/// actually running. Overridable so a copy can be tested without touching it.
fn live_credentials() -> Option<std::path::PathBuf> {
    if let Ok(named) = std::env::var("GUAC_SUBSCRIPTION_JSON") {
        let path = std::path::PathBuf::from(named);
        return path.exists().then_some(path);
    }
    let home = std::env::var("HOME").ok()?;
    let path = std::path::Path::new(&home)
        .join("Library/Application Support/com.madebywelch.guac/subscription.json");
    path.exists().then_some(path)
}
