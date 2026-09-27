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
    workspace_beside(None).await
}

/// The same, on a box whose updater listens at `updater`.
async fn workspace_beside(updater: Option<std::path::PathBuf>) -> (SocketAddr, tempfile::TempDir) {
    let dir = tempfile::tempdir().expect("a temporary workspace");
    let bound = guac_lib::server::bind(guac_lib::server::Settings {
        root: dir.path().to_path_buf(),
        // Port zero, so nothing in this suite can collide with a daemon the
        // operator happens to be running, or with another test.
        bind: "127.0.0.1:0".parse().expect("a loopback address"),
        token: TOKEN.to_string(),
        web: None,
        origin: None,
        updater,
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
async fn subscription_catalog_reports_missing_signin_over_the_shared_surface() {
    let (addr, _dir) = workspace().await;
    let (status, body) = call(addr, "subscription_models", json!({})).await;
    assert_eq!(status, 200);
    assert_eq!(body["err"]["kind"], "subscriptionModels");
    assert!(body["err"]["message"].as_str().unwrap().contains("sign in"));
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
    let refusals = [("save_file", json!({ "digest": "0".repeat(64), "name": "notes.txt" }))];

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
async fn official_harnesses_are_available_on_the_backend_without_a_guaca_api_key() {
    let (addr, _dir) = workspace().await;
    let (_, body) = call(addr, "coding_harnesses", json!({})).await;
    let harnesses = body["ok"].as_array().expect("the harnesses");
    assert_eq!(harnesses.len(), 3);
    for name in ["codex", "claude", "pi"] {
        let row = harnesses.iter().find(|h| h["harness"] == name).unwrap();
        assert!(row["withheld"].is_null(), "{row}");
        assert!(row["signIn"].is_string());
    }
}

#[tokio::test]
async fn an_agent_is_given_a_terminal_on_the_box_and_keeps_how_it_codes() {
    let (addr, dir) = workspace().await;

    // The same public commands a remote browser uses, which is the whole of
    // what a box needs: the terminal is a directory the daemon makes in its
    // own data, and nothing about it names the client's disk.
    let (_, engineer) = call(addr, "create_agent", json!({"draft": {
        "name":"Engineer", "avatar":"avocado", "color":"#7ab55c", "model":"", "systemPrompt":"Test"
    }})).await;
    let id = engineer["ok"]["id"].as_str().unwrap().to_string();
    assert_eq!(engineer["ok"]["hasTerminal"], false, "nothing is inherited: {engineer}");

    let (status, body) = call(addr, "give_agent_terminal", json!({"id": id})).await;
    assert_eq!(status, 200, "{body}");
    let (_, set) = call(
        addr,
        "set_agent_coding",
        json!({"id": id, "harness": "codex", "gate": "askBeforePushing"}),
    )
    .await;
    assert!(set.get("ok").is_some(), "{set}");
    let (_, terminal) = call(addr, "agent_terminal", json!({"id": id})).await;
    let path = terminal["ok"]["path"].as_str().expect("the terminal's path").to_string();
    let workspace = std::fs::canonicalize(dir.path()).unwrap();
    assert!(
        path.starts_with(workspace.to_str().unwrap()) && path.contains("/terminals/"),
        "the terminal lives in the workspace's own directory: {path}"
    );
    assert!(std::path::Path::new(&path).is_dir(), "and it exists once it is asked for");

    let card = |agents: &serde_json::Value| {
        agents["ok"].as_array().unwrap().iter().find(|a| a["id"] == id.as_str()).cloned().unwrap()
    };
    let (_, agents) = call(addr, "list_agents", json!({})).await;
    let given = card(&agents);
    assert_eq!(given["hasTerminal"], true, "{given}");
    assert_eq!(given["harness"], "codex", "{given}");
    assert_eq!(given["gate"], "askBeforePushing", "{given}");

    // Taking it back keeps the directory and the two answers: a change of mind
    // about access is not a reason to delete an agent's work.
    std::fs::write(std::path::Path::new(&path).join("notes.md"), "kept").unwrap();
    call(addr, "take_agent_terminal", json!({"id": id})).await;
    let (_, agents) = call(addr, "list_agents", json!({})).await;
    let taken = card(&agents);
    assert_eq!(taken["hasTerminal"], false, "{taken}");
    assert_eq!(taken["harness"], "codex", "{taken}");
    assert!(std::path::Path::new(&path).join("notes.md").exists());
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
    let health: Value = health.json().await.unwrap();
    assert_eq!(health["version"], env!("CARGO_PKG_VERSION"));
    assert_eq!(health["apiGeneration"], guac_lib::updates::protocol().generation);
    let updates =
        client.get(format!("http://{addr}/v1/updates?refresh=true")).send().await.unwrap();
    assert_eq!(updates.status(), 401, "release checks require workspace authorization");

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

/// The next event of one kind, skipping whatever else the runtime emits.
async fn next_of_kind(
    socket: &mut tokio_tungstenite::WebSocketStream<
        tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>,
    >,
    wanted: &str,
) -> Value {
    let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(10);
    loop {
        let left = deadline.saturating_duration_since(tokio::time::Instant::now());
        assert!(!left.is_zero(), "no {wanted} event arrived");
        let Ok(Some(Ok(message))) = tokio::time::timeout(left, socket.next()).await else {
            panic!("the socket closed before {wanted} arrived");
        };
        let Ok(text) = message.into_text() else { continue };
        let Ok(event) = serde_json::from_str::<Value>(&text) else { continue };
        if event["type"] == wanted {
            return event;
        }
    }
}

// Several workers, as the daemon has: on one thread two commands with no await
// between their read and their save cannot interleave, and the race is hidden.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_settings_change_reaches_every_client_and_two_at_once_both_land() {
    let (addr, _dir) = workspace().await;
    let (mut other, _) =
        tokio_tungstenite::connect_async(format!("ws://{addr}/v1/events?token={TOKEN}"))
            .await
            .expect("the event socket opens");

    // A client that did not make the change is told, with the settings in it
    // and never a key: every open settings pane is drawing from this.
    call(addr, "update_settings", json!({ "patch": { "apiKey": "sk-secret-value" } })).await;
    let event = next_of_kind(&mut other, "settingsChanged").await;
    assert_eq!(event["settings"]["apiKeySet"], true);
    assert!(!event.to_string().contains("sk-secret-value"), "a key crossed the socket: {event}");

    // Two clients patching different fields at the same moment. Each read,
    // patched and saved on its own before, and the later save put the earlier
    // one's field back.
    for round in 0..40 {
        let name = format!("Operator {round}");
        let model = format!("vendor/model-{round}");
        let (a, b) = tokio::join!(
            call(addr, "update_settings", json!({ "patch": { "operatorName": name } })),
            call(addr, "update_settings", json!({ "patch": { "defaultModel": model } })),
        );
        assert_eq!(a.0, 200, "{a:?}");
        assert_eq!(b.0, 200, "{b:?}");
        let (_, now) = call(addr, "get_settings", json!({})).await;
        assert_eq!(now["ok"]["operatorName"], name, "round {round}: {now}");
        assert_eq!(now["ok"]["defaultModel"], model, "round {round}: {now}");
    }
    let _ = other.close(None).await;
}

#[tokio::test]
async fn a_connector_run_as_a_program_on_the_host_is_tested_and_added_like_any_other() {
    let (addr, _dir) = workspace().await;
    let command =
        format!("stdio:python3 {}/tests/fixtures/mcp-stdio.py", env!("CARGO_MANIFEST_DIR"));
    let env = json!([{ "name": "FIXTURE_TOKEN", "value": "a-secret-value" }]);

    let (status, report) =
        call(addr, "probe_server", json!({ "url": command, "headers": env })).await;
    assert_eq!(status, 200, "{report}");
    assert!(report["ok"]["transport"].as_str().unwrap().starts_with("stdio"), "{report}");
    assert_eq!(report["ok"]["tools"], json!(["echo", "whoami", "fail"]));

    let (_, groups) = call(addr, "list_groups", json!({})).await;
    let group = groups["ok"][0]["id"].clone();
    let (status, added) = call(
        addr,
        "add_plugin",
        json!({ "groupId": group, "name": "fixture", "url": command, "headers": env }),
    )
    .await;
    assert_eq!(status, 200, "{added}");
    assert!(!added.to_string().contains("a-secret-value"), "a value came back: {added}");
    assert!(added.to_string().contains("FIXTURE_TOKEN"), "the name is shown: {added}");

    // A key is not how a program is given one.
    let (_, refused) = call(
        addr,
        "add_plugin",
        json!({ "groupId": group, "name": "other", "url": command, "key": "sk-x" }),
    )
    .await;
    assert_eq!(refused["err"]["kind"], "validation", "{refused}");
    assert!(refused["err"]["message"].as_str().unwrap().contains("environment variable"));
}

#[tokio::test]
async fn the_host_lists_its_own_tools_from_what_agents_are_sent() {
    let (addr, _dir) = workspace().await;
    let (status, body) = call(addr, "builtin_tools", json!({})).await;
    assert_eq!(status, 200, "{body}");
    let tools = body["ok"].as_array().expect("a list");
    let named = |name: &str| tools.iter().find(|tool| tool["name"] == name).cloned();
    assert!(named("notebook").is_some_and(|tool| tool["needs"].is_null()), "{body}");
    assert_eq!(named("browse").unwrap()["needs"], "a browser");
}

#[tokio::test]
async fn the_operators_quick_actions_reach_every_window() {
    let (addr, _dir) = workspace().await;
    let (mut other, _) =
        tokio_tungstenite::connect_async(format!("ws://{addr}/v1/events?token={TOKEN}"))
            .await
            .expect("the event socket opens");

    let (status, body) = call(
        addr,
        "add_quick_action",
        json!({ "label": "Calendar", "does": { "kind": "open", "place": { "kind": "calendar" } } }),
    )
    .await;
    assert_eq!(status, 200, "{body}");
    let id = body["ok"]["quickActions"][0]["id"].as_str().expect("an id").to_string();
    assert_eq!(body["ok"]["quickActions"][0]["addedBy"], "the operator");
    let event = next_of_kind(&mut other, "settingsChanged").await;
    assert_eq!(event["settings"]["quickActions"][0]["label"], "Calendar");

    // A button pointed at an agent that is not there is refused, not drawn dead.
    let (_, refused) = call(
        addr,
        "add_quick_action",
        json!({ "label": "Ghost", "does": {
            "kind": "message", "agentId": uuid::Uuid::new_v4(), "text": "hello",
        }}),
    )
    .await;
    assert_eq!(refused["err"]["kind"], "notFound", "{refused}");

    let (_, removed) = call(addr, "remove_quick_action", json!({ "id": id })).await;
    assert_eq!(removed["ok"]["quickActions"], json!([]));
    let _ = other.close(None).await;
}

#[tokio::test]
async fn skills_are_written_read_and_announced_over_the_hosted_surface() {
    let (addr, _dir) = workspace().await;
    let (mut socket, _) =
        tokio_tungstenite::connect_async(format!("ws://{addr}/v1/events?token={TOKEN}"))
            .await
            .expect("the event socket opens");

    let workspace = json!({ "kind": "workspace" });
    let (status, body) = call(
        addr,
        "save_skill",
        json!({ "scope": workspace, "draft": {
            "name": "house-style", "description": "When writing anything", "body": "Plain words.",
        }}),
    )
    .await;
    assert_eq!(status, 200, "{body}");
    assert_eq!(body["ok"]["scope"]["kind"], "workspace");
    let event = next_of_kind(&mut socket, "skillsChanged").await;
    assert_eq!(event["scope"]["kind"], "workspace");

    // A listing carries no bodies; a read does. The workspace opened with the
    // starters in it, because the boot both hosts share put them there.
    let (_, listed) = call(addr, "list_skills", json!({ "scope": workspace })).await;
    let listed = listed["ok"].as_array().expect("a list").clone();
    let names: Vec<_> = listed.iter().filter_map(|s| s["name"].as_str()).collect();
    for starter in guac_lib::skills::STARTERS.iter().map(|(name, _)| *name) {
        assert!(names.contains(&starter), "{starter} missing from {names:?}");
    }
    let house = listed.iter().find(|s| s["name"] == "house-style").expect("the new skill");
    assert_eq!(house["body"], "");
    assert_eq!(house["files"], json!([]));
    let (_, read) =
        call(addr, "read_skill", json!({ "scope": workspace, "name": "house-style" })).await;
    assert_eq!(read["ok"]["body"], "Plain words.");

    // Guaca's own is listed and cannot be written over, from any client.
    let (_, bundled) = call(addr, "list_skills", json!({ "scope": { "kind": "bundled" } })).await;
    assert!(bundled["ok"].as_array().unwrap().iter().any(|s| s["name"] == "guaca"));
    let (_, refused) = call(
        addr,
        "save_skill",
        json!({ "scope": workspace, "draft": { "name": "guaca", "description": "x", "body": "y" }}),
    )
    .await;
    assert_eq!(refused["err"]["kind"], "validation", "{refused}");

    // Nothing is written under the id of a crew that does not exist.
    let ghost = json!({ "kind": "crew", "groupId": uuid::Uuid::new_v4() });
    let (_, refused) = call(
        addr,
        "save_skill",
        json!({ "scope": ghost, "draft": { "name": "x", "description": "x", "body": "y" }}),
    )
    .await;
    assert_eq!(refused["err"]["kind"], "notFound", "{refused}");

    let (_, deleted) =
        call(addr, "delete_skill", json!({ "scope": workspace, "name": "house-style" })).await;
    assert_eq!(deleted["ok"], true);
    let _ = socket.close(None).await;
}

#[tokio::test]
async fn renaming_a_skill_that_carries_files_carries_them_to_the_new_name() {
    let (addr, dir) = workspace().await;
    let workspace = json!({ "kind": "workspace" });
    // A starter carries its license; a reference is what a skills.sh one
    // carries. A rename used to write the new name and delete the old one.
    let folder = dir.path().join("data/skills/workspace/grill-me");
    std::fs::create_dir_all(folder.join("references")).unwrap();
    std::fs::write(folder.join("references/rounds.md"), "# Rounds").unwrap();

    let (status, body) = call(
        addr,
        "save_skill",
        json!({ "scope": workspace, "draft": {
            "name": "grill", "description": "When grilling", "body": "Ask in rounds.",
            "previous": "grill-me",
        }}),
    )
    .await;
    assert_eq!(status, 200, "{body}");
    assert_eq!(body["ok"]["files"], json!(["LICENSE.txt", "references/rounds.md"]));
    let (_, gone) =
        call(addr, "read_skill", json!({ "scope": workspace, "name": "grill-me" })).await;
    assert_eq!(gone["err"]["kind"], "notFound", "{gone}");
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

    // A client that says what it is gets told which of the two to update.
    let body: Value = reqwest::Client::new()
        .post(format!("http://{addr}/v1/call"))
        .bearer_auth(TOKEN)
        .json(&json!({
            "name": "summon_kraken",
            "args": {},
            "client": { "version": "0.0.1", "desktop": true },
        }))
        .send()
        .await
        .expect("the daemon answers")
        .json()
        .await
        .expect("the answer is JSON");
    let said = body["err"]["message"].as_str().unwrap_or_default();
    assert!(said.contains("Guaca 0.0.1"), "it does not name the app's version: {said}");
    assert!(said.contains("download the latest Guaca"), "it does not say which side: {said}");
}

#[tokio::test]
async fn a_browser_hands_a_document_over_as_bytes_and_reads_it_back_by_digest() {
    let (addr, _dir) = workspace().await;
    let token = TOKEN;
    let client = reqwest::Client::new();

    // No token, no store.
    let refused = client
        .post(format!("http://{addr}/v1/upload?name=brief.txt"))
        .body("hello")
        .send()
        .await
        .unwrap();
    assert_eq!(refused.status(), 401);

    let stored: serde_json::Value = client
        .post(format!("http://{addr}/v1/upload?name=brief.txt"))
        .bearer_auth(token)
        .body("hello, box")
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let file = &stored["ok"];
    assert_eq!(file["name"], "brief.txt", "{stored}");
    assert_eq!(file["bytes"], 10, "{stored}");
    let digest = file["digest"].as_str().expect("a digest");

    // The same bytes, on the route every preview reads from.
    let back = client
        .get(format!("http://{addr}/v1/file/{digest}/brief.txt?token={token}"))
        .send()
        .await
        .unwrap();
    assert_eq!(back.status(), 200);
    assert_eq!(back.text().await.unwrap(), "hello, box");

    // Too big is the store's own sentence, not a bare 413.
    let big = vec![b'x'; 25 * 1024 * 1024 + 1];
    let refused = client
        .post(format!("http://{addr}/v1/upload?name=huge.bin"))
        .bearer_auth(token)
        .body(big)
        .send()
        .await
        .unwrap();
    assert_eq!(refused.status(), 422, "{}", refused.text().await.unwrap_or_default());
}

#[tokio::test]
async fn a_box_does_not_forward_files_from_its_own_disk() {
    let (addr, _dir) = workspace().await;
    let (status, body) = call(
        addr,
        "forward_files",
        json!({ "origin": "http://elsewhere", "token": "t", "paths": ["/etc/hosts"] }),
    )
    .await;
    // A desktop forwards a dropped path to the box it is showing; a box has no
    // operator's disk to read from, and says which capability that is.
    assert_eq!(body["err"]["kind"], "notHere", "{status} {body}");
    let said = body["err"]["message"].as_str().unwrap_or_default();
    assert!(said.contains("server"), "{said}");
}

#[tokio::test]
async fn a_page_an_agent_wrote_is_served_on_this_origin_under_the_same_policy() {
    let (addr, _dir) = workspace().await;
    let (status, body) = call(
        addr,
        "frame_artifact",
        json!({ "html": "<h1>hi</h1><script>guaca.answer(1)</script>" }),
    )
    .await;
    assert_eq!(status, 200, "{body}");
    let id = body["ok"]["id"].as_str().expect("an id");
    let ticket = body["ok"]["ticket"].as_str().expect("a scoped ticket");
    assert_ne!(ticket, TOKEN);

    let client = reqwest::Client::new();
    let refused = client.get(format!("http://{addr}/v1/artifact/{id}")).send().await.unwrap();
    assert_eq!(refused.status(), 401);

    let page =
        client.get(format!("http://{addr}/v1/artifact/{id}?token={ticket}")).send().await.unwrap();
    assert_eq!(page.status(), 200);
    let csp = page.headers().get("content-security-policy").expect("the policy rides along");
    assert!(csp.to_str().unwrap().contains("sandbox allow-scripts"));
    let text = page.text().await.unwrap();
    assert!(text.contains("guaca.answer"), "the bridge is prepended: {text}");
    assert!(text.contains("<h1>hi</h1>"));

    let gone = client
        .get(format!("http://{addr}/v1/artifact/{}?token={TOKEN}", "0".repeat(64)))
        .send()
        .await
        .unwrap();
    assert_eq!(gone.status(), 401);
}

#[tokio::test]
async fn a_screen_is_reached_by_a_ticket_for_that_sandbox_and_relayed_to_the_viewer() {
    let (addr, _dir) = workspace().await;
    let client = reqwest::Client::new();

    // A ticket for another sandbox, or a made-up one, opens nothing.
    let wrong = client
        .get(format!("http://{addr}/v1/screen/{}/sbx/6080/viewer.html", "f".repeat(64)))
        .send()
        .await
        .unwrap();
    assert_eq!(wrong.status(), 404);
    assert!(wrong.text().await.unwrap().contains("not a screen"));

    // The right ticket reaches the viewer, which is the one that knows there is
    // no such machine and says so: the relay carries its answer through whole.
    let ticket = guac_lib::commands::screen_ticket(TOKEN, "sbx");
    let relayed = client
        .get(format!("http://{addr}/v1/screen/{ticket}/sbx/6080/viewer.html?autoconnect=1"))
        .send()
        .await
        .unwrap();
    assert_eq!(relayed.status(), 404);
    assert!(relayed.text().await.unwrap().contains("No computer is registered"));
}

#[tokio::test]
async fn a_sign_in_nobody_is_waiting_for_is_told_so_at_the_door() {
    let (addr, _dir) = workspace().await;
    let client = reqwest::Client::new();
    // No token on this route: the browser arrives from the vendor. What bounds
    // it is that only a flow waiting on that exact state reads it.
    let page = client
        .get(format!("http://{addr}/v1/oauth/callback?state=stale&code=x"))
        .send()
        .await
        .unwrap();
    assert_eq!(page.status(), 404);
    assert!(page.text().await.unwrap().contains("Not a sign-in"));
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

#[tokio::test]
async fn desktop_downloads_backend_attachments_to_the_client_disk() {
    let (addr, backend) = workspace().await;
    let client = tempfile::tempdir().unwrap();
    let downloads = client.path().join("Downloads");
    let files = guac_lib::files::FileStore::new(backend.path().join("data/files"));
    let origin = format!("http://{addr}");
    let brief = files.put("brief.md", b"# The brief\n").unwrap();

    for (token, digest, expected) in [
        ("wrong", brief.digest.clone(), "access key"),
        (TOKEN, "0".repeat(64), "no longer"),
        (TOKEN, "../invalid".into(), "invalid content address"),
    ] {
        let error =
            guac_lib::files::download(&origin, token, &digest, &brief.name, downloads.clone())
                .await
                .unwrap_err();
        assert!(error.contains(expected), "{error}");
        assert!(!downloads.exists(), "a failed download must not create a file");
    }

    for (name, body) in [
        ("brief.md", b"# The brief\n".as_slice()),
        ("quarter report.pdf", b"%PDF-1.7\n".as_slice()),
        ("bundle.zip", b"PK\x03\x04\x00\xff".as_slice()),
    ] {
        let stored = files.put(name, body).unwrap();
        let save =
            || guac_lib::files::download(&origin, TOKEN, &stored.digest, name, downloads.clone());
        let (first, second) = tokio::join!(save(), save());
        let first = first.unwrap();
        let second = second.unwrap();
        assert_ne!(first, second, "concurrent copies must keep separate names");
        assert_eq!(first.parent(), Some(downloads.as_path()));
        assert_eq!(std::fs::read(first).unwrap(), body);
        assert_eq!(std::fs::read(second).unwrap(), body);
    }
}

#[tokio::test]
async fn the_desktop_can_preflight_a_command() {
    let (addr, _dir) = workspace().await;
    let response = reqwest::Client::new()
        .request(reqwest::Method::OPTIONS, format!("http://{addr}/v1/call"))
        .header("origin", "tauri://localhost")
        .header("access-control-request-method", "POST")
        .header("access-control-request-headers", "authorization,content-type")
        .send()
        .await
        .unwrap();
    assert!(
        response.status().is_success(),
        "preflight: {} {:?}",
        response.status(),
        response.headers()
    );
    assert!(response.headers().contains_key("access-control-allow-origin"));
}

#[tokio::test]
async fn an_html_attachment_cannot_run_on_the_workspace_origin() {
    let (addr, _dir) = workspace().await;
    let client = reqwest::Client::new();
    let stored: Value = client
        .post(format!("http://{addr}/v1/upload?name=report.html"))
        .bearer_auth(TOKEN)
        .body("<script>document.title=localStorage.getItem('guaca.workspace.token')</script>")
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let digest = stored["ok"]["digest"].as_str().unwrap();
    let response = client
        .get(format!("http://{addr}/v1/file/{digest}/report.html?token={TOKEN}"))
        .send()
        .await
        .unwrap();
    let headers = response.headers();
    let sandboxed = headers
        .get("content-security-policy")
        .and_then(|h| h.to_str().ok())
        .is_some_and(|v| v.contains("sandbox"));
    let download = headers
        .get("content-disposition")
        .and_then(|h| h.to_str().ok())
        .is_some_and(|v| v.starts_with("attachment"));
    assert!(sandboxed || download, "active document on workspace origin: {headers:?}");
}

#[tokio::test]
async fn opaque_and_unrelated_origins_cannot_read_the_workspace() {
    let (addr, _dir) = workspace().await;
    for origin in ["null", "https://unrelated.example", "https://tauri.localhost.evil.example"] {
        let response = reqwest::Client::new()
            .get(format!("http://{addr}/health"))
            .header("origin", origin)
            .send()
            .await
            .unwrap();
        assert!(response.headers().get("access-control-allow-origin").is_none());
    }
    let response = reqwest::Client::new()
        .post(format!("http://{addr}/v1/call"))
        .header("origin", "tauri://localhost")
        .json(&json!({"name":"capabilities"}))
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), 401);
    assert_eq!(response.headers()["access-control-allow-origin"], "tauri://localhost");
}

#[tokio::test]
async fn two_hosts_cannot_run_the_same_workspace() {
    let (_addr, dir) = workspace().await;
    let second = guac_lib::server::bind(guac_lib::server::Settings {
        root: dir.path().to_path_buf(),
        bind: "127.0.0.1:0".parse().unwrap(),
        token: TOKEN.into(),
        web: None,
        origin: None,
        updater: None,
    })
    .await;
    match second {
        Err(error) => assert!(error.contains("already running"), "{error}"),
        Ok(_) => panic!("a second host started the same actors"),
    }
}

#[tokio::test]
async fn a_model_address_belongs_to_the_backend_network() {
    let (addr, _dir) = workspace().await;
    for endpoint in ["http://127.0.0.1:11434/v1", "http://host.docker.internal:1234/v1"] {
        let (_, body) = call(
            addr,
            "create_group",
            json!({ "draft": {
                "name": endpoint, "inference": { "provider": "compatible", "baseUrl": endpoint }
            }}),
        )
        .await;
        assert!(body["ok"]["id"].is_string(), "{body}");
    }
}

#[tokio::test]
async fn main_calendar_and_webhook_commands_work_on_the_remote_backend() {
    let (addr, _dir) = workspace().await;
    let (_, groups) = call(addr, "list_groups", json!({})).await;
    let group = groups["ok"][0]["id"].as_str().unwrap();
    let (_, created) = call(
        addr,
        "create_occasion",
        json!({"draft": {
            "groupId": group, "title": "Remote calendar", "startsAt": "2026-09-05T15:00:00Z"
        }}),
    )
    .await;
    assert!(created["ok"]["id"].is_string(), "{created}");
    let id = created["ok"]["id"].clone();
    let (_, calendar) =
        call(addr, "calendar", json!({"from":0,"until":4102444800000_i64,"groupId":group})).await;
    assert_eq!(calendar["ok"].as_array().unwrap().len(), 1, "{calendar}");
    let (_, deleted) = call(addr, "delete_occasion", json!({"id":id})).await;
    assert!(deleted.get("ok").is_some(), "{deleted}");
    let (_, address) = call(addr, "webhook_address", json!({})).await;
    assert_eq!(address["ok"]["url"], format!("http://{addr}/events"));
    let secret = address["ok"]["secret"].as_str().unwrap();
    assert!(!secret.is_empty() && secret != TOKEN);
    let client = reqwest::Client::new();
    let url = format!("http://{addr}/events/test/event");
    assert_eq!(client.post(&url).bearer_auth(TOKEN).send().await.unwrap().status(), 401);
    assert_eq!(client.post(&url).bearer_auth(secret).send().await.unwrap().status(), 404);
    assert_eq!(
        client
            .post(&url)
            .bearer_auth(secret)
            .body(vec![b'x'; 65537])
            .send()
            .await
            .unwrap()
            .status(),
        413
    );
    assert_eq!(
        client
            .post(format!("http://{addr}/v1/call"))
            .bearer_auth(secret)
            .json(&json!({"name":"list_groups"}))
            .send()
            .await
            .unwrap()
            .status(),
        401
    );
}

#[tokio::test]
async fn groups_transfer_between_hosts_without_copying_identity() {
    let (source, _source_dir) = workspace().await;
    let (target, _target_dir) = workspace().await;
    let (_, groups) = call(source, "list_groups", json!({})).await;
    let old = groups["ok"][0]["id"].clone();
    let (_, created) = call(source, "create_agent", json!({"draft": {"groupId":old,"name":"Engineer","avatar":"avocado","color":"#7ab55c","model":"","systemPrompt":"Check before changing code.","skills":[]}})).await;
    assert!(created.get("ok").is_some(), "{created}");
    let (_, exported) = call(source, "export_group", json!({"id":old})).await;
    assert_eq!(exported["ok"]["format"], "guaca-group", "{exported}");
    let (_, imported) =
        call(target, "import_group", json!({"archive":exported["ok"],"name":"Imported crew"}))
            .await;
    assert_eq!(imported["ok"]["name"], "Imported crew", "{imported}");
    assert_ne!(imported["ok"]["id"], old);
    let (_, agents) = call(target, "list_agents", json!({})).await;
    assert_eq!(agents["ok"][0]["name"], "Engineer");
    assert_ne!(agents["ok"][0]["id"], created["ok"]["id"]);
    let (_, hints) = call(target, "group_reconnect", json!({"id":imported["ok"]["id"]})).await;
    assert_eq!(hints["ok"], json!([]));
}

#[tokio::test]
async fn secrets_are_write_only_and_manageable_over_the_hosted_surface() {
    let (addr, dir) = workspace().await;
    let (_, groups) = call(addr, "list_groups", json!({})).await;
    let group = &groups["ok"][0]["id"];
    let (_, agent) = call(addr, "create_agent", json!({"draft":{
        "name":"Deployer", "avatar":"avocado", "color":"#7ab55c", "model":"", "systemPrompt":"Test"
    }})).await;
    let agent = &agent["ok"]["id"];
    let (_, saved) = call(
        addr,
        "create_connector",
        json!({"draft":{
            "groupId":group, "service":"Cloudflare", "account":"", "envVar":"CLOUDFLARE_API_TOKEN",
            "secret":"a-private-deployment-token", "agents":[agent]
        }}),
    )
    .await;
    assert_eq!(saved["ok"]["agents"], json!([agent]));
    assert_eq!(saved["ok"]["secretSet"], true);
    assert!(!saved.to_string().contains("a-private-deployment-token"));
    assert_eq!(saved["ok"]["secretHint"], "");
    let id = &saved["ok"]["id"];
    let (_, rejected) =
        call(addr, "update_connector", json!({"id":id, "agents":[], "secret":""})).await;
    assert!(rejected.get("err").is_some());
    let (_, changed) = call(
        addr,
        "update_connector",
        json!({"id":id, "agents":[], "secret":"  rotated-private-token\n"}),
    )
    .await;
    assert!(changed.get("ok").is_some(), "{changed}");
    let db = rusqlite::Connection::open(guac_lib::boot::Paths::under(dir.path()).db()).unwrap();
    let stored: String = db
        .query_row("SELECT secret FROM connectors WHERE id=?1", [id.as_str().unwrap()], |row| {
            row.get(0)
        })
        .unwrap();
    assert_eq!(stored, "rotated-private-token", "rotation trims pasted whitespace like creation");
    let (_, listed) = call(addr, "group_connectors", json!({"groupId":group})).await;
    assert_eq!(listed["ok"][0]["agents"], json!([]));
    assert!(!listed.to_string().contains("rotated-private-token"));
    call(addr, "delete_connector", json!({"id":id})).await;
    let (_, listed) = call(addr, "group_connectors", json!({"groupId":group})).await;
    assert_eq!(listed["ok"], json!([]));
}

#[tokio::test]
async fn a_host_without_an_updater_says_so_and_refuses_to_update() {
    let (addr, _dir) = workspace().await;
    let http = reqwest::Client::new();
    let status = http.get(format!("http://{addr}/v1/host")).send().await.unwrap();
    assert_eq!(status.status(), 401, "what a box runs is the operator's business");
    let status: Value = http
        .get(format!("http://{addr}/v1/host"))
        .bearer_auth(TOKEN)
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(status, json!({ "managed": false }));
    let refused = http
        .post(format!("http://{addr}/v1/host/update"))
        .bearer_auth(TOKEN)
        .json(&json!({ "version": "9.9.9" }))
        .send()
        .await
        .unwrap();
    assert_eq!(refused.status(), 404);
    let body: Value = refused.json().await.unwrap();
    assert!(body["err"]["message"].as_str().unwrap().contains("update instructions"), "{body}");
}

#[cfg(unix)]
#[tokio::test]
async fn a_box_relays_its_updater_and_only_for_the_token() {
    use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
    let sockets = tempfile::tempdir().unwrap();
    let path = sockets.path().join("updater.sock");
    let listener = tokio::net::UnixListener::bind(&path).unwrap();
    let (asked, mut heard) = tokio::sync::mpsc::unbounded_channel::<Value>();
    let updater = tokio::spawn(async move {
        loop {
            let (stream, _) = listener.accept().await.unwrap();
            let (read, mut write) = stream.into_split();
            let mut line = String::new();
            BufReader::new(read).read_line(&mut line).await.unwrap();
            let request: Value = serde_json::from_str(&line).unwrap();
            let reply = if request["ask"] == "update" && request["version"] != "0.3.0" {
                json!({ "err": "The latest release is Guaca 0.3.0, not 9.9.9. Check for updates and review it again." })
            } else {
                json!({ "ok": { "updating": request["ask"] == "update",
                    "running": { "image": "guacad:old", "version": "0.2.0" },
                    "operation": null, "error": null } })
            };
            asked.send(request).unwrap();
            write.write_all(format!("{reply}\n").as_bytes()).await.unwrap();
        }
    });
    let (addr, _dir) = workspace_beside(Some(path)).await;
    let http = reqwest::Client::new();

    let unauthorized = http
        .post(format!("http://{addr}/v1/host/update"))
        .json(&json!({ "version": "0.3.0" }))
        .send()
        .await
        .unwrap();
    assert_eq!(unauthorized.status(), 401);
    assert!(heard.try_recv().is_err(), "nothing reaches the updater without the token");

    let status: Value = http
        .get(format!("http://{addr}/v1/host"))
        .bearer_auth(TOKEN)
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(status["managed"], true);
    assert_eq!(status["running"]["version"], "0.2.0");
    assert_eq!(heard.recv().await.unwrap(), json!({ "ask": "status" }));

    let accepted = http
        .post(format!("http://{addr}/v1/host/update"))
        .bearer_auth(TOKEN)
        .json(&json!({ "version": "0.3.0" }))
        .send()
        .await
        .unwrap();
    assert_eq!(accepted.status(), 200);
    assert_eq!(accepted.json::<Value>().await.unwrap()["updating"], true);
    assert_eq!(heard.recv().await.unwrap(), json!({ "ask": "update", "version": "0.3.0" }));

    let stale = http
        .post(format!("http://{addr}/v1/host/update"))
        .bearer_auth(TOKEN)
        .json(&json!({ "version": "9.9.9" }))
        .send()
        .await
        .unwrap();
    assert_eq!(stale.status(), 409);
    let body: Value = stale.json().await.unwrap();
    assert!(body["err"]["message"].as_str().unwrap().contains("review it again"), "{body}");
    updater.abort();
}
