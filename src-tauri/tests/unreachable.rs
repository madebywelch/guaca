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

const PATIENCE: Duration = Duration::from_secs(15);

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

async fn workspace(account: Option<Value>) -> (SocketAddr, tempfile::TempDir) {
    let dir = tempfile::tempdir().expect("a temporary workspace");
    if let Some(account) = account {
        let config = dir.path().join("config");
        std::fs::create_dir_all(&config).expect("the settings directory");
        std::fs::write(config.join("account.json"), account.to_string()).expect("an account file");
    }

    let bound = guac_lib::server::bind(guac_lib::server::Settings {
        root: dir.path().to_path_buf(),
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

async fn watching(addr: SocketAddr) -> Socket {
    let (socket, _) =
        tokio_tungstenite::connect_async(format!("ws://{addr}/v1/events?token={TOKEN}"))
            .await
            .expect("the event socket opens");
    socket
}

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

fn words(message: &Value) -> String {
    message["parts"]
        .as_array()
        .map(|parts| {
            parts.iter().filter_map(|part| part["text"].as_str()).collect::<Vec<_>>().join("\n")
        })
        .unwrap_or_default()
}

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

#[tokio::test]
async fn a_workspace_takes_a_turn_and_fires_a_routine_with_no_account_service() {
    let absent = absent_service().to_string();
    let stub = serve(|state| {
        if state["messages"].to_string().contains("Check the listings") {
            Script::Say("Nothing new in the listings.".into())
        } else {
            Script::Say("Three new listings today.".into())
        }
    })
    .await;

    let (addr, _dir) = workspace(None).await;
    let mut socket = watching(addr).await;

    let group = point_at_the_model(addr, "Support", &stub.base_url).await;
    let support = create_agent(addr, Some(&group), "Pip").await;

    let (status, sent) =
        call(addr, "send_message", json!({ "agentId": support, "text": "What changed today?" }))
            .await;
    assert_eq!(status, 200, "{sent}");
    let reply = next_saying(&mut socket, "Three new listings today.").await;
    assert_eq!(reply["from"]["kind"], "agent", "the reply is the agent's: {reply}");
    assert_eq!(reply["to"]["kind"], "human", "it is addressed to the operator: {reply}");

    let (status, channel) =
        call(addr, "channel_messages", json!({ "channelId": support, "limit": 20 })).await;
    assert_eq!(status, 200, "{channel}");
    let channel = channel["ok"].as_array().expect("a channel").clone();
    assert!(channel.iter().any(|m| words(m).contains("What changed today?")), "{channel:?}");
    assert!(channel.iter().any(|m| words(m).contains("Three new listings today.")), "{channel:?}");

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

    let (status, settings) = call(addr, "get_settings", json!({})).await;
    assert_eq!(status, 200, "{settings}");
    assert_eq!(settings["ok"]["operatorName"], "The operator");
    assert_eq!(settings["ok"]["apiKeySet"], true);
    assert!(
        settings["ok"]["defaultModel"].as_str().is_some_and(|model| !model.is_empty()),
        "the pane draws an empty model: {settings}"
    );
    assert!(!settings.to_string().contains("sk-test"), "a key crossed: {settings}");

    assert_eq!(stub.calls.load(std::sync::atomic::Ordering::SeqCst), 2);

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

    let (status, catalog) = call(addr, "plugin_catalog", json!({})).await;
    assert_eq!(status, 200, "{catalog}");
    let offers = catalog["ok"].as_array().expect("a catalog");
    let google = offers.iter().find(|offer| offer["kind"] == "google").expect("the Google tile");
    assert_eq!(google["accountBacked"], true);

    let (status, refused) =
        call(addr, "connect_plugin", json!({ "groupId": group, "kind": "google" })).await;
    assert_eq!(status, 200, "a refusal is not an HTTP failure: {refused}");
    assert_eq!(refused["err"]["kind"], "plugin", "{refused}");
    let said = refused["err"]["message"].as_str().unwrap_or_default();
    assert!(said.contains("Settings, Account"), "the refusal does not say where to fix it: {said}");

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

    let (status, again) =
        call(addr, "send_message", json!({ "agentId": support, "text": "Anything else?" })).await;
    assert_eq!(status, 200, "{again}");
    next_saying(&mut socket, "Still here.").await;
    let _ = socket.close(None).await;
}
