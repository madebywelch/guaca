use std::sync::{
    atomic::{AtomicBool, AtomicUsize, Ordering},
    Arc,
};

use axum::{
    extract::{Json, State},
    http::StatusCode,
    response::{IntoResponse, Response},
    routing::post,
    Router,
};
use guac_lib::mcp::{self, Dial};
use serde_json::{json, Value};

#[derive(Clone)]
struct Server {
    mode: Arc<AtomicUsize>,
    legacy: Arc<AtomicBool>,
    calls: Arc<AtomicUsize>,
    probes: Arc<AtomicUsize>,
}

async fn handle(State(server): State<Server>, Json(body): Json<Value>) -> Response {
    let method = body["method"].as_str().unwrap();
    let modern = body["params"].get("_meta").is_some();
    let legacy = server.legacy.load(Ordering::SeqCst);
    let result = match method {
        "server/discover" => {
            server.probes.fetch_add(1, Ordering::SeqCst);
            if legacy {
                return (StatusCode::BAD_REQUEST, "legacy").into_response();
            }
            json!({})
        }
        "initialize" if !legacy => {
            return (StatusCode::METHOD_NOT_ALLOWED, "modern").into_response()
        }
        "initialize" => json!({"protocolVersion":"2025-11-25"}),
        "notifications/initialized" => return StatusCode::ACCEPTED.into_response(),
        "tools/list" if modern == legacy => {
            return (StatusCode::METHOD_NOT_ALLOWED, "wrong protocol").into_response()
        }
        "tools/list" => {
            if body["params"].get("cursor").is_some() {
                match server.mode.load(Ordering::SeqCst) {
                    1 => return (StatusCode::INTERNAL_SERVER_ERROR, "page failed").into_response(),
                    2 => json!({"tools":[], "nextCursor":"next"}),
                    _ => json!({"tools":[{"name":"second"}]}),
                }
            } else {
                json!({"tools":[{"name":"first"}], "nextCursor":"next"})
            }
        }
        "tools/call" => {
            server.calls.fetch_add(1, Ordering::SeqCst);
            if modern == legacy {
                return (StatusCode::METHOD_NOT_ALLOWED, "wrong protocol").into_response();
            }
            json!({"content":[]})
        }
        _ => panic!("unexpected method"),
    };
    Json(json!({"jsonrpc":"2.0", "id":1, "result":result})).into_response()
}

async fn server(mode: usize) -> (String, Server) {
    let state = Server {
        mode: Arc::new(AtomicUsize::new(mode)),
        legacy: Arc::new(AtomicBool::new(false)),
        calls: Arc::new(AtomicUsize::new(0)),
        probes: Arc::new(AtomicUsize::new(0)),
    };
    let app = Router::new().route("/mcp", post(handle)).with_state(state.clone());
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = format!("http://{}/mcp", listener.local_addr().unwrap());
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    (address, state)
}

#[tokio::test]
async fn discovers_all_pages() {
    let (url, _) = server(0).await;
    let session = mcp::open(Dial::to(&url)).await.unwrap();
    let tools = mcp::list_tools(&session).await.unwrap();
    assert_eq!(tools.iter().map(|t| t.name.as_str()).collect::<Vec<_>>(), ["first", "second"]);
}

#[tokio::test]
async fn rejects_incomplete_or_repeated_pages() {
    for mode in [1, 2] {
        let (url, _) = server(mode).await;
        let session = mcp::open(Dial::to(&url)).await.unwrap();
        assert!(mcp::list_tools(&session).await.is_err());
    }
}

#[tokio::test]
async fn changed_era_is_reprobed_without_replaying_call() {
    let (url, server) = server(0).await;
    let session = mcp::open(Dial::to(&url)).await.unwrap();
    server.legacy.store(true, Ordering::SeqCst);
    assert!(mcp::call_tool(&session, "write", &json!({}), None).await.is_err());
    assert_eq!(server.calls.load(Ordering::SeqCst), 1);
    let session = mcp::open(Dial::to(&url)).await.unwrap();
    assert!(mcp::list_tools(&session).await.is_ok());
}

#[tokio::test]
async fn upgrading_legacy_reprobes_and_does_not_affect_other_endpoints() {
    let (other_url, other) = server(0).await;
    let (url, server) = server(0).await;
    server.legacy.store(true, Ordering::SeqCst);
    let legacy = mcp::open(Dial::to(&url)).await.unwrap();
    let other_session = mcp::open(Dial::to(&other_url)).await.unwrap();
    server.legacy.store(false, Ordering::SeqCst);
    assert!(mcp::list_tools(&legacy).await.is_err());
    let modern = mcp::open(Dial::to(&url)).await.unwrap();
    assert!(mcp::list_tools(&modern).await.is_ok());
    assert!(mcp::list_tools(&other_session).await.is_ok());
    mcp::open(Dial::to(&other_url)).await.unwrap();
    assert_eq!(other.probes.load(Ordering::SeqCst), 1);
}
