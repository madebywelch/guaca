mod harness;

use std::collections::{HashMap, VecDeque};
use std::io::Write;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::Duration;

use axum::extract::Path;
use axum::http::header::{AUTHORIZATION, LOCATION, RETRY_AFTER};
use axum::http::{HeaderMap, Method, StatusCode, Uri};
use axum::response::{IntoResponse, Response};
use futures_util::{SinkExt, StreamExt};
use guac_lib::db::RelayState;
use guac_lib::domain::approval::ApprovalState;
use guac_lib::domain::now_ms;
use guac_lib::domain::place::{
    AdapterId, Card, CardKind, Choice, ChoiceStyle, HeardKey, Mark, OutsideUser, PlaceKey, Posted,
    RelayTag, Spot, ThreadKey, STALE_MS,
};
use guac_lib::places::slack::SlackAdapter;
use guac_lib::places::{Adapter, AdapterStatus, SendError};
use guac_lib::runtime::guard::GuardLimits;
use guac_lib::secrets::Secret;
use harness::{anyone_said, harness, has_tool_result, serve, Harness, Script, Stub};
use serde_json::{json, Value};
use tokio::sync::mpsc;
use tokio::sync::Notify;
use tokio_tungstenite::tungstenite::Message;

const BOT: &str = "xoxb-test-not-a-token";
const APP: &str = "xapp-test-not-a-token";
const TEAM: &str = "T1";
const ME: &str = "U1";
const BOT_USER: &str = "UBOT";

#[derive(Clone, Debug)]
struct Request {
    method: String,
    verb: Method,
    authorization: String,
    body: String,
}

impl Request {
    fn field(&self, name: &str) -> Option<String> {
        if let Ok(value) = serde_json::from_str::<Value>(&self.body) {
            return match &value[name] {
                Value::Null => None,
                Value::String(text) => Some(text.clone()),
                other => Some(other.to_string()),
            };
        }
        self.body.split('&').find_map(|pair| {
            let (key, value) = pair.split_once('=')?;
            (key == name).then(|| {
                percent_encoding::percent_decode_str(value).decode_utf8_lossy().into_owned()
            })
        })
    }

    fn json(&self) -> Value {
        serde_json::from_str(&self.body).unwrap_or(Value::Null)
    }
}

#[derive(Clone)]
enum Answer {
    Json(Value),
    Held(Arc<Notify>, Value),
    AppliedThenLost,
    Status(u16, Option<u64>),
    Redirect(String),
}

fn slack_error(code: &str) -> Answer {
    Answer::Json(json!({ "ok": false, "error": code }))
}

enum Push {
    Envelope(Value),
    Ping,
    Drop,
    Freeze,
}

#[derive(Clone)]
struct Slack {
    origin: String,
    socket_url: String,
    requests: Arc<Mutex<Vec<Request>>>,
    answers: Arc<Mutex<HashMap<String, VecDeque<Answer>>>>,
    acks: Arc<Mutex<Vec<String>>>,
    pongs: Arc<AtomicUsize>,
    sockets: Arc<Mutex<Vec<mpsc::UnboundedSender<Push>>>>,
    posted: Arc<Mutex<Vec<Value>>>,
}

impl Slack {
    async fn start() -> Self {
        let socket_listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let socket_url =
            format!("ws://127.0.0.1:{}/link", socket_listener.local_addr().unwrap().port());
        let api_listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let origin = format!("http://127.0.0.1:{}", api_listener.local_addr().unwrap().port());
        let slack = Slack {
            origin,
            socket_url,
            requests: Arc::default(),
            answers: Arc::default(),
            acks: Arc::default(),
            pongs: Arc::default(),
            sockets: Arc::default(),
            posted: Arc::default(),
        };
        let api = slack.clone();
        let posts = Arc::new(AtomicUsize::new(0));
        let app = axum::Router::new().route(
            "/api/:method",
            axum::routing::any(
                move |Path(method): Path<String>,
                      verb: Method,
                      uri: Uri,
                      headers: HeaderMap,
                      body: String| {
                    let (api, posts) = (api.clone(), posts.clone());
                    async move {
                        let body = if verb == Method::GET {
                            assert!(body.is_empty());
                            uri.query().unwrap_or_default().to_string()
                        } else {
                            assert!(uri.query().is_none());
                            body
                        };
                        api.answer(method, verb, &headers, body, &posts).await
                    }
                },
            ),
        );
        tokio::spawn(async move { axum::serve(api_listener, app).await.unwrap() });
        let sockets = slack.clone();
        tokio::spawn(async move {
            while let Ok((stream, _)) = socket_listener.accept().await {
                let Ok(socket) = tokio_tungstenite::accept_async(stream).await else { continue };
                tokio::spawn(sockets.clone().hold(socket));
            }
        });
        slack
    }

    async fn answer(
        &self,
        method: String,
        verb: Method,
        headers: &HeaderMap,
        body: String,
        posts: &AtomicUsize,
    ) -> Response {
        let authorization = headers
            .get(AUTHORIZATION)
            .and_then(|value| value.to_str().ok())
            .unwrap_or_default()
            .to_string();
        let request = Request { method: method.clone(), verb, authorization, body };
        self.requests.lock().unwrap().push(request.clone());
        let scripted = self.answers.lock().unwrap().get_mut(&method).and_then(VecDeque::pop_front);
        let answer = scripted.unwrap_or_else(|| {
            Answer::Json(match method.as_str() {
                "apps.connections.open" => json!({ "ok": true, "url": self.socket_url }),
                "auth.test" => json!({ "ok": true, "user_id": BOT_USER }),
                "chat.postMessage" => self.apply_post(&request, posts),
                "conversations.history" | "conversations.replies" => {
                    let channel = request.field("channel");
                    let thread = request.field("ts");
                    let posted = self.posted.lock().unwrap();
                    let messages: Vec<_> = posted
                        .iter()
                        .filter(|message| {
                            message["channel"].as_str() == channel.as_deref()
                                && message["thread_ts"].as_str() == thread.as_deref()
                        })
                        .collect();
                    json!({ "ok": true, "messages": messages })
                }
                _ => json!({ "ok": true }),
            })
        });
        match answer {
            Answer::Json(value) => axum::Json(value).into_response(),
            Answer::Held(gate, value) => {
                gate.notified().await;
                axum::Json(value).into_response()
            }
            Answer::AppliedThenLost => {
                assert_eq!(request.method, "chat.postMessage");
                self.apply_post(&request, posts);
                let body = futures_util::stream::iter([
                    Ok(axum::body::Bytes::from_static(b"{")),
                    Err(std::io::Error::other("scripted response loss")),
                ]);
                Response::new(axum::body::Body::from_stream(body))
            }
            Answer::Status(status, after) => {
                let mut response = StatusCode::from_u16(status).unwrap().into_response();
                if let Some(after) = after {
                    response.headers_mut().insert(RETRY_AFTER, after.to_string().parse().unwrap());
                }
                response
            }
            Answer::Redirect(to) => {
                (StatusCode::FOUND, [(LOCATION, format!("{}{to}", self.origin))]).into_response()
            }
        }
    }

