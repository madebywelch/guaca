mod live;

use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::Arc;

use axum::extract::ws::{Message, WebSocket, WebSocketUpgrade};
use axum::extract::{DefaultBodyLimit, Path, Query, RawQuery, State};
use axum::http::{header, HeaderMap, HeaderValue, Method, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::{any, get, post};
use axum::{Json, Router};
use serde::Deserialize;
use serde_json::{json, Value};
use tokio::sync::broadcast;
use tower_http::cors::CorsLayer;

use crate::commands::{AppState, Reach};
use crate::domain::attachment::MAX_FILE_BYTES;
use crate::domain::deployment::Deployment;
use crate::runtime::events::UiEvent;

const BACKLOG: usize = 2048;

const SESSION_SECONDS: u64 = 30 * 24 * 60 * 60;

const SESSION_COOKIE: &str = "guaca_session_v2";

const RETIRED_COOKIES: [&str; 2] = ["guaca_session", "guaca_session_legacy"];

const APP_ORIGINS: [&str; 5] = [
    "tauri://localhost",
    "http://tauri.localhost",
    "https://tauri.localhost",
    "http://localhost:1420",
    "http://127.0.0.1:1420",
];

pub struct Settings {
    pub root: PathBuf,
    pub bind: SocketAddr,
    pub token: String,
    pub web: Option<PathBuf>,
    pub origin: Option<String>,
    pub updater: Option<PathBuf>,
}

#[derive(Clone)]
struct Serving {
    state: Arc<AppState>,
    token: Arc<str>,
    sink: Arc<live::SocketSink>,
    updates: Arc<crate::updates::Checker>,
    updater: Option<Arc<std::path::Path>>,
    revocations: Arc<parking_lot::Mutex<PathBuf>>,
    revoked: broadcast::Sender<String>,
}

pub struct Bound {
    pub addr: SocketAddr,
    pub agents: usize,
    listener: tokio::net::TcpListener,
    app: Router,
    runtime: crate::runtime::Runtime,
}

const DRAIN_WINDOW: std::time::Duration = std::time::Duration::from_secs(20);
const _: () = assert!(
    DRAIN_WINDOW.as_millis() < crate::host::STOP_TIMEOUT.as_millis(),
    "DRAIN_WINDOW has to end before host::STOP_TIMEOUT kills the daemon"
);

impl Bound {
    pub async fn serve(self) -> Result<(), String> {
        let runtime = self.runtime;
        axum::serve(self.listener, self.app)
            .with_graceful_shutdown(async move {
                stopped().await;
                runtime.drain(DRAIN_WINDOW).await;
            })
            .await
            .map_err(|err| format!("the server stopped: {err}"))
    }
}

pub async fn serve(settings: Settings) -> Result<(), String> {
    bind(settings).await?.serve().await
}

pub async fn bind(settings: Settings) -> Result<Bound, String> {
    let (events, _) = broadcast::channel(BACKLOG);
    let sink = Arc::new(live::SocketSink::new(events.clone()));

    let paths = crate::boot::Paths::under(&settings.root);
    let booted = crate::boot::open(&paths, tokio::runtime::Handle::current(), sink.clone()).await?;

    for retired in ["legacy-sessions.json", "legacy-sessions.tmp"] {
        match std::fs::remove_file(settings.root.join(retired)) {
            Ok(()) => {}
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => {}
            Err(err) => tracing::warn!(%err, retired, "could not remove a retired session file"),
        }
    }

    let runtime = booted.runtime;
    let state = Arc::new(AppState {
        runtime: runtime.clone(),
        secret: Some(Arc::from(settings.token.as_str())),
        menubar: Arc::new(|_presence| {}),
        deployment: Deployment::Server,
        open_url: page_opener(events.clone()),
        reach: Reach::served(settings.origin.clone()),
        config_path: booted.config_path,
        downloads: settings.root.join("downloads"),
        subscription: booted.subscription,
        claude_signin: Default::default(),
        account: booted.account,
        catalog: Arc::new(crate::commands::Catalog::new()),
        artifacts: booted.artifacts,
        artifact_port: booted.artifact_port,
    });

    let token: Arc<str> = settings.token.into();
    let (revoked, _) = broadcast::channel(BACKLOG);
    let serving = Serving {
        state,
        token: token.clone(),
        sink,
        updates: Arc::new(crate::updates::Checker::default()),
        updater: settings.updater.map(Arc::from),
        revocations: Arc::new(parking_lot::Mutex::new(
            settings.root.join("session-revocations.json"),
        )),
        revoked,
    };

    let mut app = Router::new()
        .route("/health", get(health))
        .route("/v1/updates", get(update_status))
        .route("/v1/host", get(host_status))
        .route("/v1/host/update", post(host_update))
        .route("/v1/call", post(call))
        .route("/v1/events", get(events_socket))
        .route("/v1/auth/session", post(auth_session))
        .route("/v1/auth/logout", post(auth_logout))
        .merge(self::events(runtime.clone()))
        .route("/v1/file/:digest/:name", get(file))
        .route("/v1/upload", post(upload))
        .layer(DefaultBodyLimit::max(4 * MAX_FILE_BYTES as usize))
        .route(crate::oauth::CALLBACK_ROUTE, get(oauth_callback))
        .route("/v1/artifact/:id", get(artifact))
        .route(
            &format!("{}/:ticket/:sandbox/:port/*rest", crate::commands::SCREEN_ROUTE),
            any(screen).layer(axum::middleware::map_response(isolate_screen)),
        )
        .with_state(serving);

    if let Some(web) = settings.web.as_ref() {
        let index = web.join("index.html");
        app = app.fallback_service(
            tower_http::services::ServeDir::new(web)
                .fallback(tower_http::services::ServeFile::new(index)),
        );
        tracing::info!(web = %web.display(), "serving the app");
    }

    app = app.layer(
        CorsLayer::new()
            .allow_origin(APP_ORIGINS.map(HeaderValue::from_static))
            .allow_methods([Method::GET, Method::POST])
            .allow_headers([header::AUTHORIZATION, header::CONTENT_TYPE, header::RANGE])
            .expose_headers([header::CONTENT_RANGE, header::CONTENT_DISPOSITION]),
    );

    let listener = tokio::net::TcpListener::bind(settings.bind)
        .await
        .map_err(|err| format!("could not listen on {}: {err}", settings.bind))?;
    let addr = listener.local_addr().map_err(|err| err.to_string())?;
    tracing::info!(%addr, agents = booted.started, "guacad ready");
    if settings.web.is_some() {
        tracing::info!(
            url = %invitation(addr, &token),
            "open this in a browser, or the same path on the address a tunnel gives this box"
        );
    }

    Ok(Bound { addr, agents: booted.started, listener, app, runtime })
}

fn invitation(addr: SocketAddr, token: &str) -> String {
    if addr.ip().is_unspecified() {
        format!("http://localhost:{}/#token={token}", addr.port())
    } else {
        format!("http://{addr}/#token={token}")
    }
}

async fn stopped() {
    let interrupt = tokio::signal::ctrl_c();
    #[cfg(unix)]
    {
        let mut term = match tokio::signal::unix::signal(
            tokio::signal::unix::SignalKind::terminate(),
        ) {
            Ok(term) => term,
            Err(err) => {
                tracing::warn!(%err, "no SIGTERM handler; only interrupt will stop this cleanly");
                let _ = interrupt.await;
                return;
            }
        };
        tokio::select! {
            _ = interrupt => tracing::info!("interrupted"),
            _ = term.recv() => tracing::info!("asked to stop"),
        }
    }
    #[cfg(not(unix))]
    {
        let _ = interrupt.await;
    }
}

async fn health() -> Json<Value> {
    let mut value = crate::updates::metadata();
    value["status"] = json!("ok");
    value["service"] = json!("guacad");
    value["build"] = json!(crate::updates::BUILD);
    Json(value)
}

#[derive(Default, Deserialize)]
struct UpdateQuery {
    #[serde(default)]
    refresh: bool,
}

async fn update_status(
    State(serving): State<Serving>,
    headers: HeaderMap,
    Query(query): Query<UpdateQuery>,
) -> Response {
    if let Err(response) = authorized(&serving, &headers, None, &Method::GET) {
        return *response;
    }
    let channel = match serving.updater.as_deref() {
        Some(socket) => followed(socket).await,
        None => Some(crate::updates::Channel::Release),
    };
    Json(serving.updates.check(channel, query.refresh).await).into_response()
}

async fn followed(socket: &std::path::Path) -> Option<crate::updates::Channel> {
    #[cfg(unix)]
    let report = crate::updater::ask(socket, &crate::updater::Request::Status).await.ok()?;
    #[cfg(not(unix))]
    let report = {
        let _ = socket;
        Value::Null
    };
    Some(serde_json::from_value(report["channel"].clone()).unwrap_or_default())
}

async fn host_status(State(serving): State<Serving>, headers: HeaderMap) -> Response {
    if let Err(response) = authorized(&serving, &headers, None, &Method::GET) {
        return *response;
    }
    let Some(socket) = serving.updater.clone() else {
        return Json(json!({ "managed": false })).into_response();
    };
    relay(&socket, crate::updater::Request::Status).await
}

#[derive(Deserialize)]
struct HostUpdate {
    version: String,
    #[serde(default)]
    commit: Option<String>,
    #[serde(default, rename = "attemptId")]
    attempt_id: Option<String>,
}

async fn host_update(
    State(serving): State<Serving>,
    headers: HeaderMap,
    Json(body): Json<HostUpdate>,
) -> Response {
    if let Err(response) = authorized(&serving, &headers, None, &Method::POST) {
        return *response;
    }
    let Some(socket) = serving.updater.clone() else {
        return (
            StatusCode::NOT_FOUND,
            Json(json!({ "err": {
                "kind": "unmanaged",
                "message": "This host has no updater. Update it on the machine it runs on, using the update instructions.",
            }})),
        )
            .into_response();
    };
    relay(
        &socket,
        crate::updater::Request::Update {
            version: body.version,
            commit: body.commit,
            attempt_id: Some(body.attempt_id.unwrap_or_else(|| uuid::Uuid::new_v4().to_string())),
        },
    )
    .await
}

async fn relay(socket: &std::path::Path, request: crate::updater::Request) -> Response {
    #[cfg(unix)]
    let answer = crate::updater::ask(socket, &request).await;
    #[cfg(not(unix))]
    let answer: Result<Value, crate::updater::Failure> = {
        let _ = (socket, request);
        Err(crate::updater::Failure::unanswered(
            "An updater is reached over a Unix socket, which this system does not have.",
        ))
    };
    match answer {
        Ok(mut report) => {
            report["managed"] = json!(true);
            Json(report).into_response()
        }
        Err(failure) => (
            StatusCode::CONFLICT,
            Json(json!({ "err": {
                "kind": "updater",
                "reason": failure.reason,
                "message": failure.message,
            }})),
        )
            .into_response(),
    }
}

#[derive(Deserialize)]
struct Call {
    name: String,
    #[serde(default)]
    args: Value,
    #[serde(default)]
    client: Option<crate::ipc::Client>,
}

fn page_opener(events: broadcast::Sender<UiEvent>) -> crate::commands::OpenUrl {
    Arc::new(move |url: &str| {
        let client = CALLER.try_with(Clone::clone).ok().flatten();
        if events.send(UiEvent::OpenUrl { url: url.to_string(), client }).is_err() {
            return Err("nobody has this workspace open in a browser, so there is nowhere to \
                        show the sign-in page. Open the workspace and try again"
                .to_string());
        }
        Ok(())
    })
}

tokio::task_local! {
    static CALLER: Option<String>;
}

async fn call(
    State(serving): State<Serving>,
    headers: HeaderMap,
    Json(body): Json<Call>,
) -> Response {
    if let Err(refused) = authorized(&serving, &headers, None, &Method::POST) {
        return *refused;
    }
    if let Some(origin) = reached_at(&headers) {
        serving.state.reach.note(origin);
    }
    let page = body.client.as_ref().and_then(crate::ipc::Client::page);
    match CALLER.scope(page, crate::ipc::dispatch(&serving.state, &body.name, body.args)).await {
        Ok(value) => (StatusCode::OK, Json(json!({ "ok": value }))).into_response(),
        Err(refused) => {
            if matches!(
                refused,
                crate::ipc::Refused::Unknown(_) | crate::ipc::Refused::Arguments { .. }
            ) {
                tracing::warn!(
                    command = %body.name,
                    client = body.client.as_ref().map(|c| c.version.as_str()).unwrap_or("unstated"),
                    client_build = body.client.as_ref().and_then(|c| c.build()).unwrap_or("unstated"),
                    host = env!("CARGO_PKG_VERSION"),
                    host_build = crate::updates::BUILD,
                    "a client of another build called a command this host cannot answer"
                );
            }
            let status =
                StatusCode::from_u16(refused.status()).unwrap_or(StatusCode::INTERNAL_SERVER_ERROR);
            (status, Json(json!({ "err": refused.body_for(body.client.as_ref()) }))).into_response()
        }
    }
}

fn reached_at(headers: &HeaderMap) -> Option<String> {
    let text = |name: &str| headers.get(name).and_then(|v| v.to_str().ok()).map(str::trim);
    let host = text("x-forwarded-host")
        .or_else(|| text("host"))
        .and_then(|h| h.split(',').next())
        .map(str::trim)
        .filter(|h| !h.is_empty())?;
    let scheme = text("x-forwarded-proto")
        .and_then(|p| p.split(',').next())
        .map(str::trim)
        .filter(|p| *p == "https" || *p == "http")
        .unwrap_or("http");
    Some(format!("{scheme}://{host}"))
}

#[derive(Deserialize)]
struct Upload {
    name: String,
}

async fn upload(
    State(serving): State<Serving>,
    headers: HeaderMap,
    Query(Upload { name }): Query<Upload>,
    body: axum::body::Bytes,
) -> Response {
    if let Err(refused) = authorized(&serving, &headers, None, &Method::POST) {
        return *refused;
    }
    match serving.state.runtime.files().put(&name, &body) {
        Ok(attachment) => (StatusCode::OK, Json(json!({ "ok": attachment }))).into_response(),
        Err(err) => (
            StatusCode::UNPROCESSABLE_ENTITY,
            Json(json!({ "err": { "kind": "file", "message": err.to_string() } })),
        )
            .into_response(),
    }
}

async fn oauth_callback(State(serving): State<Serving>, RawQuery(query): RawQuery) -> Response {
    let fields = crate::oauth::parse_query(query.as_deref().unwrap_or_default());
    let landing = match serving.state.reach.callbacks() {
        Some(callbacks) => crate::oauth::Landing::Served { origin: String::new(), callbacks },
        None => return page(404, "Not the sign-in."),
    };
    match landing.deliver(fields).await {
        Some((status, message)) => page(status, message),
        None => page(404, "Not a sign-in this workspace is waiting for."),
    }
}

async fn artifact(
    State(serving): State<Serving>,
    Query(ticket): Query<Ticket>,
    Path(id): Path<String>,
) -> Response {
    let expected = crate::commands::artifact_ticket(&serving.token, &id);
    if !ticket.token.as_deref().is_some_and(|ticket| same(ticket, &expected)) {
        return page(401, "That is not a ticket for this page.");
    }
    let (status, content_type, body) = crate::artifact::page_for(&serving.state.artifacts, &id);
    let mut response =
        (StatusCode::from_u16(status).unwrap_or(StatusCode::OK), body).into_response();
    let set = response.headers_mut();
    for (name, value) in crate::artifact::response_headers(content_type) {
        if let (Ok(name), Ok(value)) = (
            axum::http::header::HeaderName::from_bytes(name.as_bytes()),
            axum::http::header::HeaderValue::from_str(value),
        ) {
            set.insert(name, value);
        }
    }
    response
}

async fn screen(
    State(serving): State<Serving>,
    Path((ticket, sandbox, port, rest)): Path<(String, String, String, String)>,
    request: axum::extract::Request,
) -> Response {
    let Some(secret) = serving.state.secret.as_deref() else {
        return page(404, "No screen is reachable here.");
    };
    if !same(&ticket, &crate::commands::screen_ticket(secret, &sandbox)) {
        return page(404, "That is not a screen this workspace is showing.");
    }
    relay_screen(serving.state.runtime.viewer_port(), &sandbox, &port, &rest, request).await
}

async fn relay_screen(
    viewer: u16,
    sandbox: &str,
    port: &str,
    rest: &str,
    mut request: axum::extract::Request,
) -> Response {
    let query = request.uri().query().map(|q| format!("?{q}")).unwrap_or_default();
    let target = format!("/{sandbox}/{port}/{rest}{query}");

    let mut head = format!("{} {target} HTTP/1.1\r\nhost: 127.0.0.1\r\n", request.method());
    let upgrade = request
        .headers()
        .get(axum::http::header::UPGRADE)
        .and_then(|v| v.to_str().ok())
        .is_some_and(|v| v.eq_ignore_ascii_case("websocket"));
    for (name, value) in request.headers() {
        let name = name.as_str();
        if matches!(
            name,
            "host" | "connection" | "keep-alive" | "referer" | "cookie" | "authorization"
        ) {
            continue;
        }
        if let Ok(value) = value.to_str() {
            head.push_str(&format!("{name}: {value}\r\n"));
        }
    }
    head.push_str(if upgrade {
        "connection: Upgrade\r\n\r\n"
    } else {
        "connection: close\r\n\r\n"
    });

    let mut upstream = match tokio::net::TcpStream::connect(("127.0.0.1", viewer)).await {
        Ok(upstream) => upstream,
        Err(err) => return page(502, &format!("The computer viewer is not answering: {err}")),
    };
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    if upstream.write_all(head.as_bytes()).await.is_err() {
        return page(502, "The computer viewer hung up.");
    }

    let mut answered = Vec::new();
    let mut byte = [0u8; 1];
    while !answered.ends_with(b"\r\n\r\n") && answered.len() < 32 * 1024 {
        match upstream.read(&mut byte).await {
            Ok(0) => break,
            Ok(_) => answered.push(byte[0]),
            Err(_) => break,
        }
    }
    let Some((status, upstream_headers)) = parse_head(&answered) else {
        return page(502, "The computer viewer answered nothing this daemon could read.");
    };

    if status == 101 {
        let taken = hyper::upgrade::on(&mut request);
        tokio::spawn(async move {
            match taken.await {
                Ok(client) => {
                    let mut client = hyper_util::rt::TokioIo::new(client);
                    let _ = tokio::io::copy_bidirectional(&mut client, &mut upstream).await;
                }
                Err(err) => tracing::debug!(%err, "a screen socket was not handed over"),
            }
        });
        let mut response = Response::new(axum::body::Body::empty());
        *response.status_mut() = StatusCode::SWITCHING_PROTOCOLS;
        apply_headers(response.headers_mut(), &upstream_headers);
        response.headers_mut().insert(header::CONNECTION, HeaderValue::from_static("Upgrade"));
        return response;
    }

    let mut body = Vec::new();
    let _ = upstream.read_to_end(&mut body).await;
    let chunked = upstream_headers
        .iter()
        .any(|(k, v)| k == "transfer-encoding" && v.to_ascii_lowercase().contains("chunked"));
    let body = if chunked { dechunk(&body) } else { body };
    let mut response =
        (StatusCode::from_u16(status).unwrap_or(StatusCode::BAD_GATEWAY), body).into_response();
    apply_headers(response.headers_mut(), &upstream_headers);
    response
}

async fn isolate_screen(mut response: Response) -> Response {
    let headers = response.headers_mut();
    headers
        .insert(header::CONTENT_SECURITY_POLICY, HeaderValue::from_static("sandbox allow-scripts"));
    headers.insert(header::ACCESS_CONTROL_ALLOW_ORIGIN, HeaderValue::from_static("*"));
    headers.insert(header::REFERRER_POLICY, HeaderValue::from_static("no-referrer"));
    response
}

fn parse_head(head: &[u8]) -> Option<(u16, Vec<(String, String)>)> {
    let text = String::from_utf8_lossy(head);
    let mut lines = text.split("\r\n");
    let status: u16 = lines.next()?.split(' ').nth(1)?.parse().ok()?;
    let headers = lines
        .filter_map(|line| line.split_once(':'))
        .map(|(k, v)| (k.trim().to_ascii_lowercase(), v.trim().to_string()))
        .collect();
    Some((status, headers))
}

fn apply_headers(set: &mut HeaderMap, headers: &[(String, String)]) {
    for (name, value) in headers {
        if matches!(
            name.as_str(),
            "content-length" | "transfer-encoding" | "connection" | "keep-alive"
        ) {
            continue;
        }
        if let (Ok(name), Ok(value)) = (
            axum::http::header::HeaderName::from_bytes(name.as_bytes()),
            axum::http::header::HeaderValue::from_str(value),
        ) {
            set.append(name, value);
        }
    }
}

fn dechunk(body: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(body.len());
    let mut at = 0;
    while at < body.len() {
        let Some(line_end) = body[at..].windows(2).position(|w| w == b"\r\n") else { break };
        let size_text = String::from_utf8_lossy(&body[at..at + line_end]);
        let size = usize::from_str_radix(size_text.split(';').next().unwrap_or("0").trim(), 16)
            .unwrap_or(0);
        at += line_end + 2;
        if size == 0 {
            break;
        }
        let end = (at + size).min(body.len());
        out.extend_from_slice(&body[at..end]);
        at = end + 2;
    }
    out
}

fn page(status: u16, message: &str) -> Response {
    let body = format!(
        "<!doctype html><meta charset=\"utf-8\"><title>Guaca</title>\
         <body style=\"font:16px system-ui;padding:3rem;color:#1c1c1c\">{message}</body>"
    );
    (
        StatusCode::from_u16(status).unwrap_or(StatusCode::OK),
        [(axum::http::header::CONTENT_TYPE, "text/html; charset=utf-8")],
        body,
    )
        .into_response()
}

#[derive(Deserialize)]
struct Ticket {
    token: Option<String>,
}

#[derive(Deserialize)]
struct FileRequest {
    token: Option<String>,
    #[serde(default)]
    download: bool,
}

async fn events_socket(
    State(serving): State<Serving>,
    headers: HeaderMap,
    Query(ticket): Query<Ticket>,
    upgrade: WebSocketUpgrade,
) -> Response {
    let shell = app_origin(&headers);
    let revoked = serving.revoked.subscribe();
    if !shell && !proves_same_origin(&headers) {
        return StatusCode::FORBIDDEN.into_response();
    }
    let proof = if shell {
        authorized_by_token(&serving, &headers, ticket.token.as_deref())
    } else {
        authorized(&serving, &headers, ticket.token.as_deref(), &Method::GET)
    };
    if let Err(refused) = proof {
        return *refused;
    }
    let (feed, snapshot) = serving.sink.tune_in();
    let session = if shell {
        None
    } else {
        match read_session(&serving, &headers) {
            SessionState::Active { id, expires } => Some((serving, headers, id, expires, revoked)),
            _ if authorized_by_token(&serving, &headers, ticket.token.as_deref()).is_ok() => None,
            _ => return StatusCode::UNAUTHORIZED.into_response(),
        }
    };
    upgrade.on_upgrade(move |socket| pump(socket, feed, snapshot, session))
}

async fn pump(
    mut socket: WebSocket,
    mut feed: broadcast::Receiver<UiEvent>,
    snapshot: String,
    session: Option<(Serving, HeaderMap, String, u64, broadcast::Receiver<String>)>,
) {
    if session.as_ref().is_some_and(|(serving, headers, ..)| {
        !matches!(read_session(serving, headers), SessionState::Active { .. })
    }) {
        let _ = socket.close().await;
        return;
    }
    if socket.send(Message::Text(snapshot)).await.is_err() {
        return;
    }
    let (check, expiry, revocations) = match session {
        Some((serving, headers, id, expires, receiver)) => {
            (Some((serving, headers)), Some(expires), Some((id, receiver)))
        }
        None => (None, None, None),
    };
    let ended = async move {
        let Some((id, mut receiver)) = revocations else {
            return std::future::pending::<()>().await;
        };
        loop {
            match receiver.recv().await {
                Ok(revoked) if revoked != id => continue,
                _ => return,
            }
        }
    };
    let expired = async move {
        match expiry {
            Some(expires) => {
                tokio::time::sleep(std::time::Duration::from_secs(expires.saturating_sub(now())))
                    .await
            }
            None => std::future::pending::<()>().await,
        }
    };
    tokio::pin!(ended, expired);
    let mut heartbeat = tokio::time::interval(std::time::Duration::from_secs(20));
    let mut heard = tokio::time::Instant::now();
    loop {
        tokio::select! {
            biased;
            _ = &mut ended => { let _ = socket.close().await; return; },
            _ = &mut expired => { let _ = socket.close().await; return; },
            event = feed.recv() => match event {
                Ok(event) => {
                    if check.as_ref().is_some_and(|(serving, headers)| !matches!(read_session(serving, headers), SessionState::Active { .. })) {
                        let _ = socket.close().await;
                        return;
                    }
                    let Ok(text) = serde_json::to_string(&event) else { continue };
                    if socket.send(Message::Text(text)).await.is_err() {
                        return;
                    }
                }
                Err(broadcast::error::RecvError::Lagged(missed)) => {
                    tracing::warn!(missed, "a client fell behind the event stream");
                    let _ = socket.close().await;
                    return;
                }
                Err(broadcast::error::RecvError::Closed) => return,
            },
            incoming = socket.recv() => match incoming {
                Some(Ok(_)) => { heard = tokio::time::Instant::now(); },
                _ => return,
            },
            _ = heartbeat.tick() => {
                if heard.elapsed() > std::time::Duration::from_secs(60) { return; }
                if socket.send(Message::Ping(Vec::new())).await.is_err() { return; }
            },
        }
    }
}

async fn file(
    State(serving): State<Serving>,
    headers: HeaderMap,
    Query(ticket): Query<FileRequest>,
    Path((digest, name)): Path<(String, String)>,
) -> Response {
    if let Err(refused) = authorized(&serving, &headers, ticket.token.as_deref(), &Method::GET) {
        return *refused;
    }
    let range = headers
        .get(axum::http::header::RANGE)
        .and_then(|value| value.to_str().ok())
        .map(str::to_string);

    match serving.state.runtime.files().serve(&format!("{digest}/{name}"), range.as_deref()) {
        Ok(served) => {
            let status = StatusCode::from_u16(served.status).unwrap_or(StatusCode::PARTIAL_CONTENT);
            let mut response = (status, served.body).into_response();
            let headers = response.headers_mut();
            if let Ok(mime) = served.mime.parse() {
                headers.insert(axum::http::header::CONTENT_TYPE, mime);
            }
            headers.insert(header::X_CONTENT_TYPE_OPTIONS, HeaderValue::from_static("nosniff"));
            headers.insert(header::REFERRER_POLICY, HeaderValue::from_static("no-referrer"));
            headers.insert(
                header::CONTENT_SECURITY_POLICY,
                HeaderValue::from_static("sandbox; default-src 'none'; form-action 'none'"),
            );
            if ticket.download || served.mime == "text/html" {
                headers.insert(header::CONTENT_DISPOSITION, HeaderValue::from_static("attachment"));
            }
            if let Some(range) = served.content_range.and_then(|range| range.parse().ok()) {
                headers.insert(axum::http::header::CONTENT_RANGE, range);
            }
            if let Ok(cache) = "private, max-age=31536000, immutable".parse() {
                headers.insert(axum::http::header::CACHE_CONTROL, cache);
            }
            response
        }
        Err(err) => (
            StatusCode::NOT_FOUND,
            Json(json!({ "err": { "kind": "file", "message": err.to_string() } })),
        )
            .into_response(),
    }
}

pub fn events<S: Clone + Send + Sync + 'static>(runtime: crate::runtime::Runtime) -> Router<S> {
    Router::new()
        .route(
            "/events/:service/:topic",
            post(routine_event).layer(DefaultBodyLimit::max(crate::webhook::MOST_BODY_BYTES)),
        )
        .with_state(runtime)
}

