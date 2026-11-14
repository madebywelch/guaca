//! The push address: what reaches a person with every window closed, through
//! the real runtime with a scripted model and a stub receiver on loopback.
//!
//! Each scenario drives the cause, steps the dispatcher with a clock it is
//! handed, then steps the sender, and reads what the receiver was sent. No
//! test here, and nothing it starts, reaches anything but 127.0.0.1.
mod harness;

use std::collections::{HashMap, VecDeque};
use std::io::Write;
use std::sync::{Arc, Mutex, OnceLock};
use std::time::Duration;

use axum::http::{HeaderMap, StatusCode, Uri};
use guac_lib::config::{ConfigError, DeliveryConfig};
use guac_lib::domain::now_ms;
use guac_lib::domain::outbox::GATHER_MS;
use guac_lib::domain::routine::{Cadence, Trigger};
use guac_lib::domain::view::OperatorView;
use guac_lib::push::{Content, Format};
use guac_lib::runtime::guard::GuardLimits;
use harness::*;

/// The part of the address that is a password: anyone who knows a topic on a
/// public ntfy server can read it.
const TOPIC: &str = "guaca-topic-SECRET-5c1e";
const TOKEN: &str = "tk_SECRET_token_7a9b";

/// One request the receiver took.
#[derive(Debug, Clone)]
struct Seen {
    path: String,
    headers: HashMap<String, String>,
    body: String,
}

impl Seen {
    fn header(&self, name: &str) -> Option<&str> {
        self.headers.get(&name.to_ascii_lowercase()).map(String::as_str)
    }
}

/// A receiver on loopback that answers each POST with the next status it was
/// given, and 200 once it runs out.
struct Receiver {
    url: String,
    seen: Arc<Mutex<Vec<Seen>>>,
    answers: Arc<Mutex<VecDeque<u16>>>,
}

impl Receiver {
    fn seen(&self) -> Vec<Seen> {
        self.seen.lock().unwrap().clone()
    }

    fn answer(&self, statuses: &[u16]) {
        self.answers.lock().unwrap().extend(statuses);
    }
}

async fn receiver() -> Receiver {
    let seen = Arc::new(Mutex::new(Vec::new()));
    let answers = Arc::new(Mutex::new(VecDeque::new()));
    let (took, script) = (seen.clone(), answers.clone());
    let app = axum::Router::new().fallback(move |uri: Uri, headers: HeaderMap, body: String| {
        let (took, script) = (took.clone(), script.clone());
        async move {
            let headers = headers
                .iter()
                .map(|(name, value)| {
                    (name.as_str().to_string(), value.to_str().unwrap_or_default().to_string())
                })
                .collect();
            took.lock().unwrap().push(Seen { path: uri.path().to_string(), headers, body });
            let status = script.lock().unwrap().pop_front().unwrap_or(200);
            StatusCode::from_u16(status).unwrap()
        }
    });
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    Receiver { url: format!("http://127.0.0.1:{port}/{TOPIC}"), seen, answers }
}

fn deliver_to(h: &Harness, url: &str, change: impl FnOnce(&mut DeliveryConfig)) {
    h.runtime
        .change_config(|config| {
            config.delivery = DeliveryConfig {
                url: url.into(),
                token: TOKEN.into(),
                link_base: "https://box.example".into(),
                ..Default::default()
            };
            change(&mut config.delivery);
            Ok::<_, ConfigError>(())
        })
        .unwrap();
}

/// One outbox row, as much of it as these scenarios read.
#[derive(Debug, Clone)]
struct Row {
    id: String,
    destination: String,
    status: String,
    last_error: Option<String>,
}

