//! The daemon, driven the way a client drives it.
//!
//! The cascade suite proves the runtime does as it is told and the trajectory
//! suite proves the machinery behaved. Neither can see this, because both drive
//! the runtime directly: what is tested here is the *transport*, which is the
//! only part of a hosted workspace that has no desktop equivalent to fall back
//! on.
//!
//! Three questions, and they are the three that have no other answer:
//!
//! - Does a command called over HTTP do what the same command does in-process?
//! - Does an event reach a client that is not a webview?
//! - Does a workspace that runs on a server refuse the things it cannot do,
//!   with a sentence rather than a failure?
//!
//! Entirely offline. No provider is dialed, no model is called, nothing is
//! spent. A workspace is a temporary directory that is deleted at the end.

#![cfg(feature = "server")]

use std::net::SocketAddr;

use futures_util::StreamExt;
use serde_json::{json, Value};

const TOKEN: &str = "a-token-nobody-guessed";

/// A daemon on a free port, with a workspace of its own.
async fn workspace() -> (SocketAddr, tempfile::TempDir) {
    let dir = tempfile::tempdir().expect("a temporary workspace");
    let bound = guac_lib::server::bind(guac_lib::server::Settings {
        root: dir.path().to_path_buf(),
        // Port zero, so nothing in this suite can collide with a daemon the
        // operator happens to be running, or with another test.
        bind: "127.0.0.1:0".parse().expect("a loopback address"),
        token: TOKEN.to_string(),
        web: None,
    })
    .await
    .expect("the workspace opens");

    let addr = bound.addr;
    tokio::spawn(bound.serve());
    (addr, dir)
}

/// One command, as a client makes it.
async fn call(addr: SocketAddr, name: &str, args: Value) -> (u16, Value) {
    let response = reqwest::Client::new()
        .post(format!("http://{addr}/v1/call"))
        .bearer_auth(TOKEN)
        .json(&json!({ "name": name, "args": args }))
        .send()
        .await
        .expect("the daemon answers");
    let status = response.status().as_u16();
    (status, response.json().await.expect("the answer is JSON"))
}

#[tokio::test]
async fn a_workspace_on_a_server_answers_the_same_commands_the_window_does() {
    let (addr, _dir) = workspace().await;

    // The default group exists on a fresh workspace, exactly as it does on a
    // desktop: the migrations are the same migrations.
    let (status, body) = call(addr, "list_groups", json!({})).await;
    assert_eq!(status, 200);
    let groups = body["ok"].as_array().expect("a list of groups");
    assert_eq!(groups.len(), 1, "a fresh workspace has one group: {body}");

    let (_, body) = call(
        addr,
        "create_agent",
        json!({ "draft": {
            "name": "Pip",
            "avatar": "avocado",
            "color": "#7ab55c",
            "model": "",
            "systemPrompt": "A test agent.",
        }}),
    )
    .await;
    assert!(body["ok"]["id"].is_string(), "the agent came back with an id: {body}");

    let (_, body) = call(addr, "list_agents", json!({})).await;
    let agents = body["ok"].as_array().expect("a roster");
    assert_eq!(agents.len(), 1, "the agent is on the roster: {body}");
    assert_eq!(agents[0]["name"], "Pip");
}

#[tokio::test]
async fn a_server_refuses_what_it_cannot_do_and_says_what_to_do_instead() {
    let (addr, _dir) = workspace().await;

    // Every one of these is something on the operator's own machine. The
    // refusal is what turns "the button did nothing" into a sentence, and each
    // has to name an alternative: a refusal that only says no gets retried.
    let refusals = [
        (
            "create_repository",
            json!({ "draft": { "groupId": "00000000-0000-4000-8000-000000000001", "path": "/tmp" }}),
        ),
        ("stage_files", json!({ "paths": ["/etc/hosts"] })),
        ("save_file", json!({ "digest": "0".repeat(64), "name": "notes.txt" })),
    ];

    for (name, args) in refusals {
        let (status, body) = call(addr, name, args).await;
        assert_eq!(status, 200, "a refusal is not an HTTP failure: {name}");
        assert_eq!(body["err"]["kind"], "notHere", "{name} refused for the wrong reason: {body}");
        let said = body["err"]["message"].as_str().unwrap_or_default();
        assert!(said.contains("server"), "{name} does not say where it is running: {said}");
        assert!(
            said.contains(" or ") || said.contains("instead") || said.contains(", and "),
            "{name} refuses without offering a way forward: {said}"
        );
    }
}

#[tokio::test]
async fn a_harness_that_spends_a_plan_on_a_laptop_is_withheld_rather_than_hidden() {
    let (addr, _dir) = workspace().await;
    let (_, body) = call(addr, "coding_harnesses", json!({})).await;
    let harnesses = body["ok"].as_array().expect("the harnesses");

    // Both rows come back. A harness that silently vanishes from the list on a
    // server is a panel that disagrees with the operator's own laptop and
    // explains nothing.
    assert_eq!(harnesses.len(), 2, "every harness is on the list: {body}");

    let claude = harnesses.iter().find(|h| h["harness"] == "claude").expect("Claude Code's row");
    let said = claude["withheld"].as_str().expect("a reason it is withheld");
    assert!(said.contains("plan"), "the reason does not name the plan: {said}");
    assert!(said.contains("local workspace"), "the reason offers no way out: {said}");

    let pi = harnesses.iter().find(|h| h["harness"] == "pi").expect("pi's row");
    assert!(pi["withheld"].is_null(), "pi is not withheld anywhere: {pi}");
}

