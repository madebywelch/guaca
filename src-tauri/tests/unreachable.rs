//! A workspace that runs while its account service does not exist.
//!
//! Self-hosting is a promise about an absent service. A Guaca that is installed
//! and never signed in has to open, take a turn and answer the settings pane
//! with nothing of guaca.bot's reachable; the one feature that genuinely needs
//! it — the connectors whose grant lives there — has to say so in a sentence
//! rather than wait.
//!
//! This suite is the promise as a test. Everything it does goes through the
//! daemon's own HTTP surface and event socket, because that is the host a
//! self-hoster runs, and every call it makes has a deadline: a workspace that
//! waits on the account service fails here by name and by clock rather than by
//! hanging the harness. That is the shape of the defect it is written against —
//! a boot that fetches connector metadata, a turn that renews a token in front
//! of every model call, or a panel that asks what the account holds before it
//! draws itself.
//!
//! `GUACA_ACCOUNT_ORIGIN` is the seam, and deliberately the only one: where the
//! service is, is a build decision rather than a setting (`docs/ACCOUNT.md`),
//! and the variable exists so the flow can be pointed at a Worker on this
//! machine. Here it is pointed at a port nothing listens on. It is process-wide,
//! so it is set once, before any workspace in this binary opens.
//!
//! Offline. The model is a scripted endpoint on loopback and nothing is spent.

#![cfg(feature = "server")]

mod harness;

use std::net::{SocketAddr, TcpStream};
use std::sync::OnceLock;
use std::time::{Duration, Instant};

use futures_util::StreamExt;
use serde_json::{json, Value};
use tokio_tungstenite::tungstenite::Message;
use tokio_tungstenite::{MaybeTlsStream, WebSocketStream};

use harness::{serve, Script};

const TOKEN: &str = "a-token-nobody-guessed";

/// How long any one call may take.
///
/// Generous for a loopback hop and a scripted model, which answer in
/// milliseconds; short enough that a call waiting on the account service fails
/// as this suite rather than as a harness timeout. Every refusal below is
/// measured against it, because "it eventually answered" is the bug.
const PATIENCE: Duration = Duration::from_secs(15);

/// The address every guaca.bot route is pointed at, chosen once for the binary.
///
/// A closed port below 1024, because the operating system never hands one of
/// those out for the ephemeral binds the rest of this gate makes. A port bound
/// and then dropped would be eligible for reuse by any other test daemon in the
/// run, and a service that came back halfway is a suite whose premise changed
/// under it. A connection there is refused at once rather than dropped, which is
/// the deterministic failure an absent service gives: a blackholed address would
/// turn this into a suite about timeouts that passes because they are long.
fn absent_service() -> &'static str {
    static ORIGIN: OnceLock<String> = OnceLock::new();
    ORIGIN.get_or_init(|| {
        let port = [1u16, 7, 9]
            .into_iter()
            .find(|port| {
                let addr = SocketAddr::from(([127, 0, 0, 1], *port));
                TcpStream::connect_timeout(&addr, Duration::from_secs(2)).is_err()
            })
            .expect("a loopback port below 1024 with nothing listening on it");
        let origin = format!("http://127.0.0.1:{port}");
        std::env::set_var("GUACA_ACCOUNT_ORIGIN", &origin);
        origin
    })
}

/// A daemon on a free port, with a workspace of its own.
///
/// `account` is written to the account store before the workspace opens, because
/// that is what a machine that signed in once already has: `Account::open` reads
/// the file at boot and never dials for it.
async fn workspace(account: Option<Value>) -> (SocketAddr, tempfile::TempDir) {
    let dir = tempfile::tempdir().expect("a temporary workspace");
    if let Some(account) = account {
        let config = dir.path().join("config");
        std::fs::create_dir_all(&config).expect("the settings directory");
        std::fs::write(config.join("account.json"), account.to_string()).expect("an account file");
    }

    let bound = guac_lib::server::bind(guac_lib::server::Settings {
        root: dir.path().to_path_buf(),
        // Port zero, so nothing here collides with a daemon the operator
        // happens to be running or with the other test in this binary.
        bind: "127.0.0.1:0".parse().expect("a loopback address"),
        token: TOKEN.to_string(),
        web: None,
        origin: None,
        updater: None,
    })
    .await
    .expect("the workspace opens");

    let addr = bound.addr;
    tokio::spawn(bound.serve());
    (addr, dir)
}