fn rows(h: &Harness) -> Vec<Row> {
    let conn = h.runtime.store().conn().unwrap();
    let mut stmt = conn
        .prepare("SELECT id,destination,status,last_error FROM outbox ORDER BY created_at, id")
        .unwrap();
    stmt.query_map([], |row| {
        Ok(Row {
            id: row.get(0)?,
            destination: row.get(1)?,
            status: row.get(2)?,
            last_error: row.get(3)?,
        })
    })
    .unwrap()
    .collect::<Result<_, _>>()
    .unwrap()
}

fn pushes(h: &Harness) -> Vec<Row> {
    rows(h).into_iter().filter(|row| row.destination == "endpoint").collect()
}

/// Runs a routine for each agent and lets it finish, with nobody at a window.
async fn routines_finish(h: &Harness, names: &[&str]) {
    for name in names {
        let routine = h
            .runtime
            .store()
            .create_routine(
                h.id(name),
                "Morning check",
                "Check the morning.",
                Trigger::Clock(Cadence::Daily),
                Some(now_ms() + 24 * 60 * 60 * 1000),
                false,
            )
            .unwrap();
        let run = h.runtime.test_routine(&routine).unwrap();
        h.settle(run).await;
    }
}

/// Closes the gathering finished work waits in, and returns when that was.
fn gathering_closes(h: &Harness) -> i64 {
    let now = now_ms();
    h.runtime.dispatch_deliveries(now);
    let closes = now + GATHER_MS;
    h.runtime.dispatch_deliveries(closes);
    closes
}

fn done() -> impl Fn(&serde_json::Value) -> Script + Clone + Send + Sync + 'static {
    |body| Script::Say(format!("{} is done.", speaker(body)))
}

