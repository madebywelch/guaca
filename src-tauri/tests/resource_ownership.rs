mod harness;

use std::sync::{
    atomic::{AtomicUsize, Ordering},
    Arc,
};

use axum::{routing::get, Json, Router};
use guac_lib::runtime::guard::GuardLimits;
use serde_json::json;

static PROVIDER: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

fn command_state(h: &harness::Harness) -> Arc<guac_lib::commands::AppState> {
    use guac_lib::{
        account::Account,
        artifact::Artifacts,
        commands::{AppState, Reach},
        domain::deployment::Deployment,
        llm::catalog::Catalog,
        subscription::Subscription,
    };
    use std::sync::atomic::AtomicU16;
    Arc::new(AppState {
        runtime: h.runtime.clone(),
        menubar: Arc::new(|_| {}),
        secret: None,
        deployment: Deployment::Desktop,
        open_url: Arc::new(|_| Ok(())),
        reach: Reach::Desktop,
        config_path: h._dir.path().join("config.json"),
        downloads: h._dir.path().to_path_buf(),
        subscription: Arc::new(Subscription::open(h._dir.path().join("subscription.json"))),
        claude_signin: Default::default(),
        account: Arc::new(Account::open(h._dir.path().join("account.json"))),
        catalog: Arc::new(Catalog::new()),
        artifacts: Artifacts::new(),
        artifact_port: Arc::new(AtomicU16::new(0)),
    })
}

#[tokio::test]
async fn operator_deletion_never_clears_a_concurrent_replacement() {
    let _guard = PROVIDER.lock().await;
    let arrived = Arc::new(tokio::sync::Notify::new());
    let release = Arc::new(tokio::sync::Notify::new());
    let arrived_server = arrived.clone();
    let release_server = release.clone();
    let delete = move || {
        let arrived = arrived_server.clone();
        let release = release_server.clone();
        async move {
            arrived.notify_one();
            release.notified().await;
            axum::http::StatusCode::NO_CONTENT
        }
    };
    let app = Router::new()
        .route("/sandboxes/:id", axum::routing::delete(delete.clone()))
        .route("/browsers/:id", axum::routing::delete(delete));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    std::env::set_var("GUAC_E2B_API_BASE", format!("http://{addr}"));
    std::env::set_var("GUAC_KERNEL_API_BASE", format!("http://{addr}"));
    let stub = harness::serve(|_| harness::Script::Say(String::new())).await;

    let computer = harness::harness_with_computer(&stub, &["Computer"], GuardLimits::default());
    let computer_id = computer.id("Computer");
    computer
        .runtime
        .store()
        .set_agent_sandbox(computer_id, Some(("old-computer", "old", "old")))
        .unwrap();
    let state = command_state(&computer);
    let deletion = tokio::spawn(async move {
        guac_lib::commands::delete_agent_computer(&state, computer_id).await
    });
    arrived.notified().await;
    computer
        .runtime
        .store()
        .record_created_sandbox(computer_id, ("new-computer", "new", "new"))
        .unwrap();
    release.notify_one();
    deletion.await.unwrap().unwrap();
    let computer_after = computer.agent_named("Computer").unwrap();

    let browser = harness::harness_with_browser(&stub, &["Browser"], GuardLimits::default());
    let browser_id = browser.id("Browser");
    browser.runtime.store().set_agent_browser(browser_id, Some("old-browser")).unwrap();
    let state = command_state(&browser);
    let deletion =
        tokio::spawn(
            async move { guac_lib::commands::stop_agent_browser(&state, browser_id).await },
        );
    arrived.notified().await;
    browser.runtime.store().record_browser(browser_id, "new-browser", false).unwrap();
    release.notify_one();
    deletion.await.unwrap().unwrap();
    let browser_after = browser.agent_named("Browser").unwrap();

    assert!(
        computer_after.sandbox_id.as_deref() == Some("new-computer")
            && computer_after.sandbox_envd_token.as_deref() == Some("new")
            && browser_after.browser_id.as_deref() == Some("new-browser"),
        "computer: {:?}, token present: {}, browser: {:?}",
        computer_after.sandbox_id,
        computer_after.sandbox_envd_token.as_deref() == Some("new"),
        browser_after.browser_id
    );
}