async fn routine_event(
    State(runtime): State<crate::runtime::Runtime>,
    Path((service, topic)): Path<(String, String)>,
    headers: HeaderMap,
    body: axum::body::Bytes,
) -> Response {
    let secret = runtime.config().webhook.secret;
    let bearer = headers
        .get(header::AUTHORIZATION)
        .and_then(|h| h.to_str().ok())
        .and_then(|h| h.split_once(' '))
        .filter(|(scheme, _)| scheme.eq_ignore_ascii_case("bearer"))
        .map(|(_, token)| token.trim());
    if secret.is_empty() || !bearer.is_some_and(|token| same(token, secret.expose())) {
        return (
            StatusCode::UNAUTHORIZED,
            Json(json!({
                "error": "Send Guaca's webhook secret as `Authorization: Bearer <secret>`. It is on \
                          the page of any routine that waits on an event, under Show secret."
            })),
        )
            .into_response();
    }
    let Some(event) = crate::domain::routine::EventTrigger::parse(&format!("{service}/{topic}"))
    else {
        return StatusCode::BAD_REQUEST.into_response();
    };
    let delivery_id = match crate::webhook::delivery_id([
        headers.get("idempotency-key").map(|value| value.as_bytes()),
        headers.get("webhook-id").map(|value| value.as_bytes()),
        headers.get("x-github-delivery").map(|value| value.as_bytes()),
    ]) {
        Ok(id) => id,
        Err(message) => {
            return (StatusCode::BAD_REQUEST, Json(json!({"error": message}))).into_response()
        }
    };
    let delivery = match runtime.deliver_event(
        &event,
        (!body.is_empty()).then(|| String::from_utf8_lossy(&body).into_owned()),
        &delivery_id,
    ) {
        Ok(delivery) => delivery,
        Err(_) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(json!({"error":"Could not deliver the routine event"})),
            )
                .into_response()
        }
    };
    let mut answer = serde_json::to_value(delivery).unwrap_or_default();
    if delivery.listening > 0 {
        return (StatusCode::OK, Json(answer)).into_response();
    }
    let (status, said) = if delivery.paused > 0 {
        (StatusCode::CONFLICT, "The routine is there, but its agent is paused.".to_string())
    } else {
        (
            StatusCode::NOT_FOUND,
            format!(
                "No active routine waits on {}/{}. Add one in Guaca, under the agent that \
                 should handle it, and check the spelling of both names.",
                event.service, event.topic
            ),
        )
    };
    answer["error"] = json!(said);
    (status, Json(answer)).into_response()
}

