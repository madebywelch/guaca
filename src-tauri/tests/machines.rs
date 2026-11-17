mod harness;

use std::collections::HashMap;
use std::panic::AssertUnwindSafe;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use parking_lot::Mutex;
use serde_json::{json, Value};
use std::sync::OnceLock;

use futures_util::FutureExt;
use guac_lib::domain::ids::RunId;
use guac_lib::runtime::events::{EventSink, UiEvent};
use guac_lib::runtime::guard::GuardLimits;

use harness::*;

#[derive(Debug, Clone)]
struct Browser {
    id: String,
    name: String,
    agent: String,
}

#[derive(Debug, Default)]
struct KernelBooks {
    profiles: Vec<String>,
    live: Vec<Browser>,
    made: Vec<Browser>,
    refused: Vec<String>,
}

impl KernelBooks {
    fn made_for(&self, agent: &str) -> usize {
        self.made.iter().filter(|browser| browser.agent == agent).count()
    }

    fn refused_for(&self, agent: &str) -> usize {
        self.refused.iter().filter(|refused| *refused == agent).count()
    }
}

fn browser_row(browser: &Browser) -> Value {
    json!({
        "session_id": browser.id,
        "name": browser.name,
        "cdp_ws_url": format!("ws://127.0.0.1:9222/devtools/browser/{}", browser.id),
        "browser_live_view_url": format!("https://{}.kernel.sh:8443/view", browser.id),
        "tags": { "guac": "true", "guac-agent": browser.agent },
    })
}

fn refused(code: &str, message: &str) -> Response {
    let status = match code {
        "conflict" => StatusCode::CONFLICT,
        _ => StatusCode::NOT_FOUND,
    };
    (status, Json(json!({ "code": code, "message": message }))).into_response()
}

async fn kernel_profile(
    State(books): State<Arc<Mutex<KernelBooks>>>,
    Json(body): Json<Value>,
) -> Response {
    let name = body["name"].as_str().unwrap_or_default().to_string();
    let mut books = books.lock();
    if books.profiles.contains(&name) {
        return refused("conflict", "profile name already exists in project");
    }
    books.profiles.push(name.clone());
    Json(json!({ "name": name })).into_response()
}

async fn kernel_create(
    State(books): State<Arc<Mutex<KernelBooks>>>,
    Json(body): Json<Value>,
) -> Response {
    let name = body["name"].as_str().unwrap_or_default().to_string();
    let agent = body["tags"]["guac-agent"].as_str().unwrap_or_default().to_string();
    let mut books = books.lock();
    if books.live.iter().any(|held| held.name == name) {
        books.refused.push(agent);
        return refused("conflict", "browser session name already exists in project");
    }
    let browser = Browser { id: format!("bx{}", books.made.len() + 1), name, agent };
    books.live.push(browser.clone());
    books.made.push(browser.clone());
    Json(browser_row(&browser)).into_response()
}

async fn kernel_get(
    State(books): State<Arc<Mutex<KernelBooks>>>,
    Path(id): Path<String>,
) -> Response {
    match books.lock().live.iter().find(|held| held.id == id) {
        Some(browser) => Json(browser_row(browser)).into_response(),
        None => refused("not_found", &format!("browser session '{id}' not found")),
    }
}

async fn kernel_list(
    State(books): State<Arc<Mutex<KernelBooks>>>,
    Query(query): Query<HashMap<String, String>>,
) -> Response {
    let agent = query.get("tags[guac-agent]").cloned();
    let rows: Vec<Value> = books
        .lock()
        .live
        .iter()
        .filter(|held| agent.as_deref().is_none_or(|wanted| held.agent == wanted))
        .map(browser_row)
        .collect();
    Json(rows).into_response()
}

async fn kernel_delete(
    State(books): State<Arc<Mutex<KernelBooks>>>,
    Path(id): Path<String>,
) -> StatusCode {
    let mut books = books.lock();
    let before = books.live.len();
    books.live.retain(|held| held.id != id);
    if books.live.len() == before {
        StatusCode::NOT_FOUND
    } else {
        StatusCode::NO_CONTENT
    }
}

#[derive(Debug, Default)]
struct E2bBooks {
    made: Vec<(String, String)>,
}

impl E2bBooks {
    fn made_for(&self, agent: &str) -> usize {
        self.made.iter().filter(|(_, made_for)| made_for == agent).count()
    }

    fn holds(&self, id: &str) -> bool {
        self.made.iter().any(|(made, _)| made == id)
    }
}

async fn e2b_create(
    State(books): State<Arc<Mutex<E2bBooks>>>,
    Json(body): Json<Value>,
) -> Response {
    let agent = body["metadata"]["guac-agent"].as_str().unwrap_or_default().to_string();
    let mut books = books.lock();
    let id = format!("sbx{}", books.made.len() + 1);
    books.made.push((id.clone(), agent));
    Json(json!({
        "sandboxID": id,
        "envdAccessToken": format!("envd-{id}"),
        "trafficAccessToken": format!("traffic-{id}"),
    }))
    .into_response()
}

async fn e2b_state(State(books): State<Arc<Mutex<E2bBooks>>>, Path(id): Path<String>) -> Response {
    if books.lock().holds(&id) {
        Json(json!({ "state": "running" })).into_response()
    } else {
        (StatusCode::NOT_FOUND, Json(json!({ "message": "sandbox not found" }))).into_response()
    }
}

async fn e2b_timeout() -> StatusCode {
    StatusCode::NO_CONTENT
}

struct Fleet {
    kernel: Arc<Mutex<KernelBooks>>,
    e2b: Arc<Mutex<E2bBooks>>,
}