#[tokio::test]
async fn status_and_revocation_never_clear_a_concurrent_browser_replacement() {
    let _guard = PROVIDER.lock().await;
    let arrived = Arc::new(tokio::sync::Notify::new());
    let release = Arc::new(tokio::sync::Notify::new());
    let get_arrived = arrived.clone();
    let get_release = release.clone();
    let delete_arrived = arrived.clone();
    let delete_release = release.clone();
    let app = Router::new().route(
        "/browsers/:id",
        get(move || {
            let arrived = get_arrived.clone();
            let release = get_release.clone();
            async move {
                arrived.notify_one();
                release.notified().await;
                axum::http::StatusCode::NOT_FOUND
            }
        })
        .delete(move || {
            let arrived = delete_arrived.clone();
            let release = delete_release.clone();
            async move {
                arrived.notify_one();
                release.notified().await;
                axum::http::StatusCode::NO_CONTENT
            }
        }),
    );
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    std::env::set_var("GUAC_KERNEL_API_BASE", format!("http://{addr}"));
    let stub = harness::serve(|_| harness::Script::Say(String::new())).await;
    let h = harness::harness_with_browser(&stub, &["Worker"], GuardLimits::default());
    let id = h.id("Worker");

    h.runtime.store().set_agent_browser(id, Some("old-status")).unwrap();
    let state = command_state(&h);
    let status = tokio::spawn(async move { guac_lib::commands::agent_browser(&state, id).await });
    arrived.notified().await;
    h.runtime.store().record_browser(id, "new-status", false).unwrap();
    release.notify_one();
    status.await.unwrap().unwrap();
    let after_status = h.agent_named("Worker").unwrap().browser_id;

    h.runtime.store().set_agent_browser(id, Some("old-revoked")).unwrap();
    let state = command_state(&h);
    let revoke =
        tokio::spawn(async move { guac_lib::commands::take_agent_browser(&state, id).await });
    arrived.notified().await;
    h.runtime.store().record_browser(id, "new-revoked", false).unwrap();
    release.notify_one();
    revoke.await.unwrap().unwrap();
    let after_revoke = h.agent_named("Worker").unwrap().browser_id;

    assert!(
        after_status.as_deref() == Some("new-status")
            && after_revoke.as_deref() == Some("new-revoked"),
        "status: {after_status:?}; revocation: {after_revoke:?}"
    );
}

#[test]
fn copied_workspace_keeps_identity_but_new_ledger_entries_do_not_cross() {
    use guac_lib::db::{store::ResourceState, Store};

    let dir = tempfile::tempdir().unwrap();
    let first_path = dir.path().join("first.db");
    let copy_path = dir.path().join("copy.db");
    let first = Store::open(&first_path).unwrap();
    let identity = first.workspace_identity().unwrap();
    first.record_resource("e2b", "before-copy", false).unwrap();
    let connection = rusqlite::Connection::open(&first_path).unwrap();
    connection.execute("VACUUM INTO ?1", [copy_path.to_str().unwrap()]).unwrap();
    let copy = Store::open(&copy_path).unwrap();
    assert_eq!(copy.workspace_identity().unwrap(), identity);
    assert_eq!(copy.resource_state("e2b", "before-copy").unwrap(), Some(ResourceState::Created));
    copy.record_resource("e2b", "copy-only", false).unwrap();
    assert_eq!(first.resource_state("e2b", "copy-only").unwrap(), None);
    first.record_resource("e2b", "first-only", false).unwrap();
    assert_eq!(copy.resource_state("e2b", "first-only").unwrap(), None);
}