fn authority(url: &str) -> Option<(&str, Option<u16>)> {
    let rest = match url.split_once("://") {
        Some((scheme, rest)) => match scheme.to_ascii_lowercase().as_str() {
            "http" | "https" => rest,
            _ => return None,
        },
        None => url,
    };
    let host_port = rest.split('/').next()?.rsplit('@').next()?;
    if host_port.is_empty() {
        return None;
    }
    if let Some(inner) = host_port.strip_prefix('[') {
        let end = inner.find(']')?;
        let port = inner[end + 1..].strip_prefix(':').and_then(|p| p.parse().ok());
        return Some((&inner[..end], port));
    }
    match host_port.rsplit_once(':') {
        Some((host, port)) => port.parse::<u16>().ok().map(|port| (host, Some(port))),
        None => Some((host_port, None)),
    }
}

fn over_https(headers: &HeaderMap) -> bool {
    if let Some(proto) = headers.get("x-forwarded-proto").and_then(|h| h.to_str().ok()) {
        if proto.split(',').next().is_some_and(|proto| proto.trim().eq_ignore_ascii_case("https")) {
            return true;
        }
    }
    headers.get(header::ORIGIN).and_then(|h| h.to_str().ok()).is_some_and(|origin| {
        origin.split_once("://").is_some_and(|(scheme, _)| scheme.eq_ignore_ascii_case("https"))
    })
}