#[tokio::test]
async fn nothing_is_reachable_without_the_token() {
    let (addr, _dir) = workspace().await;
    let client = reqwest::Client::new();

    // Health is the one exception, and deliberately: a provider's health check
    // has no credential, and a check that needed one would report a box
    // unhealthy for the whole time a token was being rotated.
    let health = client.get(format!("http://{addr}/health")).send().await.expect("a health check");
    assert_eq!(health.status(), 200);

    for token in [None, Some("not-the-token")] {
        let mut request = client
            .post(format!("http://{addr}/v1/call"))
            .json(&json!({ "name": "list_agents", "args": {} }));
        if let Some(token) = token {
            request = request.bearer_auth(token);
        }
        let response = request.send().await.expect("the daemon answers");
        assert_eq!(response.status(), 401, "a workspace was reachable with {token:?}");
    }
}

#[tokio::test]
async fn an_event_reaches_a_client_that_is_not_a_webview() {
    let (addr, _dir) = workspace().await;

    // The token goes in the query string because a browser cannot set headers
    // on a WebSocket handshake. That is the whole reason the socket accepts it
    // there, and this is the test that says so.
    let (mut socket, _) =
        tokio_tungstenite::connect_async(format!("ws://{addr}/v1/events?token={TOKEN}"))
            .await
            .expect("the event socket opens");

    call(
        addr,
        "create_agent",
        json!({ "draft": {
            "name": "Pip",
            "avatar": "avocado",
            "color": "#7ab55c",
            "model": "",
            "systemPrompt": "A test agent.",
        }}),
    )
    .await;

    // `agentsChanged` is what the roster redraws on. Anything else that arrives
    // first is fine and is skipped: the runtime emits activity as well, and the
    // order between them is not something this transport promises.
    let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(10);
    let mut kinds = Vec::new();
    loop {
        let left = deadline.saturating_duration_since(tokio::time::Instant::now());
        assert!(!left.is_zero(), "no roster event arrived; saw {kinds:?}");
        let Ok(Some(Ok(message))) = tokio::time::timeout(left, socket.next()).await else {
            panic!("the socket closed before the roster changed; saw {kinds:?}");
        };
        let Ok(text) = message.into_text() else { continue };
        let Ok(event) = serde_json::from_str::<Value>(&text) else { continue };
        let kind = event["type"].as_str().unwrap_or_default().to_string();
        if kind == "agentsChanged" {
            break;
        }
        kinds.push(kind);
    }

    let _ = socket.close(None).await;
}

#[tokio::test]
async fn a_client_on_a_different_build_is_told_which_of_them_is_wrong() {
    let (addr, _dir) = workspace().await;

    // The failure this exists for is routine on a server and impossible on a
    // desktop: the app and the workspace it is connected to are updated
    // separately, so a name one of them has never heard of is a Tuesday.
    let (status, body) = call(addr, "summon_kraken", json!({})).await;
    assert_eq!(status, 404);
    assert_eq!(body["err"]["kind"], "unknownCommand");
    let said = body["err"]["message"].as_str().unwrap_or_default();
    assert!(said.contains("summon_kraken"), "it does not name the command: {said}");
    assert!(said.contains("update"), "it does not say what to do: {said}");

    let (status, body) = call(addr, "agent_memory", json!({ "wrongField": 1 })).await;
    assert_eq!(status, 400);
    assert_eq!(body["err"]["kind"], "badArguments");
    assert!(
        body["err"]["message"].as_str().unwrap_or_default().contains("agent_memory"),
        "it does not name the command: {body}"
    );
}

#[tokio::test]
async fn a_stored_file_is_reachable_by_its_digest() {
    let (addr, dir) = workspace().await;
    let client = reqwest::Client::new();

    // Placed the way the store lays them out, because the command that would
    // normally put one there reads a path on the operator's machine and is
    // refused on a server. What is being tested is the route, not the staging.
    let body = b"the quick brown fox";
    let digest = {
        use sha2::Digest;
        format!("{:x}", sha2::Sha256::digest(body))
    };
    let (prefix, rest) = digest.split_at(2);
    let holding = dir.path().join("data/files").join(prefix);
    std::fs::create_dir_all(&holding).expect("the file store");
    std::fs::write(holding.join(rest), body).expect("a stored file");

    let url = format!("http://{addr}/v1/file/{digest}/notes.txt");

    // Without a token this must be 401 rather than 404. The difference is the
    // whole test: a route registered with the wrong parameter syntax matches
    // nothing, falls through, and answers 404 to everything including this.
    let refused = client.get(&url).send().await.expect("an answer");
    assert_eq!(refused.status(), 401, "the file route did not match at all");

    let served = client.get(&url).query(&[("token", TOKEN)]).send().await.expect("an answer");
    assert_eq!(served.status(), 200);
    assert_eq!(served.headers()["content-type"], "text/plain");
    assert_eq!(served.bytes().await.expect("the bytes").as_ref(), body);

    // A digest nothing is stored under is a missing file, said as one.
    let missing = client
        .get(format!("http://{addr}/v1/file/{}/notes.txt", "0".repeat(64)))
        .query(&[("token", TOKEN)])
        .send()
        .await
        .expect("an answer");
    assert_eq!(missing.status(), 404);
}
