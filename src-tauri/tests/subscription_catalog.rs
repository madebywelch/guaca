//! The subscription selector reads the authenticated catalog, without a CLI.
use std::sync::{
    atomic::{AtomicUsize, Ordering},
    Arc,
};

use axum::{
    extract::Query,
    http::{HeaderMap, StatusCode},
    response::IntoResponse,
    routing::{get, post},
    Json, Router,
};
use base64::Engine;
use guac_lib::{llm::codex, subscription::Subscription};
use serde_json::{json, Value};

struct Catalog {
    subscription: Subscription,
    reads: Arc<AtomicUsize>,
    refreshes: Arc<AtomicUsize>,
    _dir: tempfile::TempDir,
    server: tokio::task::JoinHandle<()>,
}

impl Drop for Catalog {
    fn drop(&mut self) {
        self.server.abort();
    }
}

async fn catalog(status: StatusCode, body: Value, expire: bool) -> Catalog {
    let reads = Arc::new(AtomicUsize::new(0));
    let refreshes = Arc::new(AtomicUsize::new(0));
    let app = Router::new().route("/models", get({
        let reads = reads.clone();
        move |headers: HeaderMap, Query(query): Query<std::collections::HashMap<String, String>>| {
            let reads = reads.clone();
            let body = body.clone();
            async move {
                let nth = reads.fetch_add(1, Ordering::SeqCst);
                assert_eq!(query["client_version"], "0.153.3");
                assert_eq!(headers["originator"], "codex_cli_rs");
                assert_eq!(headers["chatgpt-account-id"], "account");
                assert_eq!(headers["authorization"], if expire && nth > 0 { "Bearer fresh" } else { "Bearer stored" });
                if expire && nth == 0 { return StatusCode::UNAUTHORIZED.into_response(); }
                (status, Json(body)).into_response()
            }
        }
    })).route("/oauth/token", post({
        let refreshes = refreshes.clone();
        move || {
            refreshes.fetch_add(1, Ordering::SeqCst);
            async { Json(json!({"access_token":"fresh", "refresh_token":"next"})) }
        }
    }));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    let server = tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("subscription.json");
    std::fs::write(
        &path,
        json!({
            "access_token":"stored", "refresh_token":"refresh",
            "id_token":format!("e30.{}.sig", base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(
                json!({"https://api.openai.com/auth":{"chatgpt_account_id":"account"}}).to_string()
            )),
            "account_id":"account", "expires_at":4102444800i64
        })
        .to_string(),
    )
    .unwrap();
    Catalog {
        subscription: Subscription::open_at(path, &base, &base),
        reads,
        refreshes,
        _dir: dir,
        server,
    }
}

#[tokio::test]
async fn missing_signin_does_not_contact_the_catalog() {
    let dir = tempfile::tempdir().unwrap();
    let subscription = Subscription::open_at(
        dir.path().join("absent"),
        "http://127.0.0.1:1",
        "http://127.0.0.1:1",
    );
    assert!(codex::models(&subscription).await.unwrap_err().to_string().contains("sign in"));
}

#[tokio::test]
async fn failures_and_unusable_catalogs_are_errors_not_empty_successes() {
    for (status, body) in [
        (StatusCode::SERVICE_UNAVAILABLE, json!({})),
        (StatusCode::FORBIDDEN, json!({})),
        (StatusCode::OK, json!({"unexpected":[]})),
        (StatusCode::OK, json!({"models":[]})),
        (StatusCode::OK, json!({"models":[{"slug":"hidden", "visibility":"hide"}]})),
    ] {
        let server = catalog(status, body, false).await;
        assert!(codex::models(&server.subscription).await.is_err());
        assert_eq!(server.reads.load(Ordering::SeqCst), 1);
        assert_eq!(server.refreshes.load(Ordering::SeqCst), 0);
    }
}

#[tokio::test]
async fn live_offers_replace_the_compiled_list_in_priority_order() {
    let server = catalog(
        StatusCode::OK,
        json!({"models":[
            {"slug":"older", "visibility":"list", "priority":9},
            {"slug":"internal", "visibility":"hide", "priority":0},
            {"slug":"new-model", "visibility":"list", "priority":1, "supported_in_api":false},
            {"slug":"new-model", "visibility":"list", "priority":2},
            {"slug":" ", "visibility":"list", "priority":3}
        ]}),
        false,
    )
    .await;
    assert_eq!(codex::models(&server.subscription).await.unwrap(), ["new-model", "older"]);
}

#[tokio::test]
async fn an_expired_access_token_is_refreshed_once() {
    let server = catalog(
        StatusCode::OK,
        json!({"models":[{"slug":"new-model", "visibility":"list"}]}),
        true,
    )
    .await;
    assert_eq!(codex::models(&server.subscription).await.unwrap(), ["new-model"]);
    assert_eq!(server.reads.load(Ordering::SeqCst), 2);
    assert_eq!(server.refreshes.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn a_second_unauthorized_response_is_not_retried() {
    let server = catalog(StatusCode::UNAUTHORIZED, json!({}), true).await;
    assert!(codex::models(&server.subscription).await.is_err());
    assert_eq!(server.reads.load(Ordering::SeqCst), 2);
    assert_eq!(server.refreshes.load(Ordering::SeqCst), 1);
}

#[tokio::test]
#[ignore = "reads the live ChatGPT catalog using GUAC_SUBSCRIPTION_JSON; spends no model quota"]
async fn the_live_catalog_still_publishes_selectable_models() {
    let path = std::env::var("GUAC_SUBSCRIPTION_JSON")
        .expect("set GUAC_SUBSCRIPTION_JSON to a subscription credential file");
    let subscription = Subscription::open(path.into());
    let models = codex::models(&subscription).await.unwrap();
    assert!(!models.is_empty());
    eprintln!("selectable models: {}", models.join(", "));
}
