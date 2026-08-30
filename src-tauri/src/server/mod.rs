//! The server host: the same runtime, reached over HTTP and a socket.
//!
//! This is the second of two hosts. `app.rs` puts the runtime behind a window
//! on the operator's machine; this puts it on a box that stays awake when they
//! close their laptop. Everything between the two is identical, and that is the
//! design rather than a coincidence: `boot.rs` opens the workspace, `ipc.rs`
//! lists the commands, and neither of them knows which host it is in.
//!
//! ## It cannot tell whose box it is on
//!
//! An operator rents a machine, or guaca.ai hands them one. This module cannot
//! distinguish those and must not learn how: the difference is who pressed the
//! button at the provider, which is a fact about the bill. Keeping it out is
//! what makes bring-your-own-box free rather than a second product with a
//! second code path to fall out of step.
//!
//! ## Two shapes, because that is what a webview already spoke
//!
//! Tauri's IPC is "a name, some named arguments, and a value or a structured
//! error" plus one event channel the runtime pushes into. Those are a POST and
//! a WebSocket. Nothing above [`crate::ipc::dispatch`] learns which arrived, so
//! there is no second API to keep in step, and `ipc.contract.test.ts` fails the
//! build if the two transports ever answer to different sets of names.
//!
//! ## What guards it
//!
//! One bearer token, compared in constant time, on every route but health. The
//! workspace behind it holds inference keys, plugin refresh tokens and an
//! operator's transcripts, so there is no anonymous mode and no read-only mode:
//! a caller is the operator or it is nobody.
//!
//! The socket takes the token in the query string as well as the header,
//! because a browser cannot set headers on a WebSocket handshake. That is a
//! real cost (a query string reaches proxy logs, which a header does not), and
//! it is why the token is per-workspace and rotatable rather than derived from
//! anything longer-lived.

use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::Arc;

