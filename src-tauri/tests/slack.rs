use std::collections::{HashMap, VecDeque};
use std::io::Write;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::Duration;

use axum::extract::Path;
use axum::http::header::{AUTHORIZATION, LOCATION, RETRY_AFTER};
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use futures_util::{SinkExt, StreamExt};
use guac_lib::domain::place::{
    Card, CardKind, Choice, ChoiceStyle, Heard, HeardKey, Mark, OutsideUser, PlaceKey, Posted,
    RelayTag, Said, Spot, ThreadKey,
};
use guac_lib::places::slack::SlackAdapter;
use guac_lib::places::{Adapter, AdapterError, Ears, SendError, Unheard};
use guac_lib::secrets::Secret;
use serde_json::{json, Value};
use tokio::sync::mpsc;
use tokio_tungstenite::tungstenite::Message;

const BOT: &str = "xoxb-test-not-a-token";
const APP: &str = "xapp-test-not-a-token";
const TEAM: &str = "T1";
const ME: &str = "U1";
const BOT_USER: &str = "UBOT";

#[derive(Clone, Debug)]
struct Request {
    method: String,
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
    Status(u16, Option<u64>),
    Redirect(String),
}

fn slack_error(code: &str) -> Answer {
    Answer::Json(json!({ "ok": false, "error": code }))
}