    fn apply_post(&self, request: &Request, posts: &AtomicUsize) -> Value {
        let mut message = request.json();
        let ts = format!("1700000000.{:06}", posts.fetch_add(1, Ordering::SeqCst) + 1);
        message["ts"] = json!(ts);
        message["user"] = json!(BOT_USER);
        message["bot_id"] = json!("BTEST");
        let reply = json!({ "ok": true, "channel": message["channel"], "ts": ts });
        self.posted.lock().unwrap().push(message);
        reply
    }

    async fn hold(self, mut socket: tokio_tungstenite::WebSocketStream<tokio::net::TcpStream>) {
        let (push, mut pushed) = mpsc::unbounded_channel();
        self.sockets.lock().unwrap().push(push);
        if socket.send(Message::text(json!({ "type": "hello" }).to_string())).await.is_err() {
            return;
        }
        loop {
            tokio::select! {
                next = pushed.recv() => match next {
                    Some(Push::Envelope(envelope)) => {
                        if socket.send(Message::text(envelope.to_string())).await.is_err() {
                            return;
                        }
                    }
                    Some(Push::Ping) => {
                        if socket.send(Message::Ping(Default::default())).await.is_err() {
                            return;
                        }
                    }
                    Some(Push::Freeze) => {
                        while pushed.recv().await.is_some_and(|next| !matches!(next, Push::Drop)) {}
                        return;
                    }
                    Some(Push::Drop) | None => return,
                },
                frame = socket.next() => match frame {
                    Some(Ok(Message::Text(text))) => {
                        let ack: Value = serde_json::from_str(text.as_str()).unwrap_or(Value::Null);
                        if let Some(id) = ack["envelope_id"].as_str() {
                            self.acks.lock().unwrap().push(id.to_string());
                        }
                    }
                    Some(Ok(Message::Pong(_))) => {
                        self.pongs.fetch_add(1, Ordering::SeqCst);
                    }
                    Some(Ok(_)) => {}
                    _ => return,
                },
            }
        }
    }

    fn script(&self, method: &str, answers: impl IntoIterator<Item = Answer>) {
        self.answers.lock().unwrap().entry(method.to_string()).or_default().extend(answers);
    }

    fn requests(&self, method: &str) -> Vec<Request> {
        self.requests
            .lock()
            .unwrap()
            .iter()
            .filter(|request| request.method == method)
            .cloned()
            .collect()
    }

    fn acks(&self) -> Vec<String> {
        self.acks.lock().unwrap().clone()
    }

    async fn connected(&self, count: usize) {
        until(&format!("connection {count}"), || {
            (self.sockets.lock().unwrap().len() >= count).then_some(())
        })
        .await;
    }

    fn push(&self, envelope: Value) {
        let sockets = self.sockets.lock().unwrap();
        sockets.last().expect("a connection to push to").send(Push::Envelope(envelope)).unwrap();
    }

    fn drop_connection(&self) {
        let sockets = self.sockets.lock().unwrap();
        sockets.last().expect("a connection to drop").send(Push::Drop).unwrap();
    }

    async fn ping(&self) {
        let before = self.pongs.load(Ordering::SeqCst);
        self.sockets.lock().unwrap().last().unwrap().send(Push::Ping).unwrap();
        until("pong", || (self.pongs.load(Ordering::SeqCst) > before).then_some(())).await;
    }

    fn freeze(&self) {
        let sockets = self.sockets.lock().unwrap();
        sockets.last().expect("a connection to freeze").send(Push::Freeze).unwrap();
    }

    async fn acked(&self, id: &str) {
        until(&format!("ack of {id}"), || self.acks().contains(&id.to_string()).then_some(()))
            .await;
    }

    fn adapter(&self) -> Arc<SlackAdapter> {
        Arc::new(SlackAdapter::at(Secret::new(BOT), Secret::new(APP), self.origin.clone()))
    }

    fn assert_no_token_leaked(&self, errors: &[String]) {
        for request in self.requests.lock().unwrap().iter() {
            let token = if request.method == "apps.connections.open" { APP } else { BOT };
            let verb = if matches!(
                request.method.as_str(),
                "conversations.history" | "conversations.replies"
            ) {
                Method::GET
            } else {
                Method::POST
            };
            assert_eq!(request.verb, verb, "{}", request.method);
            assert_eq!(request.authorization, format!("Bearer {token}"), "{}", request.method);
            for leaked in [BOT, APP] {
                assert!(!request.body.contains(leaked), "{} body carried a token", request.method);
                assert!(!request.method.contains(leaked));
            }
        }
        let logs = logs();
        for leaked in [BOT, APP] {
            assert!(!logs.contains(leaked), "a log line carried a token");
            for error in errors {
                assert!(!error.contains(leaked), "an error carried a token: {error}");
                assert!(!error.contains("127.0.0.1"), "an error named an address: {error}");
            }
        }
    }
}

async fn until<T>(what: &str, check: impl FnMut() -> Option<T>) -> T {
    within(what, Duration::from_secs(10), check).await
}