/// One command, as a client makes it, held to the deadline above.
async fn call(addr: SocketAddr, name: &str, args: Value) -> (u16, Value) {
    let request = reqwest::Client::new()
        .post(format!("http://{addr}/v1/call"))
        .bearer_auth(TOKEN)
        .json(&json!({ "name": name, "args": args }))
        .send();
    let response = tokio::time::timeout(PATIENCE, request).await.unwrap_or_else(|_| {
        panic!("{name} did not answer within {PATIENCE:?}; nothing may wait on guaca.bot")
    });
    let response = response.expect("the daemon answers");
    let status = response.status().as_u16();
    (status, response.json().await.expect("the answer is JSON"))
}

type Socket = WebSocketStream<MaybeTlsStream<tokio::net::TcpStream>>;

/// A client watching the workspace, connected before anything happens so no
/// event lands before there is somebody to hear it.
async fn watching(addr: SocketAddr) -> Socket {
    let (socket, _) =
        tokio_tungstenite::connect_async(format!("ws://{addr}/v1/events?token={TOKEN}"))
            .await
            .expect("the event socket opens");
    socket
}

/// The next message whose words carry `needle`, read off the event socket.
///
/// One loop for one wait rather than a `next_of_kind` per step, because the
/// events the middle of a turn produces would otherwise be consumed and dropped
/// by a loop that was not looking for them. What it saw is in every panic: a
/// turn that never reaches the model and one whose events never cross the
/// socket fail here, and the difference is the whole diagnosis.
async fn next_saying(socket: &mut Socket, needle: &str) -> Value {
    let deadline = Instant::now() + PATIENCE;
    let mut seen: Vec<String> = Vec::new();
    loop {
        let left = deadline.saturating_duration_since(Instant::now());
        assert!(
            !left.is_zero(),
            "no message said {needle:?} within {PATIENCE:?}; the socket carried {seen:?}"
        );
        let message = match tokio::time::timeout(left, socket.next()).await {
            Ok(Some(Ok(message))) => message,
            Ok(Some(Err(err))) => panic!("the event socket failed: {err}; carried {seen:?}"),
            Ok(None) => panic!("the event socket closed; carried {seen:?}"),
            Err(_) => panic!("the event socket said nothing for {PATIENCE:?}; carried {seen:?}"),
        };
        let text = match message {
            Message::Text(text) => text,
            other => {
                seen.push(format!("{other:?}"));
                continue;
            }
        };
        let Ok(event) = serde_json::from_str::<Value>(&text) else { continue };
        let kind = event["type"].as_str().unwrap_or_default().to_string();
        if kind == "messageAppended" {
            let message = event["message"].clone();
            if words(&message).contains(needle) {
                return message;
            }
            seen.push(format!("message from {}", message["from"]));
        } else {
            seen.push(kind);
        }
    }
}

/// What one envelope says, in the parts that carry text.
fn words(message: &Value) -> String {
    message["parts"]
        .as_array()
        .map(|parts| {
            parts.iter().filter_map(|part| part["text"].as_str()).collect::<Vec<_>>().join("\n")
        })
        .unwrap_or_default()
}

/// An agent in `group`, as the roster reports it.
async fn create_agent(addr: SocketAddr, group: Option<&str>, name: &str) -> String {
    let mut draft = json!({
        "name": name,
        "avatar": "avocado",
        "color": "#7ab55c",
        "model": "",
        "systemPrompt": "A test agent.",
    });
    if let Some(group) = group {
        draft["groupId"] = json!(group);
    }
    let (status, body) = call(addr, "create_agent", json!({ "draft": draft })).await;
    assert_eq!(status, 200, "{body}");
    body["ok"]["id"].as_str().expect("an agent id").to_string()
}

/// Everything a turn needs against the scripted model: the machine's key, and
/// a group whose endpoint is the stub. Returns the group's id, because an agent
/// only runs on the endpoint its own group resolves.
async fn point_at_the_model(addr: SocketAddr, group: &str, base_url: &str) -> String {
    let (status, body) = call(
        addr,
        "update_settings",
        json!({ "patch": { "apiKey": "sk-test", "operatorName": "The operator" } }),
    )
    .await;
    assert_eq!(status, 200, "{body}");

    let (status, body) = call(
        addr,
        "create_group",
        json!({ "draft": {
            "name": group,
            "inference": { "provider": "compatible", "baseUrl": base_url, "defaultModel": "test/model" },
        }}),
    )
    .await;
    assert_eq!(status, 200, "{body}");
    assert_eq!(body["ok"]["name"], group);
    body["ok"]["id"].as_str().expect("a group id").to_string()
}