fn cookie_name(headers: &HeaderMap) -> String {
    named_cookie(headers, SESSION_COOKIE)
}

fn named_cookie(headers: &HeaderMap, prefix: &str) -> String {
    headers
        .get(header::HOST)
        .and_then(|value| value.to_str().ok())
        .and_then(authority)
        .and_then(|(_, port)| port)
        .map_or_else(|| prefix.to_string(), |port| format!("{prefix}_{port}"))
}

fn session_cookie(headers: &HeaderMap, value: &str, max_age: u64) -> HeaderValue {
    make_cookie(headers, &cookie_name(headers), value, max_age)
}

fn make_cookie(headers: &HeaderMap, name: &str, value: &str, max_age: u64) -> HeaderValue {
    let secure = if over_https(headers) { "; Secure" } else { "" };
    HeaderValue::from_str(&format!(
        "{}={value}; HttpOnly; SameSite=Strict; Path=/; Max-Age={max_age}{secure}",
        name
    ))
    .expect("a session cookie is a header value")
}

#[derive(Debug, PartialEq, Eq)]
enum SessionState {
    Invalid,
    Active { id: String, expires: u64 },
    Expired,
    Revoked,
}

fn now() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap_or_default().as_secs()
}

fn session_key(token: &str) -> ring::hmac::Key {
    let salt = ring::hmac::Key::new(ring::hmac::HMAC_SHA256, &[0; 32]);
    let prk = ring::hmac::sign(&salt, token.as_bytes());
    let key = ring::hmac::Key::new(ring::hmac::HMAC_SHA256, prk.as_ref());
    let okm = ring::hmac::sign(&key, b"guaca browser session v1\x01");
    ring::hmac::Key::new(ring::hmac::HMAC_SHA256, okm.as_ref())
}

