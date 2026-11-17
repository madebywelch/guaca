mod harness;

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

use axum::response::IntoResponse;
use axum::routing::post;
use axum::Router;
use parking_lot::Mutex;

use guac_lib::config::{AppConfig, InferenceConfig, Provider};
use guac_lib::db::Store;
use guac_lib::domain::group::{CleanGroup, InferenceOverrides};
use guac_lib::domain::ids::AgentId;
use guac_lib::llm::openrouter::LlmClient;
use guac_lib::runtime::events::RecordingSink;
use guac_lib::runtime::guard::GuardLimits;
use guac_lib::runtime::{OnDisk, Runtime};
use guac_lib::subscription::Subscription;

use harness::{draft, frame};

const CALL_TOKENS: (u32, u32) = (120, 30);
const CACHED_TOKENS: u32 = 96;

#[derive(Debug, Clone)]
enum Say {
    Text(String),
    Thinking { about: String, then: String },
    SendTo { to: Vec<String>, text: String },
}

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
                "input_tokens_details": { "cached_tokens": CACHED_TOKENS },
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

struct Stub {
    backend: String,
    seen: Arc<Mutex<Vec<serde_json::Value>>>,
    headers: Arc<Mutex<Vec<(String, String)>>>,
    calls: Arc<AtomicUsize>,
}

impl Stub {
    fn bodies(&self) -> Vec<serde_json::Value> {
        self.seen.lock().clone()
    }

    fn header(&self, name: &str) -> Option<String> {
        self.headers(name).into_iter().next()
    }

    fn headers(&self, name: &str) -> Vec<String> {
        self.headers
            .lock()
            .iter()
            .filter(|(key, _)| key.eq_ignore_ascii_case(name))
            .map(|(_, value)| value.clone())
            .collect()
    }
}

fn speaker(body: &serde_json::Value) -> String {
    let instructions = body["instructions"].as_str().unwrap_or_default();
    instructions
        .split_once("You are ")
        .and_then(|(_, rest)| rest.split([',', '.', '\n']).next())
        .unwrap_or_default()
        .trim()
        .to_string()
}

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
    serve_refusing(0, decide).await
}

async fn serve_refusing<F>(refusals: usize, decide: F) -> Stub
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

                    let nth = calls.fetch_add(1, Ordering::SeqCst);
                    let script = decide(&parsed);
                    seen.lock().push(parsed);
                    if nth < refusals {
                        return (
                            axum::http::StatusCode::UNAUTHORIZED,
                            r#"{"detail":"Provided authentication token is expired."}"#,
                        )
                            .into_response();
                    }
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

struct Signin {
    issuer: String,
    refreshes: Arc<AtomicUsize>,
}

async fn signin_service(refusing: bool) -> Signin {
    let refreshes = Arc::new(AtomicUsize::new(0));

    let app = Router::new().route(
        "/oauth/token",
        post({
            let refreshes = refreshes.clone();
            move |body: String| {
                let refreshes = refreshes.clone();
                async move {
                    let parsed: serde_json::Value = serde_json::from_str(&body).unwrap();
                    assert_eq!(parsed["grant_type"], "refresh_token", "got {body}");
                    let nth = refreshes.fetch_add(1, Ordering::SeqCst) + 1;
                    if refusing {
                        return (
                            axum::http::StatusCode::BAD_REQUEST,
                            r#"{"error":"invalid_grant","error_description":"refresh token expired"}"#,
                        )
                            .into_response();
                    }
                    axum::Json(serde_json::json!({
                        "access_token": format!("refreshed-token-{nth}"),
                        "refresh_token": format!("refresh-token-{nth}"),
                    }))
                    .into_response()
                }
            }
        }),
    );

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    Signin { issuer: format!("http://{addr}"), refreshes }
}

struct Signed {
    runtime: Runtime,
    sink: Arc<RecordingSink>,
    ids: std::collections::HashMap<String, AgentId>,
    subscription: Arc<Subscription>,
    _dir: tempfile::TempDir,
}

impl Signed {
    fn id(&self, name: &str) -> AgentId {
        *self.ids.get(name).unwrap_or_else(|| panic!("no agent named {name}"))
    }