enum Push {
    Envelope(Value),
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
    sockets: Arc<Mutex<Vec<mpsc::UnboundedSender<Push>>>>,
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
            sockets: Arc::default(),
        };
        let api = slack.clone();
        let posts = Arc::new(AtomicUsize::new(0));
        let app = axum::Router::new().route(
            "/api/:method",
            axum::routing::post(
                move |Path(method): Path<String>, headers: HeaderMap, body: String| {
                    let (api, posts) = (api.clone(), posts.clone());
                    async move { api.answer(method, &headers, body, &posts) }
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

    fn answer(
        &self,
        method: String,
        headers: &HeaderMap,
        body: String,
        posts: &AtomicUsize,
    ) -> Response {
        let authorization = headers
            .get(AUTHORIZATION)
            .and_then(|value| value.to_str().ok())
            .unwrap_or_default()
            .to_string();
        let request = Request { method: method.clone(), authorization, body };
        self.requests.lock().unwrap().push(request.clone());
        let scripted = self.answers.lock().unwrap().get_mut(&method).and_then(VecDeque::pop_front);
        let answer = scripted.unwrap_or_else(|| {
            Answer::Json(match method.as_str() {
                "apps.connections.open" => json!({ "ok": true, "url": self.socket_url }),
                "auth.test" => json!({ "ok": true, "user_id": BOT_USER }),
                "chat.postMessage" => json!({
                    "ok": true,
                    "channel": request.field("channel"),
                    "ts": format!("1700000000.{:06}", posts.fetch_add(1, Ordering::SeqCst) + 1),
                }),
                "conversations.history" | "conversations.replies" => {
                    json!({ "ok": true, "messages": [] })
                }
                _ => json!({ "ok": true }),
            })
        });
        match answer {
            Answer::Json(value) => axum::Json(value).into_response(),
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

async fn until<T>(what: &str, mut check: impl FnMut() -> Option<T>) -> T {
    for _ in 0..1000 {
        if let Some(found) = check() {
            return found;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    panic!("{what} did not happen within ten seconds");
}

#[derive(Clone, Default)]
struct Heards {
    heard: Arc<Mutex<Vec<Heard>>>,
    refusals: Arc<Mutex<VecDeque<bool>>>,
}

impl Heards {
    fn ears(&self) -> Ears {
        let heards = self.clone();
        Ears::new(move |heard| {
            let heards = heards.clone();
            Box::pin(async move {
                heards.heard.lock().unwrap().push(heard);
                match heards.refusals.lock().unwrap().pop_front() {
                    Some(true) => Err(Unheard { reason: "the store is busy".into() }),
                    _ => Ok(()),
                }
            })
        })
    }

    fn all(&self) -> Vec<Heard> {
        self.heard.lock().unwrap().clone()
    }

    fn refuse_next(&self) {
        self.refusals.lock().unwrap().push_back(true);
    }

    async fn count(&self, count: usize) -> Vec<Heard> {
        until(&format!("{count} heard"), || {
            let all = self.all();
            (all.len() >= count).then_some(all)
        })
        .await
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

async fn running(slack: &Slack) -> (Heards, tokio::task::JoinHandle<Result<(), AdapterError>>) {
    captured();
    let heards = Heards::default();
    let adapter = slack.adapter();
    let ears = heards.ears();
    let task = tokio::spawn(async move { adapter.run(ears).await });
    slack.connected(1).await;
    (heards, task)
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
        SendError::Retry { .. } => String::new(),
        SendError::Refused { reason } | SendError::Gone { reason } => reason.clone(),
    }
}

#[tokio::test]
async fn an_event_that_fails_to_record_is_not_acknowledged_and_its_redelivery_is() {
    let slack = Slack::start().await;
    let (heards, _task) = running(&slack).await;
    heards.refuse_next();
    slack.push(dm("e1", "ship it", "100.000001"));
    heards.count(1).await;
    slack.push(event("e2", json!({ "type": "message", "channel": "D1", "channel_type": "im", "bot_id": "B1", "text": "noise", "ts": "100.000002" })));
    slack.acked("e2").await;
    assert!(
        !slack.acks().contains(&"e1".to_string()),
        "an unrecorded event must not be acknowledged"
    );
    let mut redelivered = dm("e1", "ship it", "100.000001");
    redelivered["retry_attempt"] = json!(1);
    slack.push(redelivered);
    slack.acked("e1").await;
    let heard = heards.all();
    assert_eq!(heard.len(), 2);
    assert_eq!(heard[0], heard[1], "a redelivery is the same Heard, so the key drops the repeat");
    assert_eq!(heard[0].key, HeardKey("D1/100.000001".into()));
    assert!(logs().contains("not acknowledged"));
    slack.assert_no_token_leaked(&[]);
}

#[tokio::test]
async fn a_revoked_token_stops_the_connection_with_a_way_forward_and_no_retry() {
    let slack = Slack::start().await;
    captured();
    slack.script("apps.connections.open", [slack_error("token_revoked")]);
    let ended = slack.adapter().run(Heards::default().ears()).await.unwrap_err();
    assert!(!ended.retry);
    assert!(ended.reason.contains("Reinstall"), "{}", ended.reason);
    slack.script("chat.postMessage", [slack_error("invalid_auth")]);
    let refused = slack
        .adapter()
        .post(&spot("D1", None), &question(), &RelayTag("message:1".into()))
        .await
        .unwrap_err();
    assert!(matches!(refused, SendError::Refused { .. }), "{refused:?}");
    assert!(reason(&refused).contains("paste"));
    slack.assert_no_token_leaked(&[ended.reason, reason(&refused)]);
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
    assert_eq!(outage, Err(SendError::Retry { after: None }));
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
    let (heards, task) = running(&slack).await;
    slack.push(json!({ "type": "disconnect", "reason": "refresh_requested" }));
    slack.connected(2).await;
    slack.push(dm("e3", "after refresh", "200.000001"));
    slack.acked("e3").await;
    slack.drop_connection();
    slack.connected(3).await;
    slack.push(dm("e4", "after drop", "200.000002"));
    slack.acked("e4").await;
    let texts: Vec<Said> = heards.all().into_iter().map(|heard| heard.said).collect();
    assert_eq!(
        texts,
        vec![Said::Text { text: "after refresh".into() }, Said::Text { text: "after drop".into() }]
    );
    assert_eq!(slack.requests("apps.connections.open").len(), 3);
    assert!(!task.is_finished());
    slack.assert_no_token_leaked(&[]);
}

#[tokio::test]
async fn a_socket_that_goes_silent_is_pinged_then_replaced() {
    let slack = Slack::start().await;
    captured();
    let heards = Heards::default();
    let adapter = Arc::new(
        SlackAdapter::at(Secret::new(BOT), Secret::new(APP), slack.origin.clone())
            .with_quiet(Duration::from_millis(200)),
    );
    let ears = heards.ears();
    let _task = tokio::spawn(async move { adapter.run(ears).await });
    slack.connected(1).await;
    slack.freeze();
    slack.connected(2).await;
    slack.push(dm("q1", "still there?", "250.000001"));
    slack.acked("q1").await;
    assert_eq!(heards.all().len(), 1);
    assert!(logs().contains("stopped answering"));
    slack.assert_no_token_leaked(&[]);
}

#[tokio::test]
async fn a_disabled_socket_mode_ends_the_run_with_what_to_turn_on() {
    let slack = Slack::start().await;
    captured();
    let adapter = slack.adapter();
    let task = tokio::spawn(async move { adapter.run(Heards::default().ears()).await });
    slack.connected(1).await;
    slack.push(json!({ "type": "disconnect", "reason": "link_disabled" }));
    let ended =
        tokio::time::timeout(Duration::from_secs(2), task).await.unwrap().unwrap().unwrap_err();
    assert!(!ended.retry);
    assert!(ended.reason.contains("Socket Mode"), "{}", ended.reason);
    assert_eq!(slack.requests("apps.connections.open").len(), 1);
    slack.assert_no_token_leaked(&[ended.reason]);
}

#[tokio::test]
async fn catch_up_that_cannot_reach_the_oldest_page_reads_nothing_and_says_so() {
    let slack = Slack::start().await;
    captured();
    let endless = (0..50).map(|page| {
        Answer::Json(json!({ "ok": true, "has_more": true, "response_metadata": { "next_cursor": format!("c{page}") },
            "messages": [{ "type": "message", "ts": format!("{}.0", 1000 - page), "user": ME, "text": "new" }] }))
    });
    slack.script("conversations.history", endless);
    let heards = Heards::default();
    let failed = slack
        .adapter()
        .catch_up(&PlaceKey("T1/C1".into()), None, &heards.ears())
        .await
        .unwrap_err();
    assert!(!failed.retry);
    assert!(heards.all().is_empty(), "the newest must not be heard past a gap they would hide");
    slack.assert_no_token_leaked(&[failed.reason]);
}

#[tokio::test]
async fn slack_refusing_channel_thread_access_is_not_a_missing_post_or_empty_catch_up() {
    let slack = Slack::start().await;
    let adapter = slack.adapter();
    let root = "600.000001";
    let to = spot("C1", Some(root));
    let tag = RelayTag("ask:ap_42".into());
    slack.script(
        "conversations.replies",
        [slack_error("not_allowed_token_type"), slack_error("missing_scope")],
    );
    slack.script(
        "conversations.history",
        [Answer::Json(json!({
            "ok": true, "messages": [{ "ts": root, "user": ME, "text": "parent", "reply_count": 1 }]
        }))],
    );
    let failed = adapter.find(&to, &tag).await.unwrap_err();
    assert!(matches!(failed, SendError::Refused { .. }));
    let heard = Heards::default();
    let stopped = adapter.catch_up(&to.place, None, &heard.ears()).await.unwrap_err();
    assert!(!stopped.retry);
    assert!(heard.all().is_empty());
    slack.assert_no_token_leaked(&[reason(&failed), stopped.reason]);
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
    captured();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    tokio::spawn(async move {
        while let Ok((stream, _)) = listener.accept().await {
            drop(tokio_tungstenite::accept_async(stream).await);
        }
    });
    slack.script(
        "apps.connections.open",
        [Answer::Json(json!({ "ok": true, "url": format!("ws://127.0.0.1:{port}/link") }))],
    );
    let ended = slack.adapter().run(Heards::default().ears()).await.unwrap_err();
    assert!(ended.retry);
    slack.assert_no_token_leaked(&[ended.reason]);
}

#[tokio::test]
async fn a_mention_and_its_message_are_one_heard_without_the_mention() {
    let slack = Slack::start().await;
    let (heards, _task) = running(&slack).await;
    let text = "<@UBOT> what changed?";
    slack.push(event("m1", json!({ "type": "app_mention", "channel": "C1", "user": ME, "text": text, "ts": "300.000001" })));
    slack.push(event("m2", message("C1", "channel", ME, text, "300.000001")));
    slack.acked("m2").await;
    let heard = heards.all();
    assert_eq!(heard.len(), 2);
    assert_eq!(heard[0], heard[1]);
    assert_eq!(heard[0].key, HeardKey("C1/300.000001".into()));
    assert_eq!(heard[0].said, Said::Text { text: "what changed?".into() });
    assert_eq!(heard[0].spot, spot("C1", Some("300.000001")));
    assert!(!heard[0].direct);
    assert_eq!(heard[0].at_ms, 300_000);
    slack.assert_no_token_leaked(&[]);
}

#[tokio::test]
async fn bots_edits_deletions_and_the_bots_own_words_are_acknowledged_and_never_heard() {
    let slack = Slack::start().await;
    let (heards, _task) = running(&slack).await;
    let mut edited = message("D1", "im", ME, "", "400.000002");
    edited["subtype"] = json!("message_changed");
    edited["message"] = json!({ "user": ME, "text": "new words", "ts": "400.000001" });
    let mut deleted = message("D1", "im", ME, "", "400.000003");
    deleted["subtype"] = json!("message_deleted");
    let mut bot = message("D1", "im", "U7", "from another bot", "400.000004");
    bot["bot_id"] = json!("B7");
    let own = message("C1", "channel", BOT_USER, "my own reply", "400.000005");
    let mut broadcast = message("C1", "channel", ME, "also to channel", "400.000006");
    broadcast["subtype"] = json!("thread_broadcast");
    broadcast["thread_ts"] = json!("399.0");
    for (id, body) in [("b1", edited), ("b2", deleted), ("b3", bot), ("b4", own), ("b5", broadcast)]
    {
        slack.push(event(id, body));
    }
    slack.acked("b5").await;
    for id in ["b1", "b2", "b3", "b4"] {
        assert!(
            slack.acks().contains(&id.to_string()),
            "{id} must be acknowledged so Slack stops sending it"
        );
    }
    let heard = heards.all();
    assert_eq!(heard.len(), 1, "{heard:?}");
    assert_eq!(heard[0].said, Said::Text { text: "also to channel".into() });
    assert_eq!(heard[0].spot.thread, Some(ThreadKey("399.0".into())));
    slack.assert_no_token_leaked(&[]);
}

#[tokio::test]
async fn a_press_becomes_the_choice_it_names_and_is_acknowledged() {
    let slack = Slack::start().await;
    let (heards, _task) = running(&slack).await;
    slack.push(json!({
        "type": "interactive",
        "envelope_id": "p1",
        "payload": {
            "type": "block_actions",
            "team": { "id": TEAM },
            "user": { "id": ME },
            "container": { "type": "message", "channel_id": "C1", "message_ts": "500.000002", "thread_ts": "500.000001" },
            "actions": [{ "action_id": "ask:ap_42:1", "action_ts": "501.000001", "value": "1" }],
        },
    }));
    slack.acked("p1").await;
    let heard = heards.count(1).await;
    assert_eq!(heard[0].said, Said::Choice { ask: "ap_42".into(), choice: "1".into() });
    assert_eq!(heard[0].speaker, OutsideUser(ME.into()));
    assert_eq!(heard[0].spot, spot("C1", Some("500.000001")));
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
    assert!(matches!(refused, SendError::Refused { .. }), "{refused:?}");
    assert!(reason(&refused).contains("302"));
    assert!(slack.requests("chat.elsewhere").is_empty());
    slack.assert_no_token_leaked(&[reason(&refused)]);
}

#[tokio::test]
async fn catch_up_reads_every_page_and_thread_oldest_first_and_waits_out_a_limit() {
    let slack = Slack::start().await;
    captured();
    slack.script("conversations.history", [
        Answer::Json(json!({ "ok": true, "has_more": true, "response_metadata": { "next_cursor": "c2" }, "messages": [
            { "type": "message", "ts": "105.000001", "user": ME, "text": "five" },
            { "type": "message", "ts": "104.000001", "user": ME, "text": "four", "reply_count": 1, "thread_ts": "104.000001" },
            { "type": "message", "ts": "103.000001", "bot_id": "B1", "text": "a bot" },
        ] })),
        Answer::Status(429, Some(1)),
        Answer::Json(json!({ "ok": true, "response_metadata": { "next_cursor": "" }, "messages": [
            { "type": "message", "ts": "102.000001", "user": ME, "text": "<@UBOT> two" },
        ] })),
    ]);
    slack.script("conversations.replies", [Answer::Json(json!({ "ok": true, "messages": [
        { "type": "message", "ts": "104.000001", "user": ME, "text": "four", "thread_ts": "104.000001" },
        { "type": "message", "ts": "104.000002", "user": ME, "text": "in the thread", "thread_ts": "104.000001" },
        { "type": "message", "ts": "101.000001", "user": ME, "text": "already heard", "thread_ts": "104.000001" },
    ] }))]);
    let heards = Heards::default();
    let after = HeardKey("C1/101.000001".into());
    slack
        .adapter()
        .catch_up(&PlaceKey("T1/C1".into()), Some(&after), &heards.ears())
        .await
        .unwrap();
    let heard = heards.all();
    let order: Vec<&str> = heard.iter().map(|heard| heard.key.0.as_str()).collect();
    assert_eq!(order, ["C1/102.000001", "C1/104.000001", "C1/104.000002", "C1/105.000001"]);
    assert_eq!(heard[0].said, Said::Text { text: "two".into() });
    assert_eq!(heard[2].spot.thread, Some(ThreadKey("104.000001".into())));
    let reads = slack.requests("conversations.history");
    assert_eq!(reads.len(), 3);
    assert_eq!(reads[0].field("oldest").as_deref(), Some("101.000001"));
    assert_eq!(reads[0].field("cursor"), None);
    assert_eq!(reads[2].field("cursor").as_deref(), Some("c2"));
    assert_eq!(
        slack.requests("conversations.replies")[0].field("ts").as_deref(),
        Some("104.000001")
    );
    slack.assert_no_token_leaked(&[]);
}

#[tokio::test]
async fn catch_up_stops_at_the_first_message_it_cannot_record() {
    let slack = Slack::start().await;
    captured();
    slack.script(
        "conversations.history",
        [Answer::Json(json!({ "ok": true, "messages": [
        { "type": "message", "ts": "11.0", "user": ME, "text": "second" },
        { "type": "message", "ts": "10.0", "user": ME, "text": "first" },
    ] }))],
    );
    let heards = Heards::default();
    heards.refuse_next();
    let failed = slack
        .adapter()
        .catch_up(&PlaceKey("T1/D1".into()), None, &heards.ears())
        .await
        .unwrap_err();
    assert!(failed.retry);
    assert_eq!(
        heards.all().len(),
        1,
        "nothing after an unrecorded message may be heard out of order"
    );
    assert!(heards.all()[0].direct);
    slack.assert_no_token_leaked(&[failed.reason]);
}

#[tokio::test]
async fn catch_up_with_a_revoked_token_says_so_and_is_not_retried() {
    let slack = Slack::start().await;
    captured();
    slack.script("auth.test", [slack_error("invalid_auth")]);
    let failed = slack
        .adapter()
        .catch_up(&PlaceKey("T1/D1".into()), None, &Heards::default().ears())
        .await
        .unwrap_err();
    assert!(!failed.retry);
    assert!(failed.reason.contains("refused the token"));
    slack.assert_no_token_leaked(&[failed.reason]);
}