// ---- the property --------------------------------------------------------

#[tokio::test]
async fn a_workspace_takes_a_turn_and_fires_a_routine_with_no_account_service() {
    let absent = absent_service().to_string();
    let stub = serve(|state| {
        // The routine's instruction and the operator's question are the two
        // things this workspace can be asked, and telling them apart is what
        // lets the test assert the firing reached the model rather than the
        // chat being answered twice.
        if state["messages"].to_string().contains("Check the listings") {
            Script::Say("Nothing new in the listings.".into())
        } else {
            Script::Say("Three new listings today.".into())
        }
    })
    .await;

    // Opens, and binds, with nothing of the account service present. A boot
    // that dialed it would be a workspace that never came up on a machine whose
    // service was down, so opening this one is itself an assertion.
    let (addr, _dir) = workspace(None).await;
    let mut socket = watching(addr).await;

    let group = point_at_the_model(addr, "Support", &stub.base_url).await;
    let support = create_agent(addr, Some(&group), "Pip").await;

    // A turn, from the operator's message to the reply on the socket.
    let (status, sent) =
        call(addr, "send_message", json!({ "agentId": support, "text": "What changed today?" }))
            .await;
    assert_eq!(status, 200, "{sent}");
    let reply = next_saying(&mut socket, "Three new listings today.").await;
    assert_eq!(reply["from"]["kind"], "agent", "the reply is the agent's: {reply}");
    assert_eq!(reply["to"]["kind"], "human", "it is addressed to the operator: {reply}");

    // And the record, read back the way a client reads a channel: the same
    // words, persisted, with the operator's question ahead of them.
    let (status, channel) =
        call(addr, "channel_messages", json!({ "channelId": support, "limit": 20 })).await;
    assert_eq!(status, 200, "{channel}");
    let channel = channel["ok"].as_array().expect("a channel").clone();
    assert!(channel.iter().any(|m| words(m).contains("What changed today?")), "{channel:?}");
    assert!(channel.iter().any(|m| words(m).contains("Three new listings today.")), "{channel:?}");

    // A routine firing: the same turn, started by the schedule rather than by
    // a person, and delivered to the model as the instruction it is.
    let (status, routine) = call(
        addr,
        "create_routine",
        json!({ "agentId": support, "draft": {
            "name": "Listings sweep",
            "what": "Check the listings and say what is new.",
            "trigger": "daily",
        }}),
    )
    .await;
    assert_eq!(status, 200, "{routine}");
    let id = routine["ok"]["id"].as_str().expect("a routine id");
    let (status, fired) = call(addr, "test_routine", json!({ "id": id })).await;
    assert_eq!(status, 200, "{fired}");
    next_saying(&mut socket, "Nothing new in the listings.").await;

    let (status, runs) = call(addr, "routine_runs", json!({ "id": id, "limit": 20 })).await;
    assert_eq!(status, 200, "{runs}");
    let runs = runs["ok"].as_array().expect("a history");
    assert!(!runs.is_empty(), "a fired routine is in its history: {runs:?}");

    // The settings pane, which draws from this on every open and must never
    // wait for a service to tell it what it already knows.
    let (status, settings) = call(addr, "get_settings", json!({})).await;
    assert_eq!(status, 200, "{settings}");
    assert_eq!(settings["ok"]["operatorName"], "The operator");
    assert_eq!(settings["ok"]["apiKeySet"], true);
    assert!(
        settings["ok"]["defaultModel"].as_str().is_some_and(|model| !model.is_empty()),
        "the pane draws an empty model: {settings}"
    );
    assert!(!settings.to_string().contains("sk-test"), "a key crossed: {settings}");

    // Every model call was the stub's, and there were two of them: one per turn.
    assert_eq!(stub.calls.load(std::sync::atomic::Ordering::SeqCst), 2);

    // The account itself reads as what it is. Answering this is a file read;
    // a build that asked the service whether a machine was signed in would have
    // waited here for something that is not there.
    let (status, account) = call(addr, "account_status", json!({})).await;
    assert_eq!(status, 200, "{account}");
    assert_eq!(account["ok"]["signedIn"], false);
    assert_eq!(account["ok"]["origin"], absent.as_str());
    let _ = socket.close(None).await;
}