    fn ask(&self, name: &str, text: &str) -> guac_lib::domain::ids::RunId {
        self.runtime.send_from_human(self.id(name), text).unwrap()
    }

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

    async fn settle(&self, run: guac_lib::domain::ids::RunId) {
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(20);
        loop {
            let settled = self.sink.count_of(
                |e| matches!(e, guac_lib::runtime::events::UiEvent::RunSettled { run_id, .. } if *run_id == run),
            );
            if settled > 0 {
                tokio::time::sleep(std::time::Duration::from_millis(60)).await;
                assert_eq!(settled, 1, "RunSettled must fire exactly once per run");
                return;
            }
            assert!(std::time::Instant::now() < deadline, "run never settled");
            tokio::time::sleep(std::time::Duration::from_millis(20)).await;
        }
    }
}

fn signed_in(stub: &Stub, names: &[&str], plan: &str) -> Signed {
    signed_in_where(stub, names, plan, false, NO_SIGNIN_SERVICE)
}

const NO_SIGNIN_SERVICE: &str = "http://127.0.0.1:1";

fn signed_in_at(stub: &Stub, names: &[&str], plan: &str, signin: &Signin) -> Signed {
    signed_in_where(stub, names, plan, false, &signin.issuer)
}

fn signed_in_where(
    stub: &Stub,
    names: &[&str],
    plan: &str,
    by_the_group: bool,
    issuer: &str,
) -> Signed {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open(&dir.path().join("guac.db")).unwrap();

    let group = by_the_group.then(|| {
        store
            .create_group(&CleanGroup {
                name: "Subscribers".into(),
                inference: Some(InferenceOverrides {
                    provider: Some(Provider::Chatgpt),
                    ..Default::default()
                }),
                ..Default::default()
            })
            .unwrap()
            .id
    });

    let mut ids = std::collections::HashMap::new();
    for name in names {
        let mut d = draft(name, &["testing"]);
        d.model = String::new();
        d.group_id = group;
        ids.insert((*name).to_string(), store.create_agent(&d).unwrap().id);
    }

    let path = dir.path().join("subscription.json");
    std::fs::write(
        &path,
        serde_json::json!({
            "access_token": "first-token-for-tests",
            "refresh_token": "refresh-token-for-tests",
            "id_token": jwt(plan),
            "account_id": "acct-test",
            "expires_at": chrono::Utc::now().timestamp() + 86_400,
            "email": "operator@example.com",
            "plan": plan,
        })
        .to_string(),
    )
    .unwrap();

    let subscription = Arc::new(Subscription::open_at(path, issuer, stub.backend.clone()));
    assert!(subscription.is_signed_in(), "the written sign-in has to be readable");

    let config = AppConfig {
        version: guac_lib::config::CURRENT_VERSION,
        operator_name: String::new(),
        inference: InferenceConfig {
            provider: if by_the_group { Provider::Compatible } else { Provider::Chatgpt },
            base_url: "http://127.0.0.1:1/v1".into(),
            api_key: Default::default(),
            default_model: "endpoint/model-that-would-fail".into(),
            subscription_model: "gpt-5.6-luna".into(),
            request_timeout_secs: 10,
            ..Default::default()
        },
        limits: GuardLimits::default(),
        e2b: Default::default(),
        kernel: Default::default(),
        webhook: Default::default(),
        quick_actions: Vec::new(),
        widgets: Vec::new(),
        attention: Default::default(),
        delivery: Default::default(),
    };

    let sink = RecordingSink::new();
    let runtime = Runtime::new(
        store,
        LlmClient::new().unwrap().with_subscription(subscription.clone()),
        config,
        OnDisk::under(dir.path()),
        sink.clone(),
    );
    runtime.start_all().unwrap();

    Signed { runtime, sink, ids, subscription, _dir: dir }
}

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

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_turn_runs_on_the_subscription_and_bills_what_it_spent() {
    let stub = serve(|_| Say::Text("Reporting in.".into())).await;
    let app = signed_in(&stub, &["Manager"], "pro");

    let run = app.ask("Manager", "Say hello.");
    app.settle(run).await;

    let texts = app.texts("Manager");
    assert!(texts.iter().any(|t| t.contains("Reporting in")), "got {texts:?}");

    let usage = app.runtime.store().usage_by_run(&[run]).unwrap();
    let tokens = *usage.get(&run).expect("the run was billed");
    assert_eq!(tokens.calls, 1);
    assert_eq!(tokens.prompt, u64::from(CALL_TOKENS.0));
    assert_eq!(tokens.completion, u64::from(CALL_TOKENS.1));
    assert_eq!(tokens.cost, None, "a subscription reports no price");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_turn_is_keyed_to_its_agent_and_reports_what_the_cache_served() {
    let stub = serve(|_| Say::Text("Done.".into())).await;
    let app = signed_in(&stub, &["Manager"], "pro");

    let run = app.ask("Manager", "Go.");
    app.settle(run).await;

    let body = stub.bodies().first().cloned().expect("the endpoint was called");
    assert_eq!(body["prompt_cache_key"], app.id("Manager").to_string(), "{body}");

    let cached: Vec<(u32, u32)> =
        app.sink
            .snapshot()
            .into_iter()
            .filter_map(|e| match e {
                guac_lib::runtime::events::UiEvent::TokensUsed {
                    run_id, prompt, cached, ..
                } if run_id == run => Some((prompt, cached)),
                _ => None,
            })
            .collect();
    assert_eq!(cached, vec![(CALL_TOKENS.0, CACHED_TOKENS)]);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn the_call_carries_the_subscription_model_not_the_endpoint_one() {
    let stub = serve(|_| Say::Text("Done.".into())).await;
    let app = signed_in(&stub, &["Manager"], "pro");

    let run = app.ask("Manager", "Go.");
    app.settle(run).await;

    let body = stub.bodies().first().cloned().expect("the endpoint was called");
    assert_eq!(body["model"], "gpt-5.6-luna");
    assert_eq!(body["store"], false, "a conversation lives in Guaca, not upstream");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_group_can_spend_the_subscription_while_the_app_pays_with_a_key() {
    let stub = serve(|_| Say::Text("On the plan.".into())).await;
    let app = signed_in_where(&stub, &["Manager"], "pro", true, NO_SIGNIN_SERVICE);

    let run = app.ask("Manager", "Say hello.");
    app.settle(run).await;

    let texts = app.texts("Manager");
    assert!(texts.iter().any(|t| t.contains("On the plan")), "got {}", app.everything("Manager"));

    let body = stub.bodies().first().cloned().expect("the subscription was called");
    assert_eq!(body["model"], "gpt-5.6-luna", "the group inherits the app's subscription model");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn the_call_is_authorized_by_the_sign_in_and_billed_to_its_account() {
    let stub = serve(|_| Say::Text("Done.".into())).await;
    let app = signed_in(&stub, &["Manager"], "pro");

    let run = app.ask("Manager", "Go.");
    app.settle(run).await;

    assert_eq!(
        stub.header("authorization").as_deref(),
        Some("Bearer first-token-for-tests"),
        "the stored token has to reach the request"
    );
    assert_eq!(stub.header("chatgpt-account-id").as_deref(), Some("acct-test"));
    assert_eq!(stub.header("originator").as_deref(), Some("codex_cli_rs"));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_tool_call_and_its_result_survive_the_round_trip() {
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

    let chef = app.texts("Chef");
    assert!(chef.iter().any(|t| t.contains("prep service")), "Chef never heard: {chef:?}");

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
    assert!(!all.contains("status line"), "reasoning reached the transcript: {texts:?}");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_finished_run_leaves_nothing_open() {
    let stub = serve(|_| Say::Text("Working.".into())).await;
    let app = signed_in(&stub, &["Manager"], "pro");

    let run = app.ask("Manager", "Go.");
    app.settle(run).await;

    assert_eq!(stub.calls.load(Ordering::SeqCst), 1, "the turn did not reach the backend");
    assert_eq!(
        app.runtime.store().usage_by_run(&[run]).unwrap().get(&run).map(|t| t.calls),
        Some(1),
        "one call, billed once"
    );

    assert!(!app.runtime.stop_run(run), "a settled run has nothing left to stop");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn testing_the_connection_reaches_the_subscription() {
    let stub = serve(|_| Say::Text("ok".into())).await;
    let app = signed_in(&stub, &["Manager"], "pro");

    let reported = app.runtime.probe(&app.runtime.config()).await.expect("the probe must succeed");

    assert_eq!(stub.calls.load(Ordering::SeqCst), 1);
    assert!(reported.contains("chatgpt.com/backend-api/codex"), "got {reported}");
    assert!(reported.contains("gpt-5.6-luna"), "got {reported}");
    assert!(!reported.contains("127.0.0.1"), "the unused endpoint leaked in: {reported}");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_token_the_backend_has_stopped_accepting_is_replaced_mid_turn() {
    let signin = signin_service(false).await;
    let stub = serve_refusing(1, |_| Say::Text("Working on it.".into())).await;
    let app = signed_in_at(&stub, &["Manager"], "pro", &signin);

    let run = app.ask("Manager", "Go.");
    app.settle(run).await;

    let said = app.texts("Manager").join("\n");
    assert!(said.contains("Working on it."), "the turn has to survive the refusal: {said}");
    let all = app.everything("Manager");
    assert!(
        !all.to_lowercase().contains("sign in"),
        "a token the app can replace itself is not the operator's problem: {all}"
    );

    assert_eq!(signin.refreshes.load(Ordering::SeqCst), 1, "one refusal, one refresh");
    assert_eq!(stub.calls.load(Ordering::SeqCst), 2, "the refused call and the retry");

    let bodies = stub.bodies();
    assert_eq!(bodies[0], bodies[1], "the retry must not be a different question");
    let bearers = stub.headers("authorization");
    assert_eq!(bearers[0], "Bearer first-token-for-tests");
    assert_eq!(bearers[1], "Bearer refreshed-token-1", "the retry has to carry the new token");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_sign_in_that_cannot_be_refreshed_is_forgotten_and_says_so() {
    let signin = signin_service(true).await;
    let app = signed_in_at(&refusing(401).await, &["Manager"], "pro", &signin);

    let run = app.ask("Manager", "Go.");
    app.settle(run).await;

    assert!(signin.refreshes.load(Ordering::SeqCst) >= 1, "the refresh token has to be tried");
    assert!(!app.subscription.is_signed_in(), "a refused sign-in must not be kept");

    let all = app.everything("Manager");
    assert!(all.to_lowercase().contains("sign in"), "the way out has to be in the message: {all}");
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
    assert!(all.contains("gpt-5.6-luna"), "which model has to survive: {all}");
    assert!(!all.contains("own setting"), "{all}");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn an_endpoint_model_on_the_agent_is_never_sent_to_the_subscription() {
    let stub = serve(|_| Say::Text("Cloning it now.".into())).await;
    let app = signed_in(&stub, &["Engineer"], "pro");
    let card = app.runtime.store().get_agent(app.id("Engineer")).unwrap().unwrap();
    let mut own = draft("Engineer", &["testing"]);
    own.model = "xiaomi/mimo-v2.6-pro".into();
    own.group_id = Some(card.group_id);
    app.runtime.store().update_agent(card.id, &own).unwrap();

    let run = app.ask("Engineer", "Clone the repo.");
    app.settle(run).await;

    let body = stub.bodies().first().cloned().expect("the subscription was called");
    assert_eq!(body["model"], "gpt-5.6-luna", "the group's subscription model runs");
    let said = app.texts("Engineer").join("\n");
    assert!(said.contains("Cloning it now."), "{}", app.everything("Engineer"));
    let kept = app.runtime.store().get_agent(card.id).unwrap().unwrap();
    assert_eq!(kept.model, "xiaomi/mimo-v2.6-pro");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_model_an_agent_names_for_itself_is_refused_with_where_it_was_named() {
    let app = signed_in(&refusing(400).await, &["Engineer"], "pro");
    let card = app.runtime.store().get_agent(app.id("Engineer")).unwrap().unwrap();
    let mut own = draft("Engineer", &["testing"]);
    own.subscription_model = Some("gpt-5.6-luna".into());
    own.group_id = Some(card.group_id);
    app.runtime.store().update_agent(card.id, &own).unwrap();

    let run = app.ask("Engineer", "Go.");
    app.settle(run).await;

    let all = app.everything("Engineer");
    assert!(all.contains("gpt-5.6-luna"), "{all}");
    assert!(all.contains("Engineer's own setting, not its group's"), "{all}");
    assert!(all.contains("use the group default"), "the way on: {all}");
}

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
            subscription_model: std::env::var("GUAC_SUBSCRIPTION_MODEL")
                .unwrap_or_else(|_| guac_lib::llm::codex::DEFAULT_MODEL.into()),
            reasoning_effort: guac_lib::domain::effort::ReasoningEffort::Low,
            request_timeout_secs: 120,
            ..Default::default()
        },
        limits: GuardLimits::default(),
        e2b: Default::default(),
        kernel: Default::default(),
        webhook: Default::default(),
        quick_actions: Vec::new(),
        widgets: Vec::new(),
        attention: Default::default(),
        delivery: Default::default(),
    };

    let sink = RecordingSink::new();
    let runtime = Runtime::new(
        store,
        LlmClient::new().unwrap().with_subscription(subscription.clone()),
        config,
        OnDisk::under(dir.path()),
        sink.clone(),
    );
    runtime.start_all().unwrap();

    let reported = runtime
        .probe(&runtime.config())
        .await
        .unwrap_or_else(|err| panic!("Test connection failed against the real endpoint: {err}"));
    eprintln!("probe: {reported}");

    let app = Signed {
        runtime,
        sink,
        ids: [("Manager".to_string(), manager)].into_iter().collect(),
        subscription,
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

    let usage = app.runtime.store().usage_by_run(&[run]).unwrap();
    let tokens = *usage.get(&run).expect("the run was billed");
    eprintln!("spent {} in, {} out", tokens.prompt, tokens.completion);
    assert!(tokens.prompt > 0 && tokens.completion > 0, "the provider reported no usage");
    assert_eq!(tokens.cost, None, "a subscription call must carry no price");
}

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

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn effort_resolves_from_app_group_and_agent_on_every_turn() {
    use guac_lib::domain::effort::ReasoningEffort as Effort;
    let stub = serve(|_| Say::Text("Finished.".into())).await;
    let app = signed_in(&stub, &["Manager"], "pro");
    let mut config = AppConfig::default();
    config.inference.provider = Provider::Chatgpt;
    config.inference.reasoning_effort = Effort::Medium;
    app.runtime.set_config(config);
    let agent = app.runtime.store().get_agent(app.id("Manager")).unwrap().unwrap();
    let mut agent_draft = draft("Manager", &["testing"]);
    agent_draft.model.clear();
    for (group, own, expected) in [
        (None, None, Some("medium")),
        (Some(Effort::High), None, Some("high")),
        (Some(Effort::High), Some(Effort::Low), Some("low")),
        (Some(Effort::High), Some(Effort::Auto), None),
        (Some(Effort::High), None, Some("high")),
        (None, None, Some("medium")),
    ] {
        app.runtime
            .store()
            .update_group(
                agent.group_id,
                &CleanGroup {
                    name: "Effort crew".into(),
                    inference: Some(InferenceOverrides {
                        reasoning_effort: group,
                        ..Default::default()
                    }),
                    ..Default::default()
                },
            )
            .unwrap();
        agent_draft.reasoning_effort = own;
        app.runtime.store().update_agent(agent.id, &agent_draft).unwrap();
        let run = app.ask("Manager", "Reply with Finished.");
        app.settle(run).await;
        let bodies = stub.bodies();
        let body = bodies.last().unwrap();
        assert_eq!(body["reasoning"]["effort"].as_str(), expected);
        assert_eq!(body["model"], guac_lib::llm::codex::DEFAULT_MODEL);
    }
}