#[tokio::test]
async fn sweeps_preserve_foreign_resources_and_stop_on_an_unreadable_store() {
    let _guard = PROVIDER.lock().await;
    let deletes = Arc::new(AtomicUsize::new(0));
    let deleted = deletes.clone();
    let state_code = Arc::new(AtomicUsize::new(500));
    let state_reply = state_code.clone();
    let app = Router::new()
        .route(
            "/v2/sandboxes",
            get(|| async {
                Json(json!([{ "sandboxID": "foreign", "metadata": { "guac": "true" } }]))
            }),
        )
        .route(
            "/sandboxes/:id",
            get(move || {
                let code = state_reply.load(Ordering::SeqCst) as u16;
                async move { (axum::http::StatusCode::from_u16(code).unwrap(), Json(json!({}))) }
            })
            .delete(move || {
                let deleted = deleted.clone();
                async move {
                    deleted.fetch_add(1, Ordering::SeqCst);
                    axum::http::StatusCode::NO_CONTENT
                }
            }),
        );
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    std::env::set_var("GUAC_E2B_API_BASE", format!("http://{addr}"));

    let stub = harness::serve(|_| harness::Script::Say(String::new())).await;
    let workspace = harness::harness_with_computer(&stub, &["Worker"], GuardLimits::default());
    let foreign_result = workspace.runtime.sweep_computers().await;
    let foreign_deletes = deletes.load(Ordering::SeqCst);
    workspace.runtime.store().record_resource("e2b", "missing", false).unwrap();
    workspace.runtime.sweep_computers().await.unwrap();
    assert_eq!(
        workspace.runtime.store().resource_state("e2b", "missing").unwrap(),
        Some(guac_lib::db::store::ResourceState::Created)
    );
    state_code.store(404, Ordering::SeqCst);
    workspace.runtime.sweep_computers().await.unwrap();
    assert_eq!(
        workspace.runtime.store().resource_state("e2b", "missing").unwrap(),
        Some(guac_lib::db::store::ResourceState::Gone)
    );

    let connection = rusqlite::Connection::open(workspace._dir.path().join("guac.db")).unwrap();
    connection.execute_batch("DROP TABLE agents").unwrap();
    let unreadable_result = workspace.runtime.sweep_computers().await;
    let unreadable_deletes = deletes.load(Ordering::SeqCst) - foreign_deletes;

    assert!(
        foreign_result.is_ok() && foreign_deletes == 0 && unreadable_result.is_err() && unreadable_deletes == 0,
        "foreign deletes: {foreign_deletes}; unreadable store returned error: {}; unreadable-store deletes: {unreadable_deletes}",
        unreadable_result.is_err()
    );
}

#[tokio::test]
async fn computer_preserves_the_reference_on_unknown_state_and_resume_write_failure() {
    let _guard = PROVIDER.lock().await;
    let phase = Arc::new(AtomicUsize::new(0));
    let creates = Arc::new(AtomicUsize::new(0));
    let pauses = Arc::new(AtomicUsize::new(0));
    let pause_count = pauses.clone();
    let state_phase = phase.clone();
    let create_count = creates.clone();
    let app = Router::new()
        .route("/sandboxes/:id", get(move || {
            let phase = state_phase.load(Ordering::SeqCst);
            async move {
                if phase == 0 {
                    (axum::http::StatusCode::INTERNAL_SERVER_ERROR, Json(json!({})))
                } else if phase == 1 {
                    (axum::http::StatusCode::OK, Json(json!({"state":"paused"})))
                } else {
                    (axum::http::StatusCode::OK, Json(json!({"state":"running"})))
                }
            }
        }))
        .route("/sandboxes", axum::routing::post(move || {
            let count = create_count.clone();
            async move {
                count.fetch_add(1, Ordering::SeqCst);
                Json(json!({"sandboxID":"replacement", "envdAccessToken":"new", "trafficAccessToken":"new"}))
            }
        }))
        .route("/sandboxes/:id/resume", axum::routing::post(|| async {
            Json(json!({"sandboxID":"original", "envdAccessToken":"new", "trafficAccessToken":"new"}))
        }))
        .route("/sandboxes/:id/pause", axum::routing::post(move || {
            let count = pause_count.clone();
            async move { count.fetch_add(1, Ordering::SeqCst); axum::http::StatusCode::NO_CONTENT }
        }));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    std::env::set_var("GUAC_E2B_API_BASE", format!("http://{addr}"));
    let stub = harness::serve(|_| harness::Script::Say(String::new())).await;
    let h = harness::harness_with_computer(&stub, &["Worker"], GuardLimits::default());
    let card = h.agent_named("Worker").unwrap();
    h.runtime.store().set_agent_sandbox(card.id, Some(("original", "old", "old"))).unwrap();
    let unknown = h.runtime.ensure_computer(&card).await;
    let unknown_reference = h.agent_named("Worker").unwrap().sandbox_id;
    let made = creates.load(Ordering::SeqCst);

    h.runtime.store().set_agent_sandbox(card.id, Some(("original", "old", "old"))).unwrap();
    let connection = rusqlite::Connection::open(h._dir.path().join("guac.db")).unwrap();
    connection.execute_batch("CREATE TRIGGER fail_resume BEFORE UPDATE OF sandbox_envd_token ON agents BEGIN SELECT RAISE(ABORT, 'write failed'); END;").unwrap();
    phase.store(1, Ordering::SeqCst);
    let resumed = h.runtime.ensure_computer(&card).await;
    let retained = h.agent_named("Worker").unwrap().sandbox_id;
    assert!(h.runtime.store().sandbox_resume_pending("original").unwrap());
    connection.execute_batch("DROP TRIGGER fail_resume").unwrap();
    phase.store(2, Ordering::SeqCst);
    let recovered = h.runtime.ensure_computer(&card).await;
    let pending = h.runtime.store().sandbox_resume_pending("original").unwrap();

    assert!(unknown.is_err() && unknown_reference.as_deref() == Some("original") && made == 0 && resumed.is_err() && retained.as_deref() == Some("original") && recovered.is_ok() && !pending && pauses.load(Ordering::SeqCst) == 1,
        "unknown returned error: {}; reference: {unknown_reference:?}; creates: {made}; resume write failure returned error: {}; reference: {retained:?}; recovered: {}; pending: {pending}; pauses: {}", unknown.is_err(), resumed.is_err(), recovered.is_ok(), pauses.load(Ordering::SeqCst));
}