static FLEET: OnceLock<Fleet> = OnceLock::new();

fn fleet() -> &'static Fleet {
    FLEET.get_or_init(|| {
        let (ready, started) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            let runtime = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .expect("a runtime for the scripted providers");
            runtime.block_on(async move {
                ready.send(start().await).expect("the test thread is waiting on this");
                std::future::pending::<()>().await;
            });
        });
        started.recv().expect("the scripted providers started")
    })
}

async fn start() -> Fleet {
    let kernel = Arc::new(Mutex::new(KernelBooks::default()));
    let e2b = Arc::new(Mutex::new(E2bBooks::default()));

    let kernel_app = Router::new()
        .route("/profiles", post(kernel_profile))
        .route("/browsers", post(kernel_create).get(kernel_list))
        .route("/browsers/:id", get(kernel_get).delete(kernel_delete))
        .with_state(kernel.clone());

    let e2b_app = Router::new()
        .route("/sandboxes", post(e2b_create))
        .route("/sandboxes/:id", get(e2b_state))
        .route("/sandboxes/:id/timeout", post(e2b_timeout))
        .with_state(e2b.clone());

    std::env::set_var("GUAC_KERNEL_API_BASE", listen(kernel_app).await);
    std::env::set_var("GUAC_E2B_API_BASE", listen(e2b_app).await);

    Fleet { kernel, e2b }
}

async fn listen(app: Router) -> String {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    format!("http://{addr}")
}

async fn quiet() -> Stub {
    serve(|_| Script::Say("nothing to say".into())).await
}

#[tokio::test]
async fn a_run_whose_settling_panics_still_releases_its_machines() {
    let stub = quiet().await;
    let h = harness(&stub, &["Scout"], GuardLimits::default());
    let run = RunId::new();
    for _ in 0..2 {
        h.sink.emit(UiEvent::RunSettled { run_id: run, steps_used: 0 });
    }
    let released = Arc::new(AtomicBool::new(false));
    let machines = {
        let released = released.clone();
        live::Machines::releasing(move || released.store(true, Ordering::SeqCst))
    };

    let settling =
        AssertUnwindSafe(live::settle_and_release(&h, run, 1, tokio::spawn(async {}), machines))
            .catch_unwind()
            .await;

    assert!(settling.is_err(), "a run that settled twice must fail its settle check");
    assert!(
        released.load(Ordering::SeqCst),
        "the settle check panicked and the machines this run started were never released"
    );
}

#[tokio::test]
async fn a_second_browse_in_one_turn_uses_the_browser_the_first_made() {
    let fleet = fleet();
    let stub = quiet().await;
    let h = harness_with_browser(&stub, &["Scout"], GuardLimits::default());

    let card = h.agent_named("Scout").unwrap();
    assert!(card.browser_id.is_none(), "a fresh agent is holding nothing");

    let (_, first) = h.runtime.ensure_browser(&card).await.expect("the first browse");
    let (_, second) = h.runtime.ensure_browser(&card).await.expect("the second browse");

    assert_eq!(
        first.id, second.id,
        "the second call made a different browser: a turn that opens two pages was signing in \
         twice and overwriting its own cookies"
    );
    let books = fleet.kernel.lock();
    let agent = card.id.to_string();
    assert_eq!(
        (books.made_for(&agent), books.refused_for(&agent)),
        (1, 0),
        "one browser, and nothing refused"
    );
    drop(books);
    assert_eq!(
        h.agent_named("Scout").unwrap().browser_id.as_deref(),
        Some(first.id.as_str()),
        "the row names the browser the agent is on"
    );
}

#[tokio::test]
async fn a_live_browser_no_row_points_at_is_adopted_rather_than_refused() {
    let fleet = fleet();
    let stub = quiet().await;
    let h = harness_with_browser(&stub, &["Stray"], GuardLimits::default());
    let card = h.agent_named("Stray").unwrap();

    let (_, orphan) = h.runtime.ensure_browser(&card).await.expect("the first browse");
    h.runtime.store().set_agent_browser(card.id, None).unwrap();

    let (_, adopted) = h.runtime.ensure_browser(&card).await.expect("the browse after the crash");

    assert_eq!(adopted.id, orphan.id, "the orphan is what the agent is put back on");
    assert!(
        !adopted.cdp_ws_url.is_empty(),
        "an adopted browser without its socket is one nothing can drive"
    );
    assert_eq!(
        fleet.kernel.lock().refused_for(&card.id.to_string()),
        1,
        "the conflict is what the adoption is a recovery from"
    );
    assert_eq!(
        h.agent_named("Stray").unwrap().browser_id.as_deref(),
        Some(orphan.id.as_str()),
        "and the row is written back, so the next turn does not have to find it again"
    );
}

#[tokio::test]
async fn a_second_command_in_one_turn_runs_on_the_machine_the_first_rented() {
    let fleet = fleet();
    let stub = quiet().await;
    let h = harness_with_computer(&stub, &["Hand"], GuardLimits::default());

    let card = h.agent_named("Hand").unwrap();
    assert!(card.sandbox_id.is_none(), "a fresh agent is holding nothing");

    let (_, first) = h.runtime.ensure_computer(&card).await.expect("the first command");
    let (_, second) = h.runtime.ensure_computer(&card).await.expect("the second command");

    assert_eq!(
        first.id, second.id,
        "the second call rented another machine: the provider allows it, charges for it, and the \
         first one keeps billing until the sweep finds it"
    );
    assert_eq!(fleet.e2b.lock().made_for(&card.name), 1, "one machine");
    assert_eq!(
        second.envd_token,
        format!("envd-{}", first.id),
        "and the token that reaches it is the one that machine was issued"
    );
}