#[tokio::test]
async fn the_connectors_that_live_on_that_service_refuse_in_their_own_panel() {
    let (addr, _dir) = workspace(None).await;
    let (status, groups) = call(addr, "list_groups", json!({})).await;
    assert_eq!(status, 200, "{groups}");
    let group = groups["ok"][0]["id"].as_str().expect("a group").to_string();

    // The catalog is a table this build ships. Listing it is what draws the
    // Connectors pane, and it must not become a question asked of the service:
    // a blank panel for everybody who never signed in is the failure.
    let (status, catalog) = call(addr, "plugin_catalog", json!({})).await;
    assert_eq!(status, 200, "{catalog}");
    let offers = catalog["ok"].as_array().expect("a catalog");
    let google = offers.iter().find(|offer| offer["kind"] == "google").expect("the Google tile");
    assert_eq!(google["accountBacked"], true);

    // Connecting it is the one thing this machine cannot do, and the answer
    // names the place that fixes it. Before anything is spent, because there is
    // nothing to spend: the refusal is the account being absent.
    let (status, refused) =
        call(addr, "connect_plugin", json!({ "groupId": group, "kind": "google" })).await;
    assert_eq!(status, 200, "a refusal is not an HTTP failure: {refused}");
    assert_eq!(refused["err"]["kind"], "plugin", "{refused}");
    let said = refused["err"]["message"].as_str().unwrap_or_default();
    assert!(said.contains("Settings, Account"), "the refusal does not say where to fix it: {said}");

    // And what the account holds is refused the same way rather than fetched.
    let (status, held) = call(addr, "account_connectors", json!({})).await;
    assert_eq!(status, 200, "{held}");
    assert_eq!(held["err"]["kind"], "signedOut", "{held}");
    assert!(
        held["err"]["message"].as_str().unwrap_or_default().contains("not signed in"),
        "the account does not say it is signed out: {held}"
    );
}

#[tokio::test]
async fn a_workspace_signed_in_to_a_service_that_vanished_still_works() {
    // The other half of the same promise: an operator who did sign in once, and
    // whose service is now gone. The token on disk is past its expiry, so every
    // account-backed path would renew it — and the renewal is a dial to an
    // address that refuses. What must not happen is that dial holding the turn,
    // the boot, or the pane.
    let absent = absent_service().to_string();
    let account = json!({
        "access_token": "an-access-token",
        "refresh_token": "a-refresh-token",
        "expires_at": 1,
        "email": "operator@example.com",
        "token_endpoint": format!("{absent}/api/auth/oauth2/token"),
        "origin": absent,
    });

    let stub = serve(|_| Script::Say("Still here.".into())).await;
    let (addr, _dir) = workspace(Some(account)).await;
    let mut socket = watching(addr).await;

    // Signed in is a fact about the file, not about the service, so it reads
    // that way with nothing there to confirm it.
    let (status, status_of) = call(addr, "account_status", json!({})).await;
    assert_eq!(status, 200, "{status_of}");
    assert_eq!(status_of["ok"]["signedIn"], true);
    assert_eq!(status_of["ok"]["email"], "operator@example.com");

    let group = point_at_the_model(addr, "Support", &stub.base_url).await;
    let support = create_agent(addr, Some(&group), "Pip").await;
    let (status, sent) =
        call(addr, "send_message", json!({ "agentId": support, "text": "Are you there?" })).await;
    assert_eq!(status, 200, "{sent}");
    next_saying(&mut socket, "Still here.").await;

    // The pane's two questions, both bounded: what the account holds, and the
    // connector that would spend it. Neither answers with a hang, and both name
    // the service they could not reach.
    let (status, held) = call(addr, "account_connectors", json!({})).await;
    assert_eq!(status, 200, "{held}");
    assert_eq!(held["err"]["kind"], "account", "{held}");
    assert!(
        held["err"]["message"].as_str().unwrap_or_default().contains(&absent),
        "the refusal does not name the service: {held}"
    );

    let (status, refused) =
        call(addr, "connect_plugin", json!({ "groupId": group, "kind": "google" })).await;
    assert_eq!(status, 200, "{refused}");
    let said = refused["err"]["message"].as_str().unwrap_or_default();
    assert!(
        said.contains(&absent),
        "the connector refused without naming what it could not reach: {said}"
    );

    // And the workspace is still a workspace afterwards: the turn above was not
    // a lucky first one, and the model is still reachable.
    let (status, again) =
        call(addr, "send_message", json!({ "agentId": support, "text": "Anything else?" })).await;
    assert_eq!(status, 200, "{again}");
    next_saying(&mut socket, "Still here.").await;
    let _ = socket.close(None).await;
}