fn escalating() -> impl Fn(&serde_json::Value) -> Script + Clone + Send + Sync + 'static {
    |body| {
        let said = body["messages"]
            .as_array()
            .map(|m| m.iter().filter_map(|m| m["content"].as_str()).collect::<Vec<_>>().join("\n"))
            .unwrap_or_default();
        if said.contains("on the operator's desk now") || said.contains("You already had that up") {
            Script::Say("Flagged it. I did what I could without the deploy key.".into())
        } else {
            Script::Escalate("The deploy needs a key only you have.".into())
        }
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn finished_work_while_away_is_one_post_of_its_own_line_with_the_token() {
    let stub = serve(done()).await;
    let h = harness(&stub, &["R1"], GuardLimits::default());
    let address = receiver().await;
    deliver_to(&h, &address.url, |_| {});

    routines_finish(&h, &["R1"]).await;
    let closes = gathering_closes(&h);
    assert_eq!(h.runtime.push_deliveries(closes).await, 1);

    let seen = address.seen();
    assert_eq!(seen.len(), 1, "{seen:#?}");
    let post = &seen[0];
    assert_eq!(post.path, format!("/{TOPIC}"));
    assert_eq!(post.body, "R1 is done.");
    assert_eq!(post.header("Authorization"), Some(format!("Bearer {TOKEN}").as_str()));
    assert_eq!(post.header("Title"), Some("R1"));
    assert_eq!(post.header("Priority"), Some("3"));
    assert_eq!(post.header("Tags"), Some("guaca"));
    assert_eq!(post.header("Click"), Some("https://box.example"));

    let pushed = pushes(&h);
    assert_eq!(pushed.len(), 1, "{pushed:#?}");
    assert_eq!(pushed[0].status, "delivered");
    assert_eq!(post.header("X-Guaca-Delivery"), Some(pushed[0].id.as_str()));
    let report = h.runtime.push_report().unwrap();
    assert_eq!(report.last_sent_at, Some(closes));
    assert_eq!(report.last_error, None);

    // Sent once: the next pass finds nothing to send.
    assert_eq!(h.runtime.push_deliveries(closes + 60 * 60 * 1000).await, 0);
    assert_eq!(address.seen().len(), 1);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn what_needs_the_operator_is_pushed_at_once_and_says_so() {
    let stub = serve(escalating()).await;
    let h = harness(&stub, &["Analyst"], GuardLimits::default());
    let address = receiver().await;
    deliver_to(&h, &address.url, |_| {});

    let run = h.runtime.send_from_human(h.id("Analyst"), "Ship it.").unwrap();
    h.settle(run).await;
    let now = now_ms();
    h.runtime.dispatch_deliveries(now);
    assert_eq!(h.runtime.push_deliveries(now).await, 1, "the escalation, not the gathering");

    let post = &address.seen()[0];
    assert_eq!(post.header("Title"), Some("Needs you"));
    assert_eq!(post.header("Priority"), Some("4"));
    assert!(post.body.starts_with("Analyst is stuck."), "{}", post.body);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn nothing_is_pushed_while_a_window_has_focus() {
    let stub = serve(escalating()).await;
    let h = harness(&stub, &["Analyst", "Notes"], GuardLimits::default());
    let address = receiver().await;
    deliver_to(&h, &address.url, |_| {});
    let view = OperatorView { agent_id: Some(h.id("Notes")), ..Default::default() };
    h.runtime.report_window(view, true, Some("desk".into()));

    let run = h.runtime.send_from_human(h.id("Analyst"), "Ship it.").unwrap();
    h.settle(run).await;
    let now = now_ms();
    assert_eq!(h.runtime.dispatch_deliveries(now), 1, "the window is offered the escalation");
    assert_eq!(h.runtime.push_deliveries(now).await, 0);
    assert!(pushes(&h).is_empty(), "{:#?}", rows(&h));
    assert!(address.seen().is_empty());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_failed_attempt_is_tried_again_once_and_then_taken() {
    let stub = serve(done()).await;
    let h = harness(&stub, &["R1"], GuardLimits::default());
    let address = receiver().await;
    address.answer(&[500]);
    deliver_to(&h, &address.url, |_| {});

    routines_finish(&h, &["R1"]).await;
    let closes = gathering_closes(&h);
    assert_eq!(h.runtime.push_deliveries(closes).await, 0);
    let report = h.runtime.push_report().unwrap();
    assert_eq!(report.last_error.as_deref(), Some("500 Internal Server Error."));
    assert_eq!(report.retry_at, Some(closes + 30_000));

    assert_eq!(h.runtime.push_deliveries(closes + 29_999).await, 0, "not before its time");
    assert_eq!(address.seen().len(), 1);
    assert_eq!(h.runtime.push_deliveries(closes + 30_000).await, 1);

    let seen = address.seen();
    assert_eq!(seen.len(), 2, "one retry: {seen:#?}");
    assert_eq!(
        seen[0].header("X-Guaca-Delivery"),
        seen[1].header("X-Guaca-Delivery"),
        "a receiver can tell the second is the first again"
    );
    let report = h.runtime.push_report().unwrap();
    assert_eq!(report.last_error, None);
    assert_eq!(report.last_sent_at, Some(closes + 30_000));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_refused_token_fails_at_once_and_the_pane_says_so() {
    let stub = serve(done()).await;
    let h = harness(&stub, &["R1"], GuardLimits::default());
    let address = receiver().await;
    address.answer(&[401]);
    deliver_to(&h, &address.url, |_| {});

    routines_finish(&h, &["R1"]).await;
    let closes = gathering_closes(&h);
    assert_eq!(h.runtime.push_deliveries(closes).await, 0);

    let pushed = pushes(&h);
    assert_eq!(pushed[0].status, "failed");
    let report = h.runtime.push_report().unwrap();
    assert_eq!(report.last_error.as_deref(), Some("401 Unauthorized. Check the token."));
    assert_eq!(report.retry_at, None, "not tried again");
    assert_eq!(report.last_attempt_at, Some(closes));
    assert_eq!(report.last_sent_at, None);

    assert_eq!(h.runtime.push_deliveries(closes + 2 * 60 * 60 * 1000).await, 0);
    assert_eq!(address.seen().len(), 1);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn generic_content_says_only_that_something_finished() {
    let stub = serve(done()).await;
    let h = harness(&stub, &["R1"], GuardLimits::default());
    let address = receiver().await;
    deliver_to(&h, &address.url, |delivery| {
        delivery.content = Content::Generic;
        delivery.format = Format::Json;
    });

    routines_finish(&h, &["R1"]).await;
    let closes = gathering_closes(&h);
    assert_eq!(h.runtime.push_deliveries(closes).await, 1);

    let post = &address.seen()[0];
    assert!(!post.body.contains("is done"), "{}", post.body);
    assert!(!post.body.contains("R1"), "no name either: {}", post.body);
    let body: serde_json::Value = serde_json::from_str(&post.body).unwrap();
    assert_eq!(body["title"], "Something finished");
    assert_eq!(body["agent"], serde_json::Value::Null);
    assert_eq!(body["group"], serde_json::Value::Null);

    // Nor is the line kept anywhere for the push: what it says was decided
    // when it was written.
    let conn = h.runtime.store().conn().unwrap();
    let stored: String = conn
        .query_row("SELECT body FROM outbox WHERE destination='endpoint'", [], |row| row.get(0))
        .unwrap();
    assert!(!stored.contains("is done"), "{stored}");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_json_receiver_gets_one_object_with_every_field() {
    let stub = serve(done()).await;
    let h = harness(&stub, &["R1"], GuardLimits::default());
    let address = receiver().await;
    deliver_to(&h, &address.url, |delivery| delivery.format = Format::Json);

    routines_finish(&h, &["R1"]).await;
    let closes = gathering_closes(&h);
    assert_eq!(h.runtime.push_deliveries(closes).await, 1);

    let post = &address.seen()[0];
    assert_eq!(post.header("Content-Type"), Some("application/json"));
    assert_eq!(post.header("Authorization"), Some(format!("Bearer {TOKEN}").as_str()));
    let body: serde_json::Value = serde_json::from_str(&post.body).unwrap();
    let group = h.runtime.store().get_agent(h.id("R1")).unwrap().unwrap().group_id;
    let group = h.runtime.store().get_group(group).unwrap().unwrap().name;
    assert_eq!(body["id"], post.header("X-Guaca-Delivery").unwrap());
    assert_eq!(body["kind"], "outcome");
    assert_eq!(body["title"], "R1");
    assert_eq!(body["body"], "R1 is done.");
    assert_eq!(body["agent"], "R1");
    assert_eq!(body["group"], group.as_str());
    assert_eq!(body["status"], "done");
    assert_eq!(body["link"], "https://box.example");
    assert!(body["createdAt"].as_i64().is_some());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn three_finishing_together_are_one_post() {
    let stub = serve(done()).await;
    let names = ["R1", "R2", "R3"];
    let h = harness(&stub, &names, GuardLimits::default());
    let address = receiver().await;
    deliver_to(&h, &address.url, |_| {});

    routines_finish(&h, &names).await;
    let closes = gathering_closes(&h);
    assert_eq!(h.runtime.push_deliveries(closes).await, 1);

    let seen = address.seen();
    assert_eq!(seen.len(), 1, "{seen:#?}");
    assert_eq!(seen[0].header("Title"), Some("3 finished"));
    let lines: Vec<&str> = seen[0].body.lines().collect();
    assert_eq!(lines.len(), 3, "{}", seen[0].body);
    for name in names {
        assert!(lines.contains(&format!("{name}: {name} is done.").as_str()), "{lines:?}");
    }
    assert_eq!(pushes(&h).len(), 1);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_restart_between_attempts_picks_the_retry_up() {
    let stub = serve(done()).await;
    let h = harness(&stub, &["R1"], GuardLimits::default());
    let address = receiver().await;
    address.answer(&[503]);
    deliver_to(&h, &address.url, |_| {});

    routines_finish(&h, &["R1"]).await;
    let closes = gathering_closes(&h);
    assert_eq!(h.runtime.push_deliveries(closes).await, 0);

    let (h, _) = h.restart(Duration::from_secs(5)).await;
    assert_eq!(h.runtime.push_deliveries(closes + 30_000).await, 1, "resumed after the restart");
    let seen = address.seen();
    assert_eq!(seen.len(), 2);
    assert_eq!(seen[0].header("X-Guaca-Delivery"), seen[1].header("X-Guaca-Delivery"));
    assert_eq!(pushes(&h)[0].status, "delivered");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn nothing_is_sent_with_no_address() {
    let stub = serve(done()).await;
    let h = harness(&stub, &["R1"], GuardLimits::default());
    let address = receiver().await;

    routines_finish(&h, &["R1"]).await;
    let closes = gathering_closes(&h);
    assert_eq!(h.runtime.push_deliveries(closes).await, 0);
    assert!(pushes(&h).is_empty(), "{:#?}", rows(&h));
    assert!(address.seen().is_empty());
    assert_eq!(h.runtime.push_report().unwrap(), Default::default());
    let refused = h.runtime.test_push().await.unwrap_err();
    assert_eq!(refused, "No address is saved yet.");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_retry_is_dropped_once_the_operator_is_back_or_has_seen_it() {
    let stub = serve(escalating()).await;
    let h = harness(&stub, &["Analyst"], GuardLimits::default());
    let address = receiver().await;
    address.answer(&[500]);
    deliver_to(&h, &address.url, |_| {});

    let run = h.runtime.send_from_human(h.id("Analyst"), "Ship it.").unwrap();
    h.settle(run).await;
    let now = now_ms();
    h.runtime.dispatch_deliveries(now);
    assert_eq!(h.runtime.push_deliveries(now).await, 0, "refused once");

    // The operator comes back before it is tried again: a window shows them.
    let view = OperatorView { agent_id: None, ..Default::default() };
    h.runtime.report_window(view, true, Some("desk".into()));
    assert_eq!(h.runtime.push_deliveries(now + 30_000).await, 0);
    assert_eq!(address.seen().len(), 1);
    let dropped = pushes(&h);
    assert_eq!(
        (dropped[0].status.as_str(), dropped[0].last_error.as_deref()),
        ("suppressed", Some("operator present"))
    );

    // Seen is the other way a push is dropped: in the commit that saw it.
    let stub = serve(done()).await;
    let h = harness(&stub, &["R1"], GuardLimits::default());
    let address = receiver().await;
    address.answer(&[500]);
    deliver_to(&h, &address.url, |_| {});
    routines_finish(&h, &["R1"]).await;
    let closes = gathering_closes(&h);
    h.runtime.push_deliveries(closes).await;
    h.runtime.mark_outcomes_seen(None, Some(now_ms() + 1)).unwrap();
    assert_eq!(h.runtime.push_deliveries(closes + 30_000).await, 0);
    assert_eq!(address.seen().len(), 1);
    assert_eq!(pushes(&h)[0].last_error.as_deref(), Some("seen"));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_test_push_says_what_came_back() {
    let stub = serve(done()).await;
    let h = harness(&stub, &["R1"], GuardLimits::default());
    let address = receiver().await;
    deliver_to(&h, &address.url, |_| {});

    let said = h.runtime.test_push().await.unwrap();
    assert!(said.starts_with("Sent"), "{said}");
    let post = &address.seen()[0];
    assert_eq!(post.header("Title"), Some("Guaca"));
    assert_eq!(post.header("Authorization"), Some(format!("Bearer {TOKEN}").as_str()));
    assert!(pushes(&h).is_empty(), "a test writes no row");

    address.answer(&[403]);
    assert_eq!(h.runtime.test_push().await.unwrap_err(), "403 Forbidden. Check the token.");
}

// ---- the address and the token stay secret ------------------------------

/// Every log line any test in this binary writes, at every level of ours and
/// at debug for every library, the HTTP client included.
fn logs() -> &'static Arc<Mutex<Vec<u8>>> {
    static LOGS: OnceLock<Arc<Mutex<Vec<u8>>>> = OnceLock::new();
    LOGS.get_or_init(|| {
        let buffer = Arc::new(Mutex::new(Vec::new()));
        let writer = buffer.clone();
        let _ = tracing_subscriber::fmt()
            .with_env_filter(tracing_subscriber::EnvFilter::new("guac_lib=trace,debug"))
            .with_ansi(false)
            .with_writer(move || Lines(writer.clone()))
            .try_init();
        buffer
    })
}

struct Lines(Arc<Mutex<Vec<u8>>>);

impl Write for Lines {
    fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
        self.0.lock().unwrap().extend_from_slice(bytes);
        Ok(bytes.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn the_address_and_the_token_never_leave_the_host_beyond_a_hint() {
    let logs = logs();
    // An agent reads the settings and asks to change the address, and every
    // request the model is sent is kept to look through afterwards.
    let asked = Arc::new(Mutex::new(Vec::<String>::new()));
    let heard = asked.clone();
    let stub = serve(move |body| {
        heard.lock().unwrap().push(body.to_string());
        let results = body["messages"]
            .as_array()
            .map(|m| m.iter().filter(|msg| msg["role"] == "tool").count())
            .unwrap_or(0);
        match results {
            0 => Script::Settings(serde_json::json!({ "action": "read" })),
            1 => Script::Settings(serde_json::json!({
                "action": "update",
                "changes": { "delivery": { "url": "https://attacker.example/topic" } },
            })),
            _ => Script::Say("I cannot change where pushes go.".into()),
        }
    })
    .await;
    let h = harness(&stub, &["Helper"], GuardLimits::default());
    let address = receiver().await;
    address.answer(&[401]);
    deliver_to(&h, &address.url, |_| {});

    // A refused push, a refused test, and a test to an address nothing
    // listens on, which is the error a client library writes the address into.
    routines_finish(&h, &["Helper"]).await;
    let closes = gathering_closes(&h);
    h.runtime.push_deliveries(closes).await;
    address.answer(&[401]);
    let refused = h.runtime.test_push().await.unwrap_err();
    let closed = format!("http://127.0.0.1:1/{TOPIC}");
    deliver_to(&h, &closed, |_| {});
    let unreachable = h.runtime.test_push().await.unwrap_err();
    assert_eq!(unreachable, "Could not connect. Check the address.");

    let run = h.runtime.send_from_human(h.id("Helper"), "Where do pushes go?").unwrap();
    h.settle(run).await;
    assert_eq!(h.runtime.config().delivery.url, closed, "no agent moved it");

    let report = serde_json::to_string(&h.runtime.push_report().unwrap()).unwrap();
    let settings = serde_json::to_string(&h.runtime.config().redacted()).unwrap();
    let events = serde_json::to_string(&h.sink.snapshot()).unwrap();
    let stored: Vec<String> = rows(&h).into_iter().filter_map(|row| row.last_error).collect();
    let model = asked.lock().unwrap().join("\n");
    let logged = String::from_utf8_lossy(&logs.lock().unwrap()).to_string();
    assert!(logged.contains("did not take a push"), "the failure was logged: {logged}");
    assert!(settings.contains("...5c1e"), "the hint is there: {settings}");
    assert!(model.contains("operatorOnly"), "the agent read the settings");

    for (place, text) in [
        ("the pane's report", report),
        ("the settings a window reads", settings),
        ("an event", events),
        ("a refused test", refused),
        ("an unreachable test", unreachable),
        ("a stored error", stored.join("\n")),
        ("what the model was sent", model),
        ("a log line", logged),
    ] {
        assert!(!text.contains("SECRET"), "{place} carries the address or the token: {text}");
    }
}
