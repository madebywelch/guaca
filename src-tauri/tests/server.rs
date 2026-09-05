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
        origin: None,
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
async fn a_repository_arrives_on_a_box_as_a_clone_of_a_remote() {
    let (addr, dir) = workspace().await;

    // A bare repository standing in for the forge, so nothing leaves this
    // machine. What matters is the shape: clone, row, and the clone's removal.
    let bare = dir.path().join("origin.git");
    let seed = dir.path().join("seed");
    for args in [
        vec!["init", "--bare", bare.to_str().unwrap()],
        vec!["init", "-b", "main", seed.to_str().unwrap()],
    ] {
        let done = std::process::Command::new("git").args(&args).output().expect("git runs");
        assert!(done.status.success(), "{args:?}: {done:?}");
    }
    std::fs::write(seed.join("a.txt"), "one").unwrap();
    for args in [
        vec!["add", "."],
        vec!["-c", "user.name=t", "-c", "user.email=t@x", "commit", "-m", "one"],
        vec!["push", bare.to_str().unwrap(), "main"],
    ] {
        let done = std::process::Command::new("git")
            .arg("-C")
            .arg(&seed)
            .args(&args)
            .output()
            .expect("git runs");
        assert!(done.status.success(), "{args:?}: {done:?}");
    }

    let remote = format!("file://{}", bare.display());
    let (status, body) = call(
        addr,
        "create_repository",
        json!({ "draft": {
            "groupId": "00000000-0000-4000-8000-000000000001",
            "remote": remote,
        }}),
    )
    .await;
    assert_eq!(status, 200, "{body}");
    let row = &body["ok"];
    assert_eq!(row["remote"], remote.as_str(), "{row}");
    assert_eq!(row["name"], "origin", "named for the repository itself: {row}");
    let path = row["path"].as_str().expect("the clone's path").to_string();
    assert!(
        path.contains("/data/repos/"),
        "the clone lives in the workspace's own directory: {path}"
    );
    assert!(std::path::Path::new(&path).join("a.txt").exists(), "the clone has the history");

    let (_, linked) = call(
        addr,
        "create_repository",
        json!({ "draft": {
            "groupId": "00000000-0000-4000-8000-000000000001", "path": seed.to_str().unwrap()
        }}),
    )
    .await;
    let linked_id = linked["ok"]["id"].as_str().expect("a backend directory can be linked");
    call(addr, "delete_repository", json!({ "id": linked_id })).await;
    assert!(
        seed.join("a.txt").exists(),
        "unlinking a mounted directory must never delete its contents"
    );

    // Repository creation, engineer assignment and harness changes are the
    // same public commands used by a remote browser. A switch preserves both
    // the path and the grant, even when the gate is still enabled.
    let (_, engineer) = call(addr, "create_agent", json!({"draft": {
        "name":"Engineer", "avatar":"avocado", "color":"#7ab55c", "model":"", "systemPrompt":"Test"
    }})).await;
    let engineer_id = engineer["ok"]["id"].as_str().unwrap();
    let (_, assigned) =
        call(addr, "set_agent_repository", json!({"id":engineer_id,"repositoryId":row["id"]}))
            .await;
    assert_eq!(assigned["ok"]["repositoryId"], row["id"]);
    for harness in ["codex", "claude", "pi"] {
        let (_, edited) = call(addr, "update_repository", json!({"id":row["id"],"name":"Code", "note":"Test", "harness":harness,"gate":"askBeforePushing","bench":"own"})).await;
        assert_eq!(edited["ok"]["harness"], harness);
        assert_eq!(edited["ok"]["path"], path);
        assert_eq!(edited["ok"]["gate"], "askBeforePushing");
        let (_, agents) = call(addr, "list_agents", json!({})).await;
        assert!(agents["ok"]
            .as_array()
            .unwrap()
            .iter()
            .any(|a| a["id"] == engineer_id && a["repositoryId"] == row["id"]));
    }
    let (_, connection) = call(addr, "repository_connection", json!({"id":row["id"]})).await;
    assert_eq!(connection["ok"]["remote"], remote);
    assert_eq!(connection["ok"]["managedCredential"], false);

    // Unlinking a clone removes it: it was the workspace's, not the operator's.
    let id = row["id"].as_str().unwrap();
    let (status, body) = call(addr, "delete_repository", json!({ "id": id })).await;
    assert_eq!(status, 200, "{body}");
    assert!(!std::path::Path::new(&path).exists(), "the clone is gone");
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