use axum::extract::ws::{Message, WebSocket, WebSocketUpgrade};
use axum::extract::{DefaultBodyLimit, Path, Query, RawQuery, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::{any, get, post};
use axum::{Json, Router};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use tokio::sync::broadcast;

use crate::commands::{AppState, Reach};
use crate::domain::attachment::MAX_FILE_BYTES;
use crate::domain::deployment::Deployment;
use crate::runtime::events::{EventSink, UiEvent};

/// How many events a client may fall behind before it starts losing them.
///
/// A streaming turn emits a token at a time, so this is seconds of backlog
/// rather than a queue depth anybody chose. Losing events is the correct
/// failure: the transcript is durable and a client that reconnects refetches,
/// exactly as the window does after a reload. What must not happen is the
/// runtime blocking on a slow socket, which would make one bad network
/// connection into a crew that stops thinking.
const BACKLOG: usize = 2048;

/// Where a daemon serves one workspace from.
pub struct Settings {
    /// The one directory everything durable lives under.
    pub root: PathBuf,
    pub bind: SocketAddr,
    /// The bearer token every call presents.
    pub token: String,
    /// The frontend bundle, served at `/` when there is one.
    ///
    /// Optional because a daemon is useful without it: a desktop app pointed at
    /// this box is a client, and so is a `curl`. Serving the bundle is what
    /// makes a browser one too.
    pub web: Option<PathBuf>,
    /// The origin a browser reaches this box at, when the operator knows it.
    ///
    /// Only a sign-in needs it, for the redirect. Absent, the daemon uses the
    /// origin of the last call that arrived, which is right whenever the
    /// operator is signing in from the page they are reading.
    pub origin: Option<String>,
}

/// Fans runtime events out to whoever is watching.
///
/// The counterpart of `TauriSink`, and it makes the same decision about a
/// failed send: no receivers means nobody is looking, which is ordinary rather
/// than an error. The transcript is already durable by the time this is called.
struct SocketSink {
    events: broadcast::Sender<UiEvent>,
}

impl EventSink for SocketSink {
    fn emit(&self, event: UiEvent) {
        // An `Err` here is "no client is attached", which is the normal state of
        // a workspace doing its work at four in the morning. It must never
        // propagate into an agent's turn.
        let _ = self.events.send(event);
    }
}

#[derive(Clone)]
struct Serving {
    state: Arc<AppState>,
    token: Arc<str>,
    events: broadcast::Sender<UiEvent>,
}

/// A workspace that is open and a socket that is listening, not yet serving.
///
/// Two steps rather than one because the address is only real once the
/// operating system has handed it back, and `0` is a legitimate thing to ask
/// for: a test wants a free port and a daemon under a supervisor may too.
/// Anything that needs to know where to connect has to be able to ask before
/// the serving future is spawned, and a future that never returns cannot be
/// asked anything.
pub struct Bound {
    /// Where it is actually listening, which is not always what was requested.
    pub addr: SocketAddr,
    /// How many agents came back up.
    pub agents: usize,
    listener: tokio::net::TcpListener,
    app: Router,
}

impl Bound {
    /// Serves until the process is asked to stop.
    pub async fn serve(self) -> Result<(), String> {
        axum::serve(self.listener, self.app)
            .with_graceful_shutdown(stopped())
            .await
            .map_err(|err| format!("the server stopped: {err}"))
    }
}

/// Opens the workspace and serves it until the process is asked to stop.
pub async fn serve(settings: Settings) -> Result<(), String> {
    bind(settings).await?.serve().await
}

/// Opens the workspace and binds the socket, without serving yet.
pub async fn bind(settings: Settings) -> Result<Bound, String> {
    let (events, _) = broadcast::channel(BACKLOG);
    let sink = Arc::new(SocketSink { events: events.clone() });

    let paths = crate::boot::Paths::under(&settings.root);
    let booted = crate::boot::open(&paths, tokio::runtime::Handle::current(), sink.clone()).await?;

    let state = Arc::new(AppState {
        runtime: booted.runtime,
        secret: Some(Arc::from(settings.token.as_str())),
        // A box has no corner of a screen. A window showing this workspace
        // feeds its own machine's strip, and that call never reaches here.
        menubar: Arc::new(|_presence| {}),
        deployment: Deployment::Server,
        open_url: {
            // Nobody is sitting at this machine. The only browser there is
            // belongs to whoever is reading the page, so the page is asked.
            // An `Err` from the send is no client attached, which is a sign-in
            // nobody could finish anyway: the flow times out on its own.
            let events = events.clone();
            Arc::new(move |url: &str| {
                if events.send(UiEvent::OpenUrl { url: url.to_string() }).is_err() {
                    return Err("nobody has this workspace open in a browser, so there is \
                                nowhere to show the sign-in page. Open the workspace and try \
                                again"
                        .to_string());
                }
                Ok(())
            })
        },
        reach: Reach::served(settings.origin.clone()),
        config_path: booted.config_path,
        // Never reached: `save_file` refuses before it looks, because a server
        // has no downloads folder anybody could open. Named honestly rather
        // than left as a path that means nothing.
        downloads: settings.root.join("downloads"),
        subscription: booted.subscription,
        account: booted.account,
        catalog: Arc::new(crate::llm::catalog::Catalog::new()),
        artifacts: booted.artifacts,
        artifact_port: booted.artifact_port,
    });

    let token: Arc<str> = settings.token.into();
    let serving = Serving { state, token: token.clone(), events };

    let mut app = Router::new()
        .route("/health", get(health))
        .route("/v1/call", post(call))
        .route("/v1/events", get(events_socket))
        // `:name` rather than `{name}`: axum 0.7 routes with matchit 0.7, where
        // braces are a literal path segment. Spelled the newer way this compiles,
        // registers a route nothing can match, and every preview draws nothing.
        // `a_stored_file_is_reachable_by_its_digest` is what catches it.
        .route("/v1/file/:digest/:name", get(file))
        // A document a browser hands over. The bytes are the body and the name
        // is on the query, which is the smallest possible shape: one file per
        // request, and `stage_files` on the desktop already answers one path
        // at a time inside its loop. The body limit is well above the store's
        // own, so a file a person plausibly drops is refused with the store's
        // sentence, which names the file and the limit, rather than with the
        // framework's bare 413. Past four times the limit nobody dropped it by
        // mistake, and the 413 is what a body that size deserves.
        .route("/v1/upload", post(upload))
        .layer(DefaultBodyLimit::max(4 * MAX_FILE_BYTES as usize))
        // Where a sign-in's browser comes back to. No token: the browser
        // arrives from the vendor with the vendor's answer, and what bounds
        // it is that only a flow that is waiting on that exact state reads it.
        .route(crate::oauth::CALLBACK_ROUTE, get(oauth_callback))
        // A page an agent wrote, on this origin instead of a loopback one.
        // The same document with the same policy; `frame-ancestors 'self'`
        // now names this origin, which is what frames it.
        .route("/v1/artifact/:id", get(artifact))
        // A computer's screen. The viewer on loopback keeps the sandbox
        // tokens and does the work; this hands bytes through, upgrade and
        // all, behind a ticket for that one sandbox.
        .route(
            &format!("{}/:ticket/:sandbox/:port/*rest", crate::commands::SCREEN_ROUTE),
            any(screen),
        )
        .with_state(serving);

    if let Some(web) = settings.web.as_ref() {
        // The same bundle the desktop app embeds. `fallback` rather than a
        // route, because the frontend is a single page and every path it
        // invents has to reach `index.html`.
        let index = web.join("index.html");
        app = app.fallback_service(
            tower_http::services::ServeDir::new(web)
                .fallback(tower_http::services::ServeFile::new(index)),
        );
        tracing::info!(web = %web.display(), "serving the app");
    }

    let listener = tokio::net::TcpListener::bind(settings.bind)
        .await
        .map_err(|err| format!("could not listen on {}: {err}", settings.bind))?;
    let addr = listener.local_addr().map_err(|err| err.to_string())?;
    tracing::info!(%addr, agents = booted.started, "guacad ready");
    if settings.web.is_some() {
        // The token is already in this log from the run that generated it.
        // Printing the whole invitation on every start is what makes the first
        // visit one click rather than a URL and a string pasted beside it, and
        // the fragment is the one part of a URL a browser never sends.
        tracing::info!(
            url = %invitation(addr, &token),
            "open this in a browser, or the same path on the address a tunnel gives this box"
        );
    }

    Ok(Bound { addr, agents: booted.started, listener, app })
}

/// The one link a browser needs, with the token where a browser keeps it.
///
/// A fragment rather than a query string, because the fragment is not sent to
/// this server, to a proxy in front of it, or to either one's logs. The socket
/// takes the token in a query string because a handshake cannot carry a
/// header; a first visit has no such excuse.
fn invitation(addr: SocketAddr, token: &str) -> String {
    // A daemon in a container listens on every interface, and `0.0.0.0` is
    // not an address a browser can open. `localhost` is right for the
    // operator who published the port to their own machine, and the log line
    // beside it already says to substitute a tunnel's address for a box.
    if addr.ip().is_unspecified() {
        format!("http://localhost:{}/#token={token}", addr.port())
    } else {
        format!("http://{addr}/#token={token}")
    }
}

/// Waits for the signal a container is stopped with.
///
/// Graceful rather than abrupt because a turn in flight is a model call
/// somebody paid for. It does not make one resumable: what this buys is the
/// requests already in the router finishing, and the log line saying so.
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

/// The commit this daemon was built from, told to the build rather than read
/// from a repository it does not ship with. Empty for a build made without
/// one, which `/health` says rather than hides: a box and a laptop that
/// disagree about this string are running different code, and that is the
/// first thing worth knowing about a bug that reproduces on one of them.
const BUILD: &str = match option_env!("GUACA_COMMIT") {
    Some(commit) => commit,
    None => "",
};

/// Liveness, and the one route with no token on it.
///
/// It says the process is up and nothing about the workspace. A provider's
/// health check has no credential, and a check that needed one would be a box
/// that reports itself unhealthy for the whole time a token is being rotated.
async fn health() -> Json<Value> {
    Json(json!({ "status": "ok", "service": "guacad", "build": BUILD }))
}

#[derive(Deserialize)]
struct Call {
    name: String,
    #[serde(default)]
    args: Value,
}

/// One command, arriving as JSON instead of over Tauri's IPC.
async fn call(
    State(serving): State<Serving>,
    headers: HeaderMap,
    Json(body): Json<Call>,
) -> Response {
    if let Err(refused) = authorized(&serving, &headers, None) {
        return *refused;
    }
    // Where this call came in, remembered for the redirect a sign-in will
    // name. Read off the headers a proxy rewrites rather than the socket,
    // because the socket sees the tunnel and the browser saw the tunnel's
    // public name.
    if let Some(origin) = reached_at(&headers) {
        serving.state.reach.note(origin);
    }
    match crate::ipc::dispatch(&serving.state, &body.name, body.args).await {
        Ok(value) => (StatusCode::OK, Json(json!({ "ok": value }))).into_response(),
        Err(refused) => {
            let status =
                StatusCode::from_u16(refused.status()).unwrap_or(StatusCode::INTERNAL_SERVER_ERROR);
            (status, Json(json!({ "err": refused.body() }))).into_response()
        }
    }
}

/// The scheme and host a request was addressed to, as the browser spelled it.
///
/// `X-Forwarded-*` first, because a tunnel or a reverse proxy terminates TLS
/// and rewrites the host, and the browser's own address is the forwarded one.
/// Then `Host`, which is what a browser sends straight to the box.
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

/// One document, arriving as bytes because a browser has no path to give.
///
/// The desktop's `stage_files` reads a path this side of IPC so a document
/// never enters the renderer; a browser is the renderer, and its bytes have to
/// cross once. They land in the same store by the same digest, and what comes
/// back is what a message carries.
async fn upload(
    State(serving): State<Serving>,
    headers: HeaderMap,
    Query(Upload { name }): Query<Upload>,
    body: axum::body::Bytes,
) -> Response {
    if let Err(refused) = authorized(&serving, &headers, None) {
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

/// A sign-in's browser, back from the vendor.
///
/// Handed to the flow waiting on its state, which decides the page: only the
/// flow knows whether the state matched and the issuer was the one it sent
/// the operator to. Nobody waiting is a stale tab or a guess, and is told so.
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

/// A page an agent wrote, by the digest it was framed under.
///
/// The desktop serves this from a loopback origin of its own; here it is a
/// route, and everything `artifact.rs` argues still holds: the policy rides
/// on every answer, refusals included, and the frame's `sandbox` keeps the
/// page's origin opaque. Token in the query, because a frame cannot carry a
/// header, which is the trade the file route already makes.
async fn artifact(
    State(serving): State<Serving>,
    headers: HeaderMap,
    Query(ticket): Query<Ticket>,
    Path(id): Path<String>,
) -> Response {
    if let Err(refused) = authorized(&serving, &headers, ticket.token.as_deref()) {
        return *refused;
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

/// One request for a computer's screen, handed through to the viewer.
///
/// The viewer is a byte-level relay on loopback, and this is the same shape one
/// hop out: the head is rewritten and everything after it is copied. An
/// upgrade is taken over from hyper once the handshake has been answered and
/// spliced with the viewer's socket; an ordinary request is read to the end,
/// because the viewer closes after one response, and answered whole.
///
/// The ticket is a path segment rather than a query, because noVNC resolves
/// its own scripts and its socket relative to the page it was served from, and
/// a query string does not survive that. `referer` and `cookie` are dropped on
/// the way in, so the ticket never reaches the machine on the far side.
async fn screen(
    State(serving): State<Serving>,
    Path((ticket, sandbox, port, rest)): Path<(String, String, String, String)>,
    mut request: axum::extract::Request,
) -> Response {
    let Some(secret) = serving.state.secret.as_deref() else {
        return page(404, "No screen is reachable here.");
    };
    if !same(&ticket, &crate::commands::screen_ticket(secret, &sandbox)) {
        return page(404, "That is not a screen this workspace is showing.");
    }
    let viewer = serving.state.runtime.viewer_port();
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
        // Rewritten, decided here, or something the far side must not see.
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

    // The viewer's answer, head first. It is a small server that answers with
    // one head and then bytes, and the head is what decides what happens next.
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
        // Hand the socket over once hyper has finished the handshake, and then
        // it is bytes both ways until one side hangs up.
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
        return response;
    }

    // One response, then the viewer closes: what is left is the body.
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

/// Status and headers out of a response head, or nothing for a head that is
/// not one.
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

/// Puts the viewer's headers on the answer, minus the ones that describe a
/// connection this daemon owns rather than the viewer.
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

/// A chunked body, joined. What the far side chunked is answered whole here
/// with a length, so the framing is decided once, by hyper.
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

/// The page a browser is left on, spelled as `oauth::reply` spells it.
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

/// The event channel, which is the socket half of what Tauri gave for free.
async fn events_socket(
    State(serving): State<Serving>,
    headers: HeaderMap,
    Query(ticket): Query<Ticket>,
    upgrade: WebSocketUpgrade,
) -> Response {
    if let Err(refused) = authorized(&serving, &headers, ticket.token.as_deref()) {
        return *refused;
    }
    // Subscribed before the upgrade completes. Between accepting and
    // subscribing there is a window in which events are dropped, and a client
    // that reconnects mid-cascade would silently miss the run settling.
    let feed = serving.events.subscribe();
    upgrade.on_upgrade(move |socket| pump(socket, feed))
}

/// Forwards events to one client until it goes away.
async fn pump(mut socket: WebSocket, mut feed: broadcast::Receiver<UiEvent>) {
    loop {
        tokio::select! {
            event = feed.recv() => match event {
                Ok(event) => {
                    let Ok(text) = serde_json::to_string(&event) else { continue };
                    if socket.send(Message::Text(text)).await.is_err() {
                        return;
                    }
                }
                // Behind by more than the backlog. Told rather than hidden: the
                // client refetches what it draws, and a gap it does not know
                // about is a transcript that is quietly missing a message.
                Err(broadcast::error::RecvError::Lagged(missed)) => {
                    tracing::warn!(missed, "a client fell behind the event stream");
                    let notice = json!({ "type": "streamLagged", "missed": missed });
                    if socket.send(Message::Text(notice.to_string())).await.is_err() {
                        return;
                    }
                }
                Err(broadcast::error::RecvError::Closed) => return,
            },
            // Read the other direction only to notice a close. Nothing a client
            // sends here is acted on: every intent goes through `/v1/call`,
            // where it is one named command with typed arguments rather than
            // whatever arrived on a socket.
            incoming = socket.recv() => match incoming {
                Some(Ok(_)) => continue,
                _ => return,
            },
        }
    }
}

/// One stored file, by the digest of its own contents.
///
/// The desktop answers this on a `guacfile:` scheme registered in `app.rs`;
/// here it is a route, and the reasoning either side of it is unchanged. The
/// bytes never cross the command surface: a transcript does, in bulk, which is
/// the whole reason a message carries a digest rather than a document.
///
/// The token is in the query string because an `<img>` and a `<frame>` cannot
/// carry a header, which is the same trade the event socket makes and for the
/// same reason. What bounds it is that nothing is addressable here but a digest
/// this workspace already stored.
async fn file(
    State(serving): State<Serving>,
    headers: HeaderMap,
    Query(ticket): Query<Ticket>,
    Path((digest, name)): Path<(String, String)>,
) -> Response {
    if let Err(refused) = authorized(&serving, &headers, ticket.token.as_deref()) {
        return *refused;
    }
    // Range, because a browser asks for one when it draws a PDF or seeks in a
    // video, and an answer that ignores it draws nothing.
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
            if let Some(range) = served.content_range.and_then(|range| range.parse().ok()) {
                headers.insert(axum::http::header::CONTENT_RANGE, range);
            }
            // Content-addressed, so the bytes at one address never change and
            // a browser may keep them for as long as it likes.
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

/// Whether a request carries the workspace's token.
///
/// Constant time, because a comparison that returns early on the first wrong
/// byte is one an attacker can walk a character at a time. Cheap enough that
/// there is no argument for the fast version.
fn authorized(
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

/// Byte equality that does not return early.
fn same(a: &str, b: &str) -> bool {
    let (a, b) = (a.as_bytes(), b.as_bytes());
    // Length is not a secret and cannot be hidden by this anyway: a token of a
    // different length is refused, and how long it took to say so tells an
    // attacker only what the length was.
    if a.len() != b.len() {
        return false;
    }
    a.iter().zip(b).fold(0u8, |differs, (x, y)| differs | (x ^ y)) == 0
}

/// What a client is told the workspace is.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Greeting {
    pub deployment: Deployment,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_invitation_keeps_the_token_out_of_every_log_but_this_one() {
        let addr: SocketAddr = "127.0.0.1:8787".parse().unwrap();
        let url = invitation(addr, "abc123");
        assert_eq!(url, "http://127.0.0.1:8787/#token=abc123");
        // A fragment, never a query string: the query string is sent to the
        // server and to anything in front of it, and the fragment is not.
        assert!(!url.contains('?'), "{url}");

        // A container binds every interface, and nobody can open 0.0.0.0.
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

        // Behind a tunnel the socket sees the tunnel and the browser saw the
        // tunnel's public name, which is what a redirect has to be sent to.
        headers.insert("x-forwarded-host", "guaca.example.com".parse().unwrap());
        headers.insert("x-forwarded-proto", "https".parse().unwrap());
        assert_eq!(reached_at(&headers).as_deref(), Some("https://guaca.example.com"));

        // A proto nobody would send a browser back over is not one.
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
        // A chunk extension is ignored, and a truncated tail keeps what arrived.
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
        // The property, stated as the thing it prevents: two wrong tokens of
        // the same length take the same path regardless of how much of the
        // prefix is right. Timing is not assertable in a unit test, so what is
        // checked is that neither returns early with a different answer.
        let real = "0123456789abcdef";
        assert!(!same("0123456789abcde_", real));
        assert!(!same("_123456789abcdef", real));
    }

    #[test]
    fn a_server_refuses_to_open_a_browser_and_says_where_to_sign_in() {
        // The failure this prevents is an operator waiting at a consent screen
        // that was never drawn, because something reported success for opening
        // a browser on a machine with no screen.
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