fn issue_session(token: &str, issued: u64) -> String {
    sign_session(token, &uuid::Uuid::new_v4().simple().to_string(), issued)
}

fn sign_session(token: &str, id: &str, issued: u64) -> String {
    use base64::Engine;
    let payload = format!("{id}.{issued}.{}", issued + SESSION_SECONDS);
    let signature = ring::hmac::sign(&session_key(token), payload.as_bytes());
    format!(
        "{}.{}",
        payload,
        base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(signature.as_ref())
    )
}

fn durable_write(path: &std::path::Path, bytes: &[u8]) -> Result<(), String> {
    use std::io::Write;
    let temp = path.with_extension("tmp");
    let mut file = std::fs::File::create(&temp).map_err(|e| e.to_string())?;
    file.write_all(bytes).and_then(|_| file.sync_all()).map_err(|e| e.to_string())?;
    std::fs::rename(temp, path).map_err(|e| e.to_string())?;
    std::fs::File::open(path.parent().ok_or("missing data directory")?)
        .and_then(|directory| directory.sync_all())
        .map_err(|e| e.to_string())
}

fn session_state(
    token: &str,
    value: &str,
    time: u64,
    revoked: &std::collections::HashMap<String, u64>,
) -> SessionState {
    use base64::Engine;
    let Some((payload, signature)) = value.rsplit_once('.') else { return SessionState::Invalid };
    let Ok(signature) = base64::engine::general_purpose::URL_SAFE_NO_PAD.decode(signature) else {
        return SessionState::Invalid;
    };
    if ring::hmac::verify(&session_key(token), payload.as_bytes(), &signature).is_err() {
        return SessionState::Invalid;
    }
    let mut parts = payload.split('.');
    let (Some(id), Some(issued), Some(expires), None) =
        (parts.next(), parts.next(), parts.next(), parts.next())
    else {
        return SessionState::Invalid;
    };
    let (Ok(issued), Ok(expires)) = (issued.parse::<u64>(), expires.parse::<u64>()) else {
        return SessionState::Invalid;
    };
    if uuid::Uuid::parse_str(id).is_err()
        || issued > time
        || issued.checked_add(SESSION_SECONDS) != Some(expires)
    {
        return SessionState::Invalid;
    }
    if time >= expires {
        return SessionState::Expired;
    }
    if revoked.contains_key(id) {
        return SessionState::Revoked;
    }
    SessionState::Active { id: id.to_string(), expires }
}