async fn within<T>(what: &str, limit: Duration, mut check: impl FnMut() -> Option<T>) -> T {
    let deadline = tokio::time::Instant::now() + limit;
    while tokio::time::Instant::now() < deadline {
        if let Some(found) = check() {
            return found;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    panic!("{what} did not happen within {limit:?}");
}

fn slack_id() -> AdapterId {
    AdapterId("slack".into())
}

fn stamp(ms: i64) -> String {
    format!("{}.{:06}", ms / 1000, (ms % 1000) * 1000)
}

struct Running {
    h: Harness,
    stub: Stub,
}

impl Running {
    async fn new() -> Self {
        Self::scripted(|_| Script::Say("Got it.".into())).await
    }

    async fn scripted(script: impl Fn(&Value) -> Script + Clone + Send + Sync + 'static) -> Self {
        captured();
        let stub = serve(script).await;
        let h = harness(&stub, &["Pip"], GuardLimits::default());
        let started = h.runtime.start_all().unwrap();
        h.runtime.pick_up(&started, Default::default());
        Self { h, stub }
    }

    fn attach(&self, adapter: Arc<SlackAdapter>) {
        self.h.runtime.attach_adapter(adapter);
    }

    async fn linked(slack: &Slack) -> Self {
        let run = Self::new().await;
        run.link(slack).await;
        run
    }

    async fn link(&self, slack: &Slack) {
        self.attach(slack.adapter());
        slack.connected(1).await;
        let code = self.h.runtime.link_code(&slack_id(), self.h.id("Pip")).unwrap();
        slack.push(dm("link", &code, &stamp(now_ms())));
        slack.acked("link").await;
        assert_eq!(self.h.runtime.store().linked(&slack_id()).unwrap().unwrap().person.0, ME);
    }

    fn bind_channel(&self) {
        let pip = self.h.id("Pip");
        let group = self.h.runtime.store().get_agent(pip).unwrap().unwrap().group_id;
        self.h
            .runtime
            .store()
            .bind_place(&slack_id(), &spot("C1", None).place, group, pip, false)
            .unwrap();
    }

    fn runs(&self) -> Vec<(String, Option<String>)> {
        let conn = self.h.runtime.store().conn().unwrap();
        let mut stmt =
            conn.prepare("SELECT heard_key, thread FROM place_runs ORDER BY rowid").unwrap();
        stmt.query_map([], |row| Ok((row.get(0)?, row.get(1)?)))
            .unwrap()
            .collect::<Result<_, _>>()
            .unwrap()
    }

    fn fail_one_write(&self, heard_key: &str) {
        assert!(heard_key.chars().all(|c| c.is_ascii_alphanumeric() || "/.".contains(c)));
        self.h.runtime.store().conn().unwrap().execute_batch(&format!(
            "CREATE TRIGGER fail_test_place BEFORE INSERT ON place_runs
             WHEN NEW.heard_key = '{heard_key}' BEGIN SELECT RAISE(ABORT, 'scripted write failure'); END;"
        )).unwrap();
    }

    fn allow_writes(&self) {
        self.h
            .runtime
            .store()
            .conn()
            .unwrap()
            .execute_batch("DROP TRIGGER fail_test_place")
            .unwrap();
    }

    async fn error(&self) -> AdapterStatus {
        until("adapter error", || {
            self.h.runtime.adapter_status(&slack_id()).filter(|status| status.last_error.is_some())
        })
        .await
    }

    async fn relayed(&self) {
        until("every place run relayed", || {
            let (runs, pending): (i64, i64) = self
                .h
                .runtime
                .store()
                .conn()
                .unwrap()
                .query_row(
                    "SELECT COUNT(*), COUNT(*) FILTER (WHERE relayed=0) FROM place_runs",
                    [],
                    |row| Ok((row.get(0)?, row.get(1)?)),
                )
                .unwrap();
            (runs > 0 && pending == 0).then_some(())
        })
        .await;
    }

    async fn resume(&self, slack: &Slack) {
        assert!(self.h.runtime.detach_adapter(&slack_id()));
        self.attach(slack.adapter());
        slack.connected(2).await;
    }
}

impl Drop for Running {
    fn drop(&mut self) {
        self.h.runtime.detach_adapter(&slack_id());
    }
}

fn logs() -> String {
    String::from_utf8_lossy(&captured().lock().unwrap()).into_owned()
}

fn captured() -> &'static Arc<Mutex<Vec<u8>>> {
    static LOGS: OnceLock<Arc<Mutex<Vec<u8>>>> = OnceLock::new();
    LOGS.get_or_init(|| {
        let logs = Arc::new(Mutex::new(Vec::new()));
        let writer = logs.clone();
        let _ = tracing_subscriber::fmt()
            .with_max_level(tracing::Level::TRACE)
            .with_writer(move || Capture(writer.clone()))
            .try_init();
        logs
    })
}

struct Capture(Arc<Mutex<Vec<u8>>>);

impl Write for Capture {
    fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
        self.0.lock().unwrap().extend_from_slice(bytes);
        Ok(bytes.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

fn event(id: &str, event: Value) -> Value {
    json!({
        "type": "events_api",
        "envelope_id": id,
        "retry_attempt": 0,
        "payload": {
            "team_id": TEAM,
            "authorizations": [{ "user_id": BOT_USER, "is_bot": true }],
            "event": event,
        },
    })
}

fn message(channel: &str, channel_type: &str, user: &str, text: &str, ts: &str) -> Value {
    json!({ "type": "message", "channel": channel, "channel_type": channel_type, "user": user, "text": text, "ts": ts })
}

fn dm(id: &str, text: &str, ts: &str) -> Value {
    event(id, message("D1", "im", ME, text, ts))
}

fn spot(place: &str, thread: Option<&str>) -> Spot {
    Spot {
        place: PlaceKey(format!("{TEAM}/{place}")),
        thread: thread.map(|ts| ThreadKey(ts.into())),
    }
}

fn question() -> Card {
    Card {
        kind: CardKind::Question,
        from: Some("Pip".into()),
        text: "Which **one** ships?".into(),
        fields: vec![],
        ask: Some("ap_42".into()),
        choices: vec![
            Choice { id: "0".into(), label: "Backoff fix".into(), style: ChoiceStyle::Primary },
            Choice { id: "1".into(), label: "Rewrite".into(), style: ChoiceStyle::Plain },
        ],
        settled: None,
    }
}

fn reason(error: &SendError) -> String {
    match error {
        SendError::Retry { .. } | SendError::Ambiguous { .. } => String::new(),
        SendError::Refused { reason } | SendError::Gone { reason } => reason.clone(),
    }
}

#[tokio::test]
async fn an_event_that_fails_to_record_is_not_acknowledged_and_its_redelivery_is() {
    let slack = Slack::start().await;
    let run = Running::linked(&slack).await;
    let ts = stamp(now_ms());
    let key = format!("D1/{ts}");
    run.fail_one_write(&key);
    slack.push(dm("e1", "ship it", &ts));
    slack.push(event("e2", json!({ "type": "message", "channel": "D1", "bot_id": "B1" })));
    slack.acked("e2").await;
    assert!(!slack.acks().contains(&"e1".to_string()));
    assert!(run.runs().is_empty(), "the failed write did not commit a run");
    assert!(!run.h.runtime.store().heard_before(&format!("outside:slack:{key}")).unwrap());

    run.allow_writes();
    let mut redelivered = dm("e1", "ship it", &ts);
    redelivered["retry_attempt"] = json!(1);
    slack.push(redelivered);
    slack.acked("e1").await;
    assert_eq!(run.runs(), [(key, None)]);
    assert!(logs().contains("not acknowledged"));
    slack.assert_no_token_leaked(&[]);
}

#[tokio::test]
async fn a_revoked_token_stops_the_connection_with_a_way_forward_and_no_retry() {
    let slack = Slack::start().await;
    let run = Running::new().await;
    slack.script("apps.connections.open", [slack_error("token_revoked")]);
    run.attach(slack.adapter());
    let ended = run.error().await;
    assert!(ended.stopped);
    let error = ended.last_error.unwrap();
    assert!(error.contains("Reinstall"), "{error}");
    slack.script("chat.postMessage", [slack_error("invalid_auth")]);
    let refused = slack
        .adapter()
        .post(&spot("D1", None), &question(), &RelayTag("message:1".into()))
        .await
        .unwrap_err();
    assert!(matches!(refused, SendError::Refused { .. }), "{refused:?}");
    assert!(reason(&refused).contains("paste"));
    slack.assert_no_token_leaked(&[error, reason(&refused)]);
}

#[tokio::test]
async fn a_rate_limited_post_waits_as_long_as_slack_asks() {
    let slack = Slack::start().await;
    slack.script("chat.postMessage", [Answer::Status(429, Some(30))]);
    let limited = slack
        .adapter()
        .post(&spot("C1", Some("5.0")), &question(), &RelayTag("message:2".into()))
        .await;
    assert_eq!(limited, Err(SendError::Retry { after: Some(Duration::from_secs(30)) }));
    slack.script("chat.postMessage", [Answer::Status(503, None)]);
    let outage = slack
        .adapter()
        .post(&spot("C1", Some("5.0")), &question(), &RelayTag("message:2".into()))
        .await;
    assert_eq!(outage, Err(SendError::Ambiguous { after: None }));
    slack.assert_no_token_leaked(&[]);
}

#[tokio::test]
async fn a_post_slack_applied_with_no_response_is_found_and_not_sent_twice() {
    let slack = Slack::start().await;
    let run = Running::linked(&slack).await;
    slack.script("chat.postMessage", [Answer::AppliedThenLost]);
    slack.push(dm("ask", "hello", &stamp(now_ms())));
    slack.acked("ask").await;
    let posted = within("delivery recovery", Duration::from_secs(30), || {
        let store = run.h.runtime.store();
        let key: String = store
            .conn()
            .unwrap()
            .query_row("SELECT key FROM relay WHERE key LIKE 'message:%'", [], |row| row.get(0))
            .ok()?;
        store.relay_row(&key).unwrap().filter(|row| row.state == RelayState::Sent)?.posted
    })
    .await;
    assert_eq!(posted, Posted("D1/1700000000.000001".into()));
    assert_eq!(slack.requests("chat.postMessage").len(), 1);
    assert_eq!(slack.posted.lock().unwrap().len(), 1);
    assert_eq!(slack.requests("conversations.history").len(), 1);
    slack.assert_no_token_leaked(&[]);
}

#[tokio::test]
async fn a_post_with_an_unconfirmed_success_body_is_ambiguous() {
    let slack = Slack::start().await;
    slack.script(
        "chat.postMessage",
        [
            Answer::Json(json!({ "ok": true, "channel": "D1" })),
            Answer::Json(json!({ "channel": "D1", "ts": "1.000000" })),
            Answer::Json(json!({ "ok": true, "channel": "D2", "ts": "1.000000" })),
            slack_error("fatal_error"),
            Answer::Status(408, None),
        ],
    );
    let adapter = slack.adapter();
    for _ in 0..5 {
        assert_eq!(
            adapter.post(&spot("D1", None), &question(), &RelayTag("message:1".into())).await,
            Err(SendError::Ambiguous { after: None })
        );
    }
    slack.assert_no_token_leaked(&[]);
}

#[tokio::test]
async fn a_post_to_a_channel_the_app_is_not_in_is_gone() {
    let slack = Slack::start().await;
    slack.script("chat.postMessage", [slack_error("not_in_channel")]);
    let gone = slack
        .adapter()
        .post(&spot("C9", Some("5.0")), &question(), &RelayTag("message:3".into()))
        .await
        .unwrap_err();
    assert!(matches!(gone, SendError::Gone { .. }), "{gone:?}");
    assert!(reason(&gone).contains("/invite"));
    slack.assert_no_token_leaked(&[reason(&gone)]);
}

#[tokio::test]
async fn a_disconnect_or_a_dropped_socket_reconnects_and_keeps_hearing() {
    let slack = Slack::start().await;
    let run = Running::linked(&slack).await;
    slack.push(json!({ "type": "disconnect", "reason": "refresh_requested" }));
    slack.connected(2).await;
    let first = stamp(now_ms());
    slack.push(dm("e3", "after refresh", &first));
    slack.acked("e3").await;
    slack.drop_connection();
    slack.connected(3).await;
    let second = stamp(now_ms());
    slack.push(dm("e4", "after drop", &second));
    slack.acked("e4").await;
    assert_eq!(run.runs(), [(format!("D1/{first}"), None), (format!("D1/{second}"), None)]);
    assert_eq!(slack.requests("apps.connections.open").len(), 3);
    slack.assert_no_token_leaked(&[]);
}

#[tokio::test]
async fn a_socket_that_goes_silent_is_pinged_then_replaced() {
    let slack = Slack::start().await;
    let run = Running::new().await;
    run.attach(Arc::new(
        SlackAdapter::at(Secret::new(BOT), Secret::new(APP), slack.origin.clone())
            .with_quiet(Duration::from_millis(200)),
    ));
    slack.connected(1).await;
    slack.freeze();
    slack.connected(2).await;
    slack.push(dm("q1", "still there?", &stamp(now_ms())));
    slack.acked("q1").await;
    assert_eq!(
        slack.requests("chat.postEphemeral")[0].json()["text"],
        "This Guaca answers its owner."
    );
    assert!(logs().contains("stopped answering"));
    slack.assert_no_token_leaked(&[]);
}

#[tokio::test]
async fn reconnect_recovers_a_message_in_the_gap_before_acknowledging_buffered_events() {
    let slack = Slack::start().await;
    let run = Running::linked(&slack).await;
    let base = now_ms();
    let first = stamp(base);
    let gap = stamp(base + 1);
    let overlap = stamp(base + 2);
    let last = stamp(base + 3);
    slack.push(dm("before", "before the gap", &first));
    slack.acked("before").await;
    let gate = Arc::new(Notify::new());
    slack.script(
        "conversations.history",
        [Answer::Held(
            gate.clone(),
            json!({
                "ok": true, "messages": [
                    { "ts": overlap, "user": ME, "text": "overlap" },
                    { "ts": gap, "user": ME, "text": "during the gap" },
                    { "ts": first, "user": ME, "text": "before the gap" },
                ]
            }),
        )],
    );
    slack.drop_connection();
    slack.connected(2).await;
    until("history request", || {
        (!slack.requests("conversations.history").is_empty()).then_some(())
    })
    .await;
    slack.push(dm("overlap", "overlap", &overlap));
    slack.push(dm("after", "after the gap", &last));
    slack.ping().await;
    assert_eq!(slack.acks(), ["link", "before"]);
    gate.notify_one();
    slack.acked("after").await;
    assert_eq!(run.runs(), [first, gap, overlap, last].map(|ts| (format!("D1/{ts}"), None)));
    assert_eq!(slack.requests("apps.connections.open").len(), 2);
    slack.assert_no_token_leaked(&[]);
}

#[tokio::test]
async fn a_disabled_socket_mode_ends_the_run_with_what_to_turn_on() {
    let slack = Slack::start().await;
    let run = Running::new().await;
    run.attach(slack.adapter());
    slack.connected(1).await;
    slack.push(json!({ "type": "disconnect", "reason": "link_disabled" }));
    let ended = run.error().await;
    assert!(ended.stopped);
    let error = ended.last_error.unwrap();
    assert!(error.contains("Socket Mode"), "{error}");
    assert_eq!(slack.requests("apps.connections.open").len(), 1);
    slack.assert_no_token_leaked(&[error]);
}

#[tokio::test]
async fn catch_up_that_cannot_reach_the_oldest_page_reads_nothing_and_says_so() {
    let slack = Slack::start().await;
    let run = Running::linked(&slack).await;
    let base = now_ms();
    let endless = (0..50).map(|page| {
        Answer::Json(json!({ "ok": true, "has_more": true, "response_metadata": { "next_cursor": format!("c{page}") },
            "messages": [{ "type": "message", "ts": stamp(base - page), "user": ME, "text": "new" }] }))
    });
    slack.script("conversations.history", endless);
    run.resume(&slack).await;
    let failed = run.error().await;
    assert!(failed.stopped);
    assert!(run.runs().is_empty(), "the newest must not be heard past a gap they would hide");
    assert!(failed.last_error.as_deref().unwrap().contains("incomplete history"));
    slack.assert_no_token_leaked(&[failed.last_error.unwrap()]);
}

#[tokio::test]
async fn slack_refusing_channel_thread_access_is_not_a_missing_post_or_empty_catch_up() {
    let slack = Slack::start().await;
    let run = Running::linked(&slack).await;
    run.bind_channel();
    let root = stamp(now_ms() - 10_000);
    slack.push(event("root", message("C1", "channel", ME, "<@UBOT> parent", &root)));
    slack.acked("root").await;
    run.relayed().await;
    let tag = RelayTag("ask:ap_42".into());
    slack.script(
        "conversations.replies",
        [slack_error("not_allowed_token_type"), slack_error("missing_scope")],
    );
    let failed = slack.adapter().find(&spot("C1", Some(&root)), &tag).await.unwrap_err();
    assert!(matches!(failed, SendError::Refused { .. }));
    let other = stamp(now_ms());
    slack.script("conversations.history", [Answer::Json(json!({
        "ok": true, "messages": [{ "ts": other, "user": ME, "text": "not delivered on an incomplete sweep" }]
    }))]);
    run.resume(&slack).await;
    let stopped = run.error().await;
    assert!(stopped.stopped);
    assert_eq!(run.runs(), [(format!("C1/{root}"), Some(root))]);
    slack.assert_no_token_leaked(&[reason(&failed), stopped.last_error.unwrap()]);
}

#[tokio::test]
async fn an_incomplete_lookup_is_not_proof_that_a_post_was_never_sent() {
    let slack = Slack::start().await;
    let adapter = slack.adapter();
    let tag = RelayTag("ask:ap_42".into());
    let dm = spot("D1", None);
    let thread = spot("D1", Some("500.0"));
    slack.script("conversations.history", [
        Answer::Json(json!({ "ok": true, "messages": [], "response_metadata": { "next_cursor": "older" } })),
        Answer::Json(json!({ "ok": true, "messages": [{ "ts": "600.0", "metadata": { "event_type": "guaca_relay" } }] })),
        Answer::Json(json!({ "ok": true })),
    ]);
    slack.script(
        "conversations.replies",
        [Answer::Json(json!({ "ok": true, "messages": [], "has_more": true }))],
    );
    for to in [&dm, &thread, &dm, &dm] {
        assert!(adapter.find(to, &tag).await.is_err());
    }
    assert!(slack.requests("chat.postMessage").is_empty());
}

#[tokio::test]
async fn a_socket_that_closes_before_hello_ends_the_run_to_be_retried() {
    let slack = Slack::start().await;
    let run = Running::new().await;
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    tokio::spawn(async move {
        while let Ok((stream, _)) = listener.accept().await {
            drop(tokio_tungstenite::accept_async(stream).await);
        }
    });
    slack.script(
        "apps.connections.open",
        [Answer::Json(json!({
            "ok": true, "url": format!("ws://127.0.0.1:{port}/link")
        }))],
    );
    run.attach(slack.adapter());
    let ended = run.error().await;
    assert!(!ended.stopped);
    slack.connected(1).await;
    assert_eq!(slack.requests("apps.connections.open").len(), 2);
    slack.assert_no_token_leaked(&[ended.last_error.unwrap()]);
}

#[tokio::test]
async fn a_mention_and_its_message_are_one_heard_without_the_mention() {
    let slack = Slack::start().await;
    let run = Running::linked(&slack).await;
    run.bind_channel();
    let text = "<@UBOT> what changed?";
    let root = stamp(now_ms());
    slack.push(event(
        "m1",
        json!({ "type": "app_mention", "channel": "C1", "user": ME, "text": text, "ts": root }),
    ));
    slack.push(event("m2", message("C1", "channel", ME, text, &root)));
    slack.acked("m2").await;
    assert_eq!(run.runs(), [(format!("C1/{root}"), Some(root))]);
    assert!(slack.acks().contains(&"m1".to_string()));
    let requests = until("model request", || {
        let requests = run.stub.transcript.lock().clone();
        (!requests.is_empty()).then_some(requests)
    })
    .await;
    assert!(anyone_said(&requests[0], "what changed?"));
    assert!(!anyone_said(&requests[0], "<@UBOT>"));
    slack.assert_no_token_leaked(&[]);
}

#[tokio::test]
async fn unrelated_channel_text_never_becomes_an_operator_request() {
    let slack = Slack::start().await;
    let run = Running::linked(&slack).await;
    run.bind_channel();
    let root = stamp(now_ms());
    slack.push(event("chatter", message("C1", "channel", ME, "Message to the team", &root)));
    let mut stranger_thread = message("C1", "channel", ME, "Reply to the team", &stamp(now_ms()));
    stranger_thread["thread_ts"] = json!(root);
    slack.push(event("chatter-thread", stranger_thread));
    slack.acked("chatter-thread").await;
    assert!(run.runs().is_empty());
    assert_eq!(run.stub.calls.load(Ordering::SeqCst), 0);
    assert_eq!(slack.requests("chat.postEphemeral").len(), 1);

    let addressed = stamp(now_ms());
    slack.push(event("addressed", message("C1", "channel", ME, "<@UBOT> for Guaca", &addressed)));
    slack.acked("addressed").await;
    let ts = stamp(now_ms());
    let mut reply = message("C1", "channel", ME, "A follow-up for Guaca", &ts);
    reply["thread_ts"] = json!(addressed);
    slack.push(event("follow-up", reply));
    slack.acked("follow-up").await;
    assert_eq!(
        run.runs(),
        [
            (format!("C1/{addressed}"), Some(addressed.clone())),
            (format!("C1/{ts}"), Some(addressed)),
        ]
    );
    slack.assert_no_token_leaked(&[]);
}

#[tokio::test]
async fn bots_edits_deletions_and_the_bots_own_words_are_acknowledged_and_never_heard() {
    let slack = Slack::start().await;
    let run = Running::linked(&slack).await;
    run.bind_channel();
    let base = now_ms() - 10_000;
    let mut edited = message("D1", "im", ME, "edit noise", &stamp(base + 1));
    edited["subtype"] = json!("message_changed");
    let mut deleted = message("D1", "im", ME, "delete noise", &stamp(base + 2));
    deleted["subtype"] = json!("message_deleted");
    let mut bot = message("D1", "im", "U7", "from another bot", &stamp(base + 3));
    bot["bot_id"] = json!("B7");
    let own = message("C1", "channel", BOT_USER, "my own reply", &stamp(base + 4));
    let ts = stamp(base + 5);
    let root = stamp(base);
    let mut broadcast = message("C1", "channel", ME, "<@UBOT> also to channel", &ts);
    broadcast["subtype"] = json!("thread_broadcast");
    broadcast["thread_ts"] = json!(root);
    for (id, body) in [("b1", edited), ("b2", deleted), ("b3", bot), ("b4", own), ("b5", broadcast)]
    {
        slack.push(event(id, body));
    }
    slack.acked("b5").await;
    assert_eq!(slack.acks(), ["link", "b1", "b2", "b3", "b4", "b5"]);
    assert_eq!(run.runs(), [(format!("C1/{ts}"), Some(root))]);
    assert_eq!(
        slack.requests("chat.postEphemeral").len(),
        1,
        "bad input never reached the owner check"
    );
    slack.assert_no_token_leaked(&[]);
}

#[tokio::test]
async fn a_press_becomes_the_choice_it_names_and_is_acknowledged() {
    let slack = Slack::start().await;
    let run = Running::scripted(|body| {
        if has_tool_result(body) {
            Script::Say("Will do.".into())
        } else {
            Script::AskQuestion {
                question: "Which one ships?".into(),
                options: vec!["Backoff fix".into(), "Rewrite".into()],
            }
        }
    })
    .await;
    run.link(&slack).await;
    run.bind_channel();
    let root = stamp(now_ms());
    slack.push(event("ask", message("C1", "channel", ME, "<@UBOT> which one?", &root)));
    slack.acked("ask").await;
    let ask = run.h.awaited_request().await;
    let action = format!("ask:{ask}:1");
    let posted = until("card with the second choice", || {
        slack.requests("chat.postMessage").into_iter().map(|r| r.json()).find(|body| {
            body["blocks"].as_array().is_some_and(|blocks| {
                blocks.iter().any(|block| {
                    block["elements"].as_array().is_some_and(|items| {
                        items.iter().any(|button| button["action_id"].as_str() == Some(&action))
                    })
                })
            })
        })
    })
    .await;
    assert_eq!(posted["thread_ts"], root);
    let message_ts = until("recorded ask post", || {
        run.h
            .runtime
            .store()
            .relay_row(&format!("ask:{ask}"))
            .unwrap()
            .filter(|row| row.state == RelayState::Sent)
            .and_then(|row| row.posted)
    })
    .await
    .0
    .split_once('/')
    .unwrap()
    .1
    .to_string();
    slack.push(json!({
        "type": "interactive",
        "envelope_id": "p1",
        "payload": {
            "type": "block_actions",
            "team": { "id": TEAM },
            "user": { "id": ME },
            "container": { "type": "message", "channel_id": "C1", "message_ts": message_ts, "thread_ts": root },
            "actions": [{ "action_id": action, "action_ts": stamp(now_ms()), "value": "0" }],
        },
    }));
    slack.acked("p1").await;
    assert_ne!(
        run.h.runtime.store().get_approval(ask).unwrap().unwrap().state,
        ApprovalState::Pending
    );
    until("settled rewrite card", || {
        slack
            .requests("chat.update")
            .into_iter()
            .find(|request| request.body.contains("Answered: Rewrite"))
    })
    .await;
    slack.assert_no_token_leaked(&[]);
}

#[tokio::test]
async fn a_post_carries_its_tag_thread_and_buttons_and_find_locates_it_again() {
    let slack = Slack::start().await;
    let adapter = slack.adapter();
    let to = spot("C1", Some("600.000001"));
    let tag = RelayTag("ask:ap_42".into());
    let posted = adapter.post(&to, &question(), &tag).await.unwrap();
    let sent = slack.requests("chat.postMessage")[0].json();
    assert_eq!(sent["channel"], "C1");
    assert_eq!(sent["thread_ts"], "600.000001");
    assert_eq!(
        sent["metadata"],
        json!({ "event_type": "guaca_relay", "event_payload": { "tag": "ask:ap_42" } })
    );
    let actions = sent["blocks"]
        .as_array()
        .unwrap()
        .iter()
        .find(|block| block["type"] == "actions")
        .unwrap()
        .clone();
    assert_eq!(actions["elements"][0]["action_id"], "ask:ap_42:0");
    assert_eq!(actions["elements"][0]["style"], "primary");
    assert!(sent["text"].as_str().unwrap().contains("Which"));
    let ts = posted.0.split_once('/').unwrap().1.to_string();
    slack.script("conversations.replies", [
        Answer::Json(json!({ "ok": true, "messages": [
            { "ts": "600.000001", "user": ME, "text": "root" },
            { "ts": "600.000002", "bot_id": "B1", "text": "other", "metadata": { "event_type": "guaca_relay", "event_payload": { "tag": "message:9" } } },
        ], "has_more": true, "response_metadata": { "next_cursor": "n2" } })),
        Answer::Json(json!({ "ok": true, "messages": [
            { "ts": ts, "bot_id": "B1", "text": "Which", "metadata": { "event_type": "guaca_relay", "event_payload": { "tag": "ask:ap_42" } } },
        ] })),
    ]);
    assert_eq!(adapter.find(&to, &tag).await.unwrap(), Some(posted));
    let reads = slack.requests("conversations.replies");
    assert_eq!(reads[0].field("include_all_metadata").as_deref(), Some("true"));
    assert_eq!(reads[0].field("ts").as_deref(), Some("600.000001"));
    assert_eq!(reads[1].field("cursor").as_deref(), Some("n2"));
    assert_eq!(adapter.find(&spot("D1", None), &tag).await.unwrap(), None);
    assert_eq!(
        slack.requests("conversations.history")[0].field("include_all_metadata").as_deref(),
        Some("true")
    );
    slack.assert_no_token_leaked(&[]);
}

#[tokio::test]
async fn an_edit_trades_the_buttons_for_what_was_decided() {
    let slack = Slack::start().await;
    let mut settled = question();
    settled.settled = Some("Answered: Rewrite".into());
    slack
        .adapter()
        .edit(&spot("C1", Some("1.0")), &Posted("C1/700.000001".into()), &settled)
        .await
        .unwrap();
    let update = slack.requests("chat.update")[0].json();
    assert_eq!(update["channel"], "C1");
    assert_eq!(update["ts"], "700.000001");
    let blocks = update["blocks"].as_array().unwrap();
    assert!(blocks.iter().all(|block| block["type"] != "actions"));
    assert_eq!(blocks.last().unwrap()["elements"][0]["text"], "Answered: Rewrite");
    slack.assert_no_token_leaked(&[]);
}

#[tokio::test]
async fn a_later_mark_replaces_the_eyes_and_a_repeat_is_not_an_error() {
    let slack = Slack::start().await;
    let adapter = slack.adapter();
    let heard = HeardKey("D1/800.000001".into());
    let to = spot("D1", None);
    slack.script("reactions.add", [slack_error("already_reacted")]);
    adapter.mark(&heard, &to, Mark::Working).await.unwrap();
    assert!(slack.requests("reactions.remove").is_empty());
    slack.script("reactions.remove", [slack_error("no_reaction")]);
    adapter.mark(&heard, &to, Mark::Done).await.unwrap();
    adapter.mark(&heard, &to, Mark::NeedsYou).await.unwrap();
    adapter.mark(&heard, &to, Mark::Failed).await.unwrap();
    let added: Vec<String> = slack
        .requests("reactions.add")
        .iter()
        .map(|r| r.json()["name"].as_str().unwrap().to_string())
        .collect();
    assert_eq!(added, ["eyes", "white_check_mark", "raised_hand", "warning"]);
    let removed = slack.requests("reactions.remove");
    assert_eq!(removed.len(), 3);
    assert!(removed
        .iter()
        .all(|r| r.json()["name"] == "eyes" && r.json()["timestamp"] == "800.000001"));
    slack.script("reactions.add", [slack_error("channel_not_found")]);
    assert!(matches!(adapter.mark(&heard, &to, Mark::Done).await, Err(SendError::Gone { .. })));
    slack.assert_no_token_leaked(&[]);
}

#[tokio::test]
async fn a_told_line_reaches_only_that_person() {
    let slack = Slack::start().await;
    slack
        .adapter()
        .tell(
            &spot("C1", Some("900.0")),
            &OutsideUser("U9".into()),
            "This Guaca answers its owner.",
        )
        .await
        .unwrap();
    let told = slack.requests("chat.postEphemeral")[0].json();
    assert_eq!(told["channel"], "C1");
    assert_eq!(told["user"], "U9");
    assert_eq!(told["thread_ts"], "900.0");
    assert_eq!(told["text"], "This Guaca answers its owner.");
    assert!(slack.requests("chat.postMessage").is_empty());
    slack.assert_no_token_leaked(&[]);
}

#[tokio::test]
async fn a_redirect_is_never_followed() {
    let slack = Slack::start().await;
    slack.script("chat.postMessage", [Answer::Redirect("/api/chat.elsewhere".into())]);
    let refused = slack
        .adapter()
        .post(&spot("C1", None), &question(), &RelayTag("message:4".into()))
        .await
        .unwrap_err();
    assert_eq!(refused, SendError::Ambiguous { after: None });
    assert!(slack.requests("chat.elsewhere").is_empty());
    slack.assert_no_token_leaked(&[reason(&refused)]);
}

#[tokio::test]
async fn catch_up_reads_recent_replies_to_old_roots_and_paged_history_oldest_first() {
    let slack = Slack::start().await;
    let run = Running::linked(&slack).await;
    run.bind_channel();
    let base = now_ms();
    let root = stamp(base - STALE_MS - 100_000);
    let seed = stamp(base);
    let mut in_thread = message("C1", "channel", ME, "<@UBOT> first visit", &seed);
    in_thread["thread_ts"] = json!(root);
    slack.push(event("seed", in_thread));
    slack.acked("seed").await;
    run.relayed().await;
    let first = stamp(base + 1);
    let middle = stamp(base + 2);
    let last = stamp(base + 3);
    slack.script("conversations.history", [
        Answer::Json(json!({ "ok": true, "has_more": true, "response_metadata": { "next_cursor": "c2" }, "messages": [
            { "ts": last, "user": ME, "text": "<@UBOT> last", "reply_count": 1 },
            { "ts": stamp(base + 10), "user": ME, "text": "unrelated channel history", "reply_count": 1 },
        ] })),
        Answer::Status(429, Some(1)),
        Answer::Json(json!({ "ok": true, "messages": [
            { "ts": first, "user": ME, "text": "<@UBOT> first root since reconnect" },
        ] })),
    ]);
    slack.script(
        "conversations.replies",
        [Answer::Json(json!({ "ok": true, "messages": [
        { "ts": root, "user": ME, "text": "old parent", "thread_ts": root },
        { "ts": middle, "user": ME, "text": "new reply to the older parent", "thread_ts": root },
        { "ts": seed, "user": ME, "text": "already took part", "thread_ts": root },
    ] }))],
    );
    let began = tokio::time::Instant::now();
    run.resume(&slack).await;
    slack.push(event("barrier", json!({ "type": "message", "channel": "C1", "bot_id": "B1" })));
    slack.acked("barrier").await;
    assert!(began.elapsed() >= Duration::from_secs(1), "Retry-After was observed");
    assert_eq!(
        run.runs(),
        [
            (format!("C1/{seed}"), Some(root.clone())),
            (format!("C1/{first}"), Some(first)),
            (format!("C1/{middle}"), Some(root.clone())),
            (format!("C1/{last}"), Some(last)),
        ]
    );
    let reads: Vec<_> = slack
        .requests("conversations.history")
        .into_iter()
        .filter(|request| request.field("channel").as_deref() == Some("C1"))
        .collect();
    assert_eq!(reads.len(), 3);
    assert_eq!(reads[0].field("cursor"), None);
    assert_eq!(reads[2].field("cursor").as_deref(), Some("c2"));
    let replies = slack.requests("conversations.replies");
    assert_eq!(replies.len(), 1);
    assert_eq!(replies[0].field("ts").as_deref(), Some(root.as_str()));
    assert_eq!(replies[0].field("oldest"), reads[0].field("oldest"));
    slack.assert_no_token_leaked(&[]);
}

#[tokio::test]
async fn catch_up_stops_at_the_first_message_it_cannot_record() {
    let slack = Slack::start().await;
    let run = Running::linked(&slack).await;
    let first = stamp(now_ms());
    let second = stamp(now_ms());
    run.fail_one_write(&format!("D1/{first}"));
    slack.script(
        "conversations.history",
        [Answer::Json(json!({ "ok": true, "messages": [
        { "ts": second, "user": ME, "text": "second" },
        { "ts": first, "user": ME, "text": "first" },
    ] }))],
    );
    run.resume(&slack).await;
    let failed = run.error().await;
    assert!(!failed.stopped);
    assert!(run.runs().is_empty(), "nothing after an unrecorded message may be heard out of order");
    slack.assert_no_token_leaked(&[failed.last_error.unwrap()]);
}

#[tokio::test]
async fn catch_up_with_a_revoked_token_says_so_and_is_not_retried() {
    let slack = Slack::start().await;
    let run = Running::linked(&slack).await;
    slack.script("auth.test", [slack_error("invalid_auth")]);
    run.resume(&slack).await;
    let failed = run.error().await;
    assert!(failed.stopped);
    let error = failed.last_error.unwrap();
    assert!(error.contains("refused the token"));
    slack.assert_no_token_leaked(&[error]);
}