#[tokio::test]
async fn failed_ledger_write_compensates_only_the_newly_created_resource() {
    let _guard = PROVIDER.lock().await;
    let deletes = Arc::new(AtomicUsize::new(0));
    let fail_delete = Arc::new(AtomicUsize::new(0));
    let deleted = deletes.clone();
    let fail = fail_delete.clone();
    let app = Router::new()
        .route("/sandboxes", axum::routing::post(|| async {
            Json(json!({"sandboxID":"new-resource", "envdAccessToken":"token", "trafficAccessToken":"token"}))
        }))
        .route("/sandboxes/:id", axum::routing::delete(move || {
            let deleted = deleted.clone();
            let fail = fail.clone();
            async move {
                deleted.fetch_add(1, Ordering::SeqCst);
                if fail.load(Ordering::SeqCst) == 0 { axum::http::StatusCode::NO_CONTENT } else { axum::http::StatusCode::INTERNAL_SERVER_ERROR }
            }
        }));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    std::env::set_var("GUAC_E2B_API_BASE", format!("http://{addr}"));
    let stub = harness::serve(|_| harness::Script::Say(String::new())).await;
    let h = harness::harness_with_computer(&stub, &["Worker"], GuardLimits::default());
    let card = h.agent_named("Worker").unwrap();
    let connection = rusqlite::Connection::open(h._dir.path().join("guac.db")).unwrap();
    connection.execute_batch("CREATE TABLE IF NOT EXISTS resource_ledger (provider TEXT, resource_id TEXT, created_at INTEGER, released_at INTEGER); CREATE TRIGGER fail_ledger BEFORE INSERT ON resource_ledger BEGIN SELECT RAISE(ABORT, 'ledger failed'); END;").unwrap();
    let first = h.runtime.ensure_computer(&card).await;
    let first_reference = h.agent_named("Worker").unwrap().sandbox_id;
    let first_deletes = deletes.load(Ordering::SeqCst);
    h.runtime.store().set_agent_sandbox(card.id, None).unwrap();
    fail_delete.store(1, Ordering::SeqCst);
    let second = h.runtime.ensure_computer(&card).await;
    let second_reference = h.agent_named("Worker").unwrap().sandbox_id;
    assert!(first.is_err() && second.is_err() && first_reference.is_none() && second_reference.is_none() && first_deletes == 1 && deletes.load(Ordering::SeqCst) == 2,
        "first error: {}, first reference: {first_reference:?}, first deletes: {first_deletes}, second error: {}, second reference: {second_reference:?}, total deletes: {}", first.is_err(), second.is_err(), deletes.load(Ordering::SeqCst));
}