fn cookie_value(headers: &HeaderMap) -> Option<&str> {
    let wanted = cookie_name(headers);
    headers.get(header::COOKIE)?.to_str().ok()?.split(';').find_map(|part| {
        let (name, value) = part.trim().split_once('=')?;
        (name.trim() == wanted).then_some(value.trim())
    })
}

fn revocations(path: &std::path::Path) -> Result<std::collections::HashMap<String, u64>, String> {
    match std::fs::read(path) {
        Ok(bytes) => serde_json::from_slice(&bytes).map_err(|e| e.to_string()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Default::default()),
        Err(e) => Err(e.to_string()),
    }
}

fn read_session(serving: &Serving, headers: &HeaderMap) -> SessionState {
    let Some(value) = cookie_value(headers) else { return SessionState::Invalid };
    let path = serving.revocations.lock();
    let Ok(revoked) = revocations(&path) else { return SessionState::Invalid };
    session_state(&serving.token, value, now(), &revoked)
}

fn proves_same_origin(headers: &HeaderMap) -> bool {
    match headers.get("sec-fetch-site").and_then(|h| h.to_str().ok()) {
        Some(site) => site.eq_ignore_ascii_case("same-origin") || site.eq_ignore_ascii_case("none"),
        None => same_origin(headers),
    }
}

fn app_origin(headers: &HeaderMap) -> bool {
    headers
        .get(header::ORIGIN)
        .and_then(|h| h.to_str().ok())
        .is_some_and(|origin| APP_ORIGINS.iter().any(|known| origin.eq_ignore_ascii_case(known)))
}

fn same_origin(headers: &HeaderMap) -> bool {
    let Some(origin) = headers.get(header::ORIGIN).and_then(|h| h.to_str().ok()) else {
        return true;
    };
    let Some((origin_host, origin_port)) = authority(origin) else {
        return false;
    };
    let candidates = headers
        .get("x-forwarded-host")
        .and_then(|h| h.to_str().ok())
        .into_iter()
        .chain(headers.get(header::HOST).and_then(|h| h.to_str().ok()));
    let default_port =
        if origin.split_once("://").is_some_and(|(scheme, _)| scheme.eq_ignore_ascii_case("https"))
        {
            443
        } else {
            80
        };
    candidates.filter_map(authority).any(|(host, port)| {
        host.eq_ignore_ascii_case(origin_host)
            && origin_port.unwrap_or(default_port) == port.unwrap_or(default_port)
    })
}

async fn auth_session(State(serving): State<Serving>, headers: HeaderMap) -> Response {
    let presented = headers
        .get(header::AUTHORIZATION)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.strip_prefix("Bearer "))
        .map(str::trim);

    match presented {
        Some(token) if same(token, &serving.token) => {
            let mut response = Json(json!({ "ok": "signed in" })).into_response();
            response.headers_mut().insert(
                header::SET_COOKIE,
                session_cookie(&headers, &issue_session(&serving.token, now()), SESSION_SECONDS),
            );
            response
        }
        _ => (
            StatusCode::UNAUTHORIZED,
            Json(json!({ "err": {
                "kind": "unauthorized",
                "message": "that token was not accepted",
            }})),
        )
            .into_response(),
    }
}

async fn auth_logout(State(serving): State<Serving>, headers: HeaderMap) -> Response {
    if !proves_same_origin(&headers) {
        return (
            StatusCode::FORBIDDEN,
            Json(json!({ "err": {
                "kind": "forbidden",
                "message": "this request was refused because it did not come from the same \
                            origin",
            }})),
        )
            .into_response();
    }
    let path = serving.revocations.lock();
    let result = (|| -> Result<Option<String>, String> {
        let mut revoked = revocations(&path)?;
        if let Some(value) = cookie_value(&headers) {
            if let SessionState::Active { id, expires } =
                session_state(&serving.token, value, now(), &revoked)
            {
                revoked.retain(|_, expiry| *expiry > now());
                revoked.insert(id.clone(), expires);
                let bytes = serde_json::to_vec(&revoked).map_err(|e| e.to_string())?;
                durable_write(&path, &bytes)?;
                return Ok(Some(id));
            }
        }
        Ok(None)
    })();
    match result {
        Ok(Some(id)) => {
            let _ = serving.revoked.send(id);
        }
        Ok(None) => {}
        Err(_) => return StatusCode::INTERNAL_SERVER_ERROR.into_response(),
    }
    let mut response = Json(json!({ "ok": "signed out" })).into_response();
    response.headers_mut().insert(header::SET_COOKIE, session_cookie(&headers, "", 0));
    for prefix in RETIRED_COOKIES {
        response.headers_mut().append(
            header::SET_COOKIE,
            make_cookie(&headers, &named_cookie(&headers, prefix), "", 0),
        );
    }
    response
}

fn authorized(
    serving: &Serving,
    headers: &HeaderMap,
    ticket: Option<&str>,
    method: &Method,
) -> Result<(), Box<Response>> {
    if matches!(read_session(serving, headers), SessionState::Active { .. }) {
        let state_changing =
            *method == Method::POST || *method == Method::PUT || *method == Method::DELETE;
        if state_changing && !proves_same_origin(headers) {
            return Err(Box::new(
                (
                    StatusCode::FORBIDDEN,
                    Json(json!({ "err": {
                        "kind": "forbidden",
                        "message": "this request was refused because it did not come \
                                    from the same origin. A state-changing request \
                                    authenticated by cookie must prove same-origin",
                    }})),
                )
                    .into_response(),
            ));
        }
        return Ok(());
    }

    authorized_by_token(serving, headers, ticket)
}

fn authorized_by_token(
    serving: &Serving,
    headers: &HeaderMap,
    ticket: Option<&str>,
) -> Result<(), Box<Response>> {
    let presented = headers
        .get(axum::http::header::AUTHORIZATION)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.strip_prefix("Bearer "))
        .map(str::trim)
        .or(ticket);

    match presented {
        Some(token) if same(token, &serving.token) => Ok(()),
        _ => Err(Box::new(
            (
                StatusCode::UNAUTHORIZED,
                Json(json!({ "err": {
                    "kind": "unauthorized",
                    "message": "this workspace needs the token it printed when it started. Copy it \
                                from the box's logs, or from the token file beside its settings",
                }})),
            )
                .into_response(),
        )),
    }
}

fn same(a: &str, b: &str) -> bool {
    let (a, b) = (a.as_bytes(), b.as_bytes());
    if a.len() != b.len() {
        return false;
    }
    a.iter().zip(b).fold(0u8, |differs, (x, y)| differs | (x ^ y)) == 0
}

#[cfg(test)]
mod tests {
    #[test]
    fn session_transitions_are_explicit_and_bound_to_the_bearer() {
        use super::{issue_session, session_state, SessionState, SESSION_SECONDS};
        let mut revoked = std::collections::HashMap::new();
        let first = issue_session("token", 100);
        let second = issue_session("token", 100);
        assert_ne!(first, second);
        assert_eq!(session_state("token", "token", 100, &revoked), SessionState::Invalid);
        assert_eq!(session_state("other", &first, 100, &revoked), SessionState::Invalid);
        assert_eq!(session_state("token", &first, 99, &revoked), SessionState::Invalid);
        assert_eq!(
            session_state("token", &first, 100 + SESSION_SECONDS, &revoked),
            SessionState::Expired
        );
        let SessionState::Active { id, expires } = session_state("token", &first, 100, &revoked)
        else {
            panic!("issued session must be active")
        };
        revoked.insert(id, expires);
        assert_eq!(session_state("token", &first, 101, &revoked), SessionState::Revoked);
        assert!(matches!(
            session_state("token", &second, 101, &revoked),
            SessionState::Active { .. }
        ));
        assert_eq!(session_state("token", "invalid", 101, &revoked), SessionState::Invalid);
    }
    use super::*;