#[tokio::test]
async fn browser_sweeps_keep_foreign_adopted_legacy_and_composted_resources() {
    use axum::extract::Path;
    let _guard = PROVIDER.lock().await;
    let stub = harness::serve(|_| harness::Script::Say(String::new())).await;
    let first = harness::harness_with_browser(&stub, &["First"], GuardLimits::default());
    let second = harness::harness_with_browser(&stub, &["Second"], GuardLimits::default());
    let first_id = first.runtime.store().workspace_identity().unwrap();
    let second_id = second.runtime.store().workspace_identity().unwrap();
    assert_ne!(first_id, second_id);
    first.runtime.store().record_resource("kernel", "first-owned", false).unwrap();
    first.runtime.store().record_resource("kernel", "adopted", true).unwrap();
    first.runtime.store().record_resource("kernel", "composted", false).unwrap();
    second.runtime.store().record_resource("kernel", "second-owned", false).unwrap();
    let card = first.agent_named("First").unwrap();
    first.runtime.store().set_agent_browser(card.id, Some("composted")).unwrap();
    first.runtime.store().discard_agent(card.id, 1).unwrap();
    let inventory = json!([
        {"session_id":"first-owned", "tags":{"guac":"true", "guac-workspace":first_id}},
        {"session_id":"second-owned", "tags":{"guac":"true", "guac-workspace":second_id}},
        {"session_id":"adopted", "tags":{"guac":"true", "guac-workspace":first_id}},
        {"session_id":"composted", "tags":{"guac":"true", "guac-workspace":first_id}},
        {"session_id":"legacy", "tags":{"guac":"true"}}
    ]);
    let deleted = Arc::new(parking_lot::Mutex::new(Vec::<String>::new()));
    let recorded = deleted.clone();
    let app = Router::new()
        .route(
            "/browsers",
            get(move || {
                let inventory = inventory.clone();
                async move { Json(inventory) }
            }),
        )
        .route(
            "/browsers/:id",
            axum::routing::delete(move |Path(id): Path<String>| {
                let recorded = recorded.clone();
                async move {
                    recorded.lock().push(id);
                    axum::http::StatusCode::NO_CONTENT
                }
            }),
        );
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    std::env::set_var("GUAC_KERNEL_API_BASE", format!("http://{addr}"));
    assert_eq!(first.runtime.sweep_browsers().await.unwrap(), 1);
    assert_eq!(&*deleted.lock(), &["first-owned"]);
    assert_eq!(second.runtime.sweep_browsers().await.unwrap(), 1);
    assert_eq!(&*deleted.lock(), &["first-owned", "second-owned"]);
}

#[tokio::test]
async fn sweep_and_provisioning_cannot_cross_ownership_snapshots() {
    let _guard = PROVIDER.lock().await;
    let inventory_seen = Arc::new(tokio::sync::Notify::new());
    let release_inventory = Arc::new(tokio::sync::Notify::new());
    let created = Arc::new(tokio::sync::Notify::new());
    let seen = inventory_seen.clone();
    let release = release_inventory.clone();
    let made = created.clone();
    let app = Router::new()
        .route("/v2/sandboxes", get(move || {
            let seen = seen.clone();
            let release = release.clone();
            async move { seen.notify_one(); release.notified().await; Json(json!([])) }
        }))
        .route("/sandboxes", axum::routing::post(move || {
            let made = made.clone();
            async move {
                made.notify_one();
                Json(json!({"sandboxID":"new", "envdAccessToken":"envd", "trafficAccessToken":"traffic"}))
            }
        }));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    std::env::set_var("GUAC_E2B_API_BASE", format!("http://{addr}"));
    let stub = harness::serve(|_| harness::Script::Say(String::new())).await;
    let h = harness::harness_with_computer(&stub, &["Worker"], GuardLimits::default());
    let card = h.agent_named("Worker").unwrap();
    let sweep_runtime = h.runtime.clone();
    let sweep = tokio::spawn(async move { sweep_runtime.sweep_computers().await });
    inventory_seen.notified().await;
    let provision_runtime = h.runtime.clone();
    let (started, ready) = tokio::sync::oneshot::channel();
    let provision = tokio::spawn(async move {
        let _ = started.send(());
        provision_runtime.ensure_computer(&card).await
    });
    ready.await.unwrap();
    tokio::time::pause();
    assert!(tokio::time::timeout(std::time::Duration::from_secs(1), created.notified())
        .await
        .is_err());
    tokio::time::resume();
    release_inventory.notify_one();
    assert_eq!(sweep.await.unwrap().unwrap(), 0);
    assert!(provision.await.unwrap().is_ok());
}