    #[tokio::test]
    async fn a_sign_in_page_is_addressed_to_the_window_that_asked() {
        let (events, mut heard) = broadcast::channel(4);
        let open = page_opener(events);
        CALLER
            .scope(Some("page-a".to_string()), async { open("https://a.example").unwrap() })
            .await;
        open("https://b.example").unwrap();
        for (url, client) in [("https://a.example", Some("page-a")), ("https://b.example", None)] {
            match heard.recv().await.unwrap() {
                UiEvent::OpenUrl { url: sent, client: to } => {
                    assert_eq!(sent, url);
                    assert_eq!(to.as_deref(), client);
                }
                other => panic!("expected a sign-in page, got {other:?}"),
            }
        }
    }

    #[test]
    fn an_invitation_keeps_the_token_out_of_every_log_but_this_one() {
        let addr: SocketAddr = "127.0.0.1:8787".parse().unwrap();
        let url = invitation(addr, "abc123");
        assert_eq!(url, "http://127.0.0.1:8787/#token=abc123");
        assert!(!url.contains('?'), "{url}");

        let every: SocketAddr = "0.0.0.0:8787".parse().unwrap();
        assert_eq!(invitation(every, "abc123"), "http://localhost:8787/#token=abc123");
        let every6: SocketAddr = "[::]:8787".parse().unwrap();
        assert_eq!(invitation(every6, "abc123"), "http://localhost:8787/#token=abc123");
    }

    #[test]
    fn the_origin_a_call_arrived_at_is_read_as_the_browser_spelled_it() {
        let mut headers = HeaderMap::new();
        headers.insert("host", "127.0.0.1:8787".parse().unwrap());
        assert_eq!(reached_at(&headers).as_deref(), Some("http://127.0.0.1:8787"));

        headers.insert("x-forwarded-host", "guaca.example.com".parse().unwrap());
        headers.insert("x-forwarded-proto", "https".parse().unwrap());
        assert_eq!(reached_at(&headers).as_deref(), Some("https://guaca.example.com"));

        headers.insert("x-forwarded-proto", "gopher".parse().unwrap());
        assert_eq!(reached_at(&headers).as_deref(), Some("http://guaca.example.com"));

        assert_eq!(reached_at(&HeaderMap::new()), None);
    }

    #[test]
    fn a_viewer_head_is_read_for_its_status_and_headers() {
        let (status, headers) = parse_head(
            b"HTTP/1.1 101 Switching Protocols\r\nUpgrade: websocket\r\nSec-WebSocket-Accept: abc\r\n\r\n",
        )
        .unwrap();
        assert_eq!(status, 101);
        assert!(headers.contains(&("upgrade".to_string(), "websocket".to_string())));
        assert!(headers.contains(&("sec-websocket-accept".to_string(), "abc".to_string())));
        assert!(parse_head(b"nonsense").is_none());
    }

    #[test]
    fn a_chunked_body_is_joined_and_the_framing_dropped() {
        assert_eq!(dechunk(b"4\r\nnoVN\r\n1\r\nC\r\n0\r\n\r\n"), b"noVNC");
        assert_eq!(dechunk(b"3;ext=1\r\nabc\r\n5\r\nde"), b"abcde");
    }

    #[test]
    fn a_screen_ticket_is_for_one_sandbox() {
        let a = crate::commands::screen_ticket("secret", "sbx-a");
        let b = crate::commands::screen_ticket("secret", "sbx-b");
        assert_ne!(a, b);
        assert_eq!(a.len(), 64);
        assert_ne!(a, crate::commands::screen_ticket("other", "sbx-a"));
    }

    #[test]
    fn a_token_matches_only_itself() {
        assert!(same("hunter2", "hunter2"));
        assert!(!same("hunter2", "hunter3"));
        assert!(!same("hunter2", "hunter20"));
        assert!(!same("", "hunter2"));
        assert!(same("", ""));
    }

    #[test]
    fn comparing_a_token_does_not_stop_at_the_first_wrong_byte() {
        let real = "0123456789abcdef";
        assert!(!same("0123456789abcde_", real));
        assert!(!same("_123456789abcdef", real));
    }

    #[test]
    fn a_server_refuses_to_open_a_browser_and_says_where_to_sign_in() {
        let open: crate::commands::OpenUrl = Arc::new(|_| {
            Err("this workspace runs on a server, so there is no browser here to open. Sign in \
                 from the app or the browser you are reading this in, and the workspace is \
                 handed the result"
                .to_string())
        });
        let said = open("https://example.com").unwrap_err();
        assert!(said.contains("no browser here"), "{said}");
        assert!(said.contains("Sign in from"), "{said}");
    }
}

#[cfg(test)]
mod relay_tests {
    use super::*;
    use futures_util::{SinkExt, StreamExt};

    #[tokio::test]
    async fn a_screen_socket_upgrades_and_carries_bytes_both_ways() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let viewer = listener.local_addr().unwrap().port();
        let upstream = tokio::spawn(async move {
            let (stream, _) = listener.accept().await.unwrap();
            let mut socket = tokio_tungstenite::accept_async(stream).await.unwrap();
            let message = socket.next().await.unwrap().unwrap();
            socket.send(message).await.unwrap();
        });
        let app = Router::new()
            .route(
                "/",
                any(move |request| async move {
                    relay_screen(viewer, "test", "6080", "websockify", request).await
                }),
            )
            .layer(axum::middleware::map_response(isolate_screen));
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let proxy = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        let (mut socket, _) =
            tokio_tungstenite::connect_async(format!("ws://{addr}/")).await.unwrap();
        socket
            .send(tokio_tungstenite::tungstenite::Message::Binary(vec![1, 2, 3].into()))
            .await
            .unwrap();
        let echoed = tokio::time::timeout(std::time::Duration::from_secs(2), socket.next())
            .await
            .unwrap()
            .unwrap()
            .unwrap();
        assert_eq!(echoed.into_data(), vec![1, 2, 3]);
        upstream.await.unwrap();
        proxy.abort();
    }

    #[tokio::test]
    async fn the_viewer_is_opaque_and_its_modules_remain_loadable() {
        let response = isolate_screen(page(200, "viewer")).await;
        assert_eq!(response.headers()[header::CONTENT_SECURITY_POLICY], "sandbox allow-scripts");
        assert_eq!(response.headers()[header::ACCESS_CONTROL_ALLOW_ORIGIN], "*");
        assert_eq!(response.headers()[header::REFERRER_POLICY], "no-referrer");
    }
}
