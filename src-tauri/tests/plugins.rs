//! Plugins, end to end, against a scripted MCP server that signs Guaca in.
//!
//! A fourth suite, and it is here for the reason `subscription.rs` is here:
//! this is a protocol nothing else in the build speaks. The cascade tests drive
//! the runtime against an OpenAI-compatible endpoint, and every one of them
//! would pass with the whole plugin path never dispatched, the sign-in never
//! stored and the grant never spent.
//!
//! Everything below goes through the real `oauth`, `mcp`, `plugins` and store
//! code. What is scripted is the far side: a server that publishes RFC 9728
//! protected-resource metadata, RFC 8414 authorization-server metadata, an
//! RFC 7591 registration endpoint, an authorization endpoint, a token endpoint
//! and an MCP endpoint. That is the whole surface an operator's Neon account is
//! on the other side of.
//!
//! The browser is a callback. `oauth::authorize` takes the URL to visit rather
//! than opening one itself, so a test plays the part of the person: it reads
//! the redirect URI and the state out of the URL and calls back to the loopback
//! listener, which is exactly what a browser does and nothing more.

mod harness;

use std::collections::HashMap;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

use axum::extract::{Query, State};
use axum::response::IntoResponse;
use axum::routing::{get, post};
use axum::Router;
use parking_lot::Mutex;

use guac_lib::db::store::PluginReach;
use guac_lib::db::Store;
use guac_lib::domain::agent::CleanDraft;
use guac_lib::domain::group::CleanGroup;
use guac_lib::domain::ids::{AgentId, GroupId};
use guac_lib::domain::plugin::{PluginAccess, PluginKind};
use guac_lib::llm::tools::{self, ToolInvocation};
use guac_lib::mcp::PROTOCOL_VERSION;
use guac_lib::plugins;

/// A tool narrowed to nobody: switched off for the whole crew.
const NOBODY: PluginAccess = PluginAccess::Chosen { agents: Vec::new() };

/// How the scripted server behaves. Every field is something a real one does.
#[derive(Debug, Clone)]
struct Rules {
    /// False is a server that authorizes everybody and asks for nothing. None
    /// of the five on the list does today, and the row must not claim a
    /// sign-in if one starts.
    needs_token: bool,
    /// False is a server that publishes no RFC 7591 registration endpoint.
    /// Guaca cannot sign in to one of those at all, and the rule that keeps a
    /// vendor on the list is that it does.
    registers: bool,
    /// Seconds until the issued token expires. `None` is a server that does not
    /// say, which means the token is used until it is refused.
    expires_in: Option<i64>,
    /// True makes the first token stale on arrival, so a call has to refresh
    /// before it can be made.
    issue_expired: bool,
    /// A bearer the server accepts without ever having issued it, which is what
    /// an account-backed plugin presents: the token is the machine's Guaca
    /// account, minted somewhere this server's sign-in never ran.
    account_token: Option<String>,
    /// Which shape of the protocol this server implements.
    era: Era,
    /// Every revision it will accept, for a modern one. Empty is "whatever it
    /// is at", and a list that shares nothing with this build is the failure
    /// no amount of reconnecting fixes.
    versions: Vec<String>,
    /// A tool that asks for one of its arguments in an HTTP header.
    ///
    /// Optional for a server and mandatory for a client, so the only way to
    /// prove Guaca does it is a server that demands it and refuses a call that
    /// arrives without it — which is exactly what a real one does.
    mirrors: bool,
}

/// Which shape of the protocol the scripted server speaks.
///
/// Both, because both are in the field and a client that speaks one is a
/// client that cannot reach half of them. Every vendor on the list today is
/// `Legacy`; `Modern` is revision 2026-07-28, which deleted the handshake.
#[derive(Debug, Clone, Copy, PartialEq)]
enum Era {
    /// `initialize`, a session id, and no `server/discover`.
    Legacy,
    /// Per-request `_meta`, mirrored headers, and no session at all.
    Modern,
}

impl Default for Rules {
    fn default() -> Self {
        Rules {
            needs_token: true,
            registers: true,
            expires_in: Some(3600),
            issue_expired: false,
            account_token: None,
            era: Era::Legacy,
            versions: Vec::new(),
            mirrors: false,
        }
    }
}

#[derive(Clone)]
struct Server {
    rules: Rules,
    base: Arc<Mutex<String>>,
    /// Every access token this server has issued, oldest first.
    issued: Arc<Mutex<Vec<String>>>,
    /// Tokens the server has stopped accepting. Nothing local can tell that a
    /// grant was revoked at the vendor, so a test has to be able to do it from
    /// the far side, between one call and the next.
    revoked: Arc<Mutex<Vec<String>>>,
    /// Bearer tokens seen on an MCP request, so a test can assert the grant was
    /// spent rather than merely stored.
    seen: Arc<Mutex<Vec<Option<String>>>>,
    /// What `tools/call` was asked for.
    called: Arc<Mutex<Vec<(String, serde_json::Value)>>>,
    /// The `mcp-*` headers on every request, so a test can assert what an
    /// intermediary would have been able to route on without reading a body.
    noted: Arc<Mutex<Vec<HashMap<String, String>>>>,
    registrations: Arc<AtomicUsize>,
    refreshes: Arc<AtomicUsize>,
}

impl Server {
    fn base(&self) -> String {
        self.base.lock().clone()
    }

    /// Whether this token is one the server still accepts.
    /// The revision this server will serve, for a modern one.
    fn version(&self) -> String {
        self.rules.versions.first().cloned().unwrap_or_else(|| PROTOCOL_VERSION.to_string())
    }

    fn accepts(&self, token: &str) -> bool {
        if self.revoked.lock().iter().any(|dead| dead == token) {
            return false;
        }
        if self.rules.account_token.as_deref() == Some(token) {
            return true;
        }
        self.issued.lock().iter().any(|held| held == token)
    }
}

async fn serve(rules: Rules) -> Server {
    let server = Server {
        rules,
        base: Arc::new(Mutex::new(String::new())),
        issued: Arc::new(Mutex::new(Vec::new())),
        revoked: Arc::new(Mutex::new(Vec::new())),
        seen: Arc::new(Mutex::new(Vec::new())),
        called: Arc::new(Mutex::new(Vec::new())),
        noted: Arc::new(Mutex::new(Vec::new())),
        registrations: Arc::new(AtomicUsize::new(0)),
        refreshes: Arc::new(AtomicUsize::new(0)),
    };

    let app = Router::new()
        .route("/.well-known/oauth-protected-resource", get(protected_resource))
        .route("/.well-known/oauth-authorization-server", get(authorization_server))
        .route("/register", post(register))
        .route("/authorize", get(authorize))
        .route("/token", post(token))
        .route("/mcp", post(rpc))
        .with_state(server.clone());

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    *server.base.lock() = format!("http://{addr}");
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    server
}

async fn protected_resource(State(server): State<Server>) -> impl IntoResponse {
    let base = server.base();
    axum::Json(serde_json::json!({
        "resource": format!("{base}/mcp"),
        "authorization_servers": [base],
        "bearer_methods_supported": ["header"],
    }))
}

async fn authorization_server(State(server): State<Server>) -> impl IntoResponse {
    let base = server.base();
    let mut metadata = serde_json::json!({
        "issuer": base,
        "authorization_endpoint": format!("{base}/authorize"),
        "token_endpoint": format!("{base}/token"),
        "response_types_supported": ["code"],
        "grant_types_supported": ["authorization_code", "refresh_token"],
        "code_challenge_methods_supported": ["S256"],
        // Neon publishes exactly this. The wildcard has to be filtered out on
        // the way into the authorization URL, and this is where that is proved.
        "scopes_supported": ["read", "write", "*"],
    });
    if server.rules.registers {
        metadata["registration_endpoint"] = serde_json::json!(format!("{base}/register"));
    }
    axum::Json(metadata)
}

async fn register(
    State(server): State<Server>,
    axum::Json(body): axum::Json<serde_json::Value>,
) -> impl IntoResponse {
    server.registrations.fetch_add(1, Ordering::SeqCst);
    // The redirect has to be a loopback address that was bound before this call
    // was made. A registration carrying anything else is a client that cannot
    // catch its own answer.
    let redirect = body["redirect_uris"][0].as_str().unwrap_or_default().to_string();
    assert!(redirect.starts_with("http://127.0.0.1:"), "registered {redirect}");
    axum::Json(serde_json::json!({
        "client_id": "scripted-client",
        // Issued alongside `none`, exactly as Neon's does. Sending it back at
        // the token endpoint is what gets a public client rejected.
        "client_secret": "must-not-be-sent",
        "token_endpoint_auth_method": "none",
        "redirect_uris": [redirect],
    }))
}

/// Stands in for the page the operator would see, and answers with a code.
async fn authorize(Query(params): Query<HashMap<String, String>>) -> impl IntoResponse {
    assert_eq!(params.get("code_challenge_method").map(String::as_str), Some("S256"));
    assert!(params.contains_key("code_challenge"));
    assert_eq!(params.get("scope").map(String::as_str), Some("read write"));
    axum::Json(serde_json::json!({ "seen": params }))
}

async fn token(
    State(server): State<Server>,
    axum::extract::Form(form): axum::extract::Form<HashMap<String, String>>,
) -> impl IntoResponse {
    assert!(
        !form.contains_key("client_secret"),
        "a client registered as public must not send the secret it was handed"
    );

    let grant = form.get("grant_type").map(String::as_str).unwrap_or_default();
    if grant == "refresh_token" {
        server.refreshes.fetch_add(1, Ordering::SeqCst);
    } else {
        assert!(form.contains_key("code_verifier"), "the exchange has to prove the PKCE challenge");
        // RFC 8707. A server that issues audience-bound tokens needs it, and
        // dropping it is invisible until one of them refuses.
        assert!(form.contains_key("resource"), "the exchange has to name the resource");
    }

    let serial = server.issued.lock().len();
    let access = format!("access-{serial}");
    server.issued.lock().push(access.clone());

    let mut issued = serde_json::json!({
        "access_token": access,
        "refresh_token": "refresh-token",
        "token_type": "Bearer",
    });
    // A first token that is already expired, so the next call has to renew it
    // before it is spent.
    let seconds = if server.rules.issue_expired && grant != "refresh_token" {
        Some(0)
    } else {
        server.rules.expires_in
    };
    if let Some(seconds) = seconds {
        issued["expires_in"] = serde_json::json!(seconds);
    }
    axum::Json(issued)
}

async fn rpc(
    State(server): State<Server>,
    headers: axum::http::HeaderMap,
    axum::Json(body): axum::Json<serde_json::Value>,
) -> axum::response::Response {
    let bearer = headers
        .get("authorization")
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.strip_prefix("Bearer "))
        .map(str::to_string);
    server.seen.lock().push(bearer.clone());

    if server.rules.needs_token && !bearer.as_deref().is_some_and(|t| server.accepts(t)) {
        return (
            axum::http::StatusCode::UNAUTHORIZED,
            [(
                "www-authenticate",
                format!(
                    r#"Bearer error="invalid_token", resource_metadata="{}/.well-known/oauth-protected-resource""#,
                    server.base()
                ),
            )],
            "",
        )
            .into_response();
    }

    let id = body.get("id").cloned().unwrap_or(serde_json::Value::Null);
    let method = body.get("method").and_then(|m| m.as_str()).unwrap_or_default().to_string();

    // What an intermediary could have routed on. Recorded whatever the era, so
    // a test can also assert that a legacy request carries none of it.
    let noted: HashMap<String, String> = headers
        .iter()
        .filter(|(name, _)| name.as_str().starts_with("mcp-"))
        .filter_map(|(name, value)| {
            value.to_str().ok().map(|value| (name.as_str().to_string(), value.to_string()))
        })
        .collect();
    server.noted.lock().push(noted.clone());

    let refusal = |status: axum::http::StatusCode, code: i64, message: String, data| {
        (
            status,
            axum::Json(serde_json::json!({
                "jsonrpc": "2.0",
                "id": id,
                "error": { "code": code, "message": message, "data": data },
            })),
        )
            .into_response()
    };

    if server.rules.era == Era::Modern {
        // A modern-only server has never heard of the handshake, and answers
        // it the way it answers any unknown method.
        if method == "initialize" || method == "notifications/initialized" {
            return refusal(
                axum::http::StatusCode::NOT_FOUND,
                -32601,
                format!("no method {method}"),
                serde_json::Value::Null,
            );
        }

        let want = server.version();
        let declared = body["params"]["_meta"]["io.modelcontextprotocol/protocolVersion"]
            .as_str()
            .unwrap_or_default()
            .to_string();
        let supported: Vec<String> = if server.rules.versions.is_empty() {
            vec![want.clone()]
        } else {
            server.rules.versions.clone()
        };
        if !supported.contains(&declared) {
            return refusal(
                axum::http::StatusCode::BAD_REQUEST,
                -32022,
                "Unsupported protocol version".into(),
                serde_json::json!({ "supported": supported, "requested": declared }),
            );
        }
        // The header has to agree with the body, which is the whole reason the
        // header exists: an intermediary routes on it and the server executes
        // on the body, and a client that lets them differ is the vulnerability.
        let mismatch = noted.get("mcp-protocol-version") != Some(&declared)
            || noted.get("mcp-method") != Some(&method)
            || (method == "tools/call"
                && noted.get("mcp-name").map(String::as_str) != body["params"]["name"].as_str());
        if mismatch {
            return refusal(
                axum::http::StatusCode::BAD_REQUEST,
                -32020,
                format!("headers do not match the body: {noted:?}"),
                serde_json::Value::Null,
            );
        }
        if method == "server/discover" {
            return event(
                &id,
                serde_json::json!({
                    "supportedVersions": supported,
                    "capabilities": { "tools": {} },
                    "_meta": {
                        "io.modelcontextprotocol/serverInfo": { "name": "Scripted MCP Server" },
                    },
                }),
            );
        }
    } else if method == "server/discover" {
        // A legacy server answering an unknown method, which is how this client
        // finds out it is one. Two hundred with a JSON-RPC error rather than a
        // status code, because that is what a real one does and it is the case
        // a status-only fallback rule would get wrong.
        return axum::Json(serde_json::json!({
            "jsonrpc": "2.0",
            "id": id,
            "error": { "code": -32601, "message": "no method server/discover" },
        }))
        .into_response();
    }

    let result = match method.as_str() {
        "initialize" => serde_json::json!({
            "protocolVersion": "2025-06-18",
            "capabilities": { "tools": {} },
            "serverInfo": { "name": "Scripted MCP Server" },
        }),
        "notifications/initialized" => return axum::http::StatusCode::ACCEPTED.into_response(),
        "tools/list" => {
            let mut tools = vec![
                serde_json::json!({
                    "name": "run_sql",
                    "description": "Run a query.",
                    "inputSchema": { "type": "object", "properties": { "sql": { "type": "string" } } },
                }),
                // No schema at all, which is legal and means no arguments. It
                // has to reach the model as an empty object rather than as null.
                serde_json::json!({ "name": "list_projects", "description": "" }),
            ];
            if server.rules.mirrors {
                tools.push(serde_json::json!({
                    "name": "deploy",
                    "description": "Deploy to a region.",
                    "inputSchema": {
                        "type": "object",
                        "properties": {
                            "region": { "type": "string", "x-mcp-header": "Region" },
                            "what": { "type": "string" },
                        },
                    },
                }));
                // And one no client may offer at all: its annotation sits under
                // `items`, where a call has no single value to mirror.
                tools.push(serde_json::json!({
                    "name": "broken",
                    "description": "Annotated somewhere nothing can read.",
                    "inputSchema": {
                        "type": "object",
                        "properties": {
                            "rows": {
                                "type": "array",
                                "items": {
                                    "type": "object",
                                    "properties": {
                                        "id": { "type": "string", "x-mcp-header": "Id" },
                                    },
                                },
                            },
                        },
                    },
                }));
            }
            serde_json::json!({ "tools": tools })
        }
        "tools/call" => {
            let name = body["params"]["name"].as_str().unwrap_or_default().to_string();
            let arguments = body["params"]["arguments"].clone();
            if server.rules.mirrors && name == "deploy" {
                // The server validates the mirrored header against the body,
                // which is what the spec requires of it and what makes a client
                // that skipped the header fail here rather than silently.
                if noted.get("mcp-param-region").map(String::as_str) != arguments["region"].as_str()
                {
                    return refusal(
                        axum::http::StatusCode::BAD_REQUEST,
                        -32020,
                        "Mcp-Param-Region does not match the body".into(),
                        serde_json::Value::Null,
                    );
                }
            }
            server.called.lock().push((name.clone(), arguments));
            serde_json::json!({ "content": [{ "type": "text", "text": format!("{name} ran") }] })
        }
        other => {
            return axum::Json(serde_json::json!({
                "jsonrpc": "2.0",
                "id": id,
                "error": { "code": -32601, "message": format!("no method {other}") },
            }))
            .into_response()
        }
    };

    event(&id, result)
}

/// A reply, as an event stream rather than as JSON.
///
/// Because a real server on the list does, and parsing only JSON made a working
/// server look broken.
fn event(id: &serde_json::Value, result: serde_json::Value) -> axum::response::Response {
    (
        [("content-type", "text/event-stream")],
        format!(
            "event: message\ndata: {}\n\n",
            serde_json::json!({ "jsonrpc": "2.0", "id": id, "result": result })
        ),
    )
        .into_response()
}

/// Plays the browser: visits the authorization page, then calls the app back.
///
/// Both halves matter. Visiting the page is what puts the PKCE challenge and
/// the scope in front of the scripted server's assertions; calling back is what
/// exercises the loopback listener, which is the part of this flow that has no
/// other test.
fn browser(outcome: Outcome) -> impl FnOnce(&str) -> Result<(), String> {
    move |url: &str| {
        let url = url.to_string();
        std::thread::spawn(move || {
            let query = url.split_once('?').map(|(_, q)| q).unwrap_or_default();
            let mut fields: HashMap<String, String> = HashMap::new();
            for pair in query.split('&') {
                if let Some((key, value)) = pair.split_once('=') {
                    fields.insert(key.to_string(), unescape(value));
                }
            }
            fetch(&url);

            let redirect = fields.get("redirect_uri").cloned().unwrap_or_default();
            let state = fields.get("state").cloned().unwrap_or_default();
            let back = match outcome {
                Outcome::Allowed => format!("{redirect}?code=the-code&state={state}"),
                Outcome::WrongState => format!("{redirect}?code=the-code&state=somebody-elses"),
                Outcome::Refused => format!(
                    "{redirect}?error=access_denied&error_description=Not+today&state={state}"
                ),
            };
            // A browser asks for the favicon while it is showing the page. The
            // listener has to keep waiting through that rather than taking it
            // as the answer, so it is sent here on purpose.
            fetch(&redirect.replace("/callback", "/favicon.ico"));
            fetch(&back);
        });
        Ok(())
    }
}

/// One HTTP GET, written by hand.
///
/// Not `reqwest`: its blocking client is a feature this build does not carry,
/// and a browser stand-in that needs a runtime inside a spawned thread is more
/// machinery than a request line.
fn fetch(url: &str) {
    use std::io::{Read, Write};

    let Some(rest) = url.strip_prefix("http://") else { return };
    let (host, path) = match rest.find('/') {
        Some(at) => (&rest[..at], &rest[at..]),
        None => (rest, "/"),
    };
    let Ok(mut socket) = std::net::TcpStream::connect(host) else { return };
    let _ = socket.write_all(
        format!("GET {path} HTTP/1.1\r\nHost: {host}\r\nConnection: close\r\n\r\n").as_bytes(),
    );
    let _ = socket.read_to_string(&mut String::new());
}

#[derive(Debug, Clone, Copy)]
enum Outcome {
    Allowed,
    WrongState,
    Refused,
}

fn unescape(raw: &str) -> String {
    let bytes = raw.as_bytes();
    let mut out = Vec::new();
    let mut at = 0;
    while at < bytes.len() {
        if bytes[at] == b'%' && at + 2 < bytes.len() {
            out.push(u8::from_str_radix(&raw[at + 1..at + 3], 16).unwrap_or(b'%'));
            at += 3;
        } else {
            out.push(bytes[at]);
            at += 1;
        }
    }
    String::from_utf8_lossy(&out).to_string()
}

/// A store with one group in it, in a directory that dies with the test.
/// A crew of one, which is what every call below is made on behalf of. A
/// plugin call is an agent's, not a group's: the group holds the sign-in and
/// the agent has to be one the operator allowed to spend it.
fn workspace() -> (tempfile::TempDir, Store, GroupId, AgentId) {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open(&dir.path().join("guac.db")).unwrap();
    let group = store
        .create_group(&CleanGroup { name: "Crew".to_string(), ..Default::default() })
        .unwrap()
        .id;
    let agent = crew(&store, group, "Manager");
    (dir, store, group, agent)
}

fn crew(store: &Store, group: GroupId, name: &str) -> AgentId {
    store
        .create_agent(&CleanDraft {
            group_id: Some(group),
            name: name.to_string(),
            avatar: "avocado".to_string(),
            color: "#7fb069".to_string(),
            model: "anthropic/claude-sonnet-4.5".to_string(),
            system_prompt: "be useful".to_string(),
            skills: Vec::new(),
        })
        .unwrap()
        .id
}

#[tokio::test]
async fn a_server_that_asks_for_nothing_connects_without_sending_anybody_to_a_browser() {
    // An operator sent to authorize a server that authorizes everybody is a
    // consent prompt for nothing, and the row would claim a sign-in that never
    // happened. No vendor on the list is public today; this is the behavior if
    // one becomes it, and the live test is what would say so.
    let server = serve(Rules { needs_token: false, ..Default::default() }).await;
    let (_dir, store, group, _agent) = workspace();

    let plugin = plugins::connect(
        &store,
        group,
        &PluginKind::Neon,
        &format!("{}/mcp", server.base()),
        plugins::Credential::Discover,
        |_| panic!("a public server must not open a browser"),
    )
    .await
    .expect("a public server connects");

    assert!(!plugin.signed_in, "nothing was authorized, so nothing may claim to have been");
    let named: Vec<&str> = plugin.tools.iter().map(|tool| tool.name.as_str()).collect();
    assert_eq!(named, vec!["run_sql", "list_projects"]);
    assert!(
        plugin.tools.iter().all(|tool| tool.access == PluginAccess::Everyone),
        "a plugin arrives with nothing narrowed"
    );
    assert_eq!(server.registrations.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn a_protected_server_signs_in_and_the_grant_stays_in_the_store() {
    let server = serve(Rules::default()).await;
    let (_dir, store, group, _agent) = workspace();

    let plugin = plugins::connect(
        &store,
        group,
        &PluginKind::Neon,
        &format!("{}/mcp", server.base()),
        plugins::Credential::Discover,
        browser(Outcome::Allowed),
    )
    .await
    .expect("the whole dance completes");

    assert!(plugin.signed_in);
    assert_eq!(server.registrations.load(Ordering::SeqCst), 1, "registered once, not per leg");

    // The grant is readable by the code that spends it and by nothing else. The
    // type that crosses IPC has no field for one, which is the real guarantee;
    // this checks the row it would have come from.
    let held = store.group_plugins(group).unwrap();
    let json = serde_json::to_string(&held).unwrap();
    assert!(!json.contains("access-0"), "a grant must not be serializable to the webview: {json}");
    assert!(!json.contains("refresh-token"));
    assert!(!json.contains("must-not-be-sent"));
}

#[tokio::test]
async fn a_redirect_that_does_not_match_is_refused() {
    // Nothing else can arrive on that port with the wrong state, so this is an
    // attack rather than a mistake, and it must not produce a usable plugin.
    let server = serve(Rules::default()).await;
    let (_dir, store, group, _agent) = workspace();

    let failed = plugins::connect(
        &store,
        group,
        &PluginKind::Neon,
        &format!("{}/mcp", server.base()),
        plugins::Credential::Discover,
        browser(Outcome::WrongState),
    )
    .await
    .expect_err("a mismatched state cannot connect");

    assert!(failed.to_string().contains("did not match"), "{failed}");
    assert!(store.group_plugins(group).unwrap().is_empty());
}

#[tokio::test]
async fn a_refusal_in_the_browser_is_reported_as_one() {
    let server = serve(Rules::default()).await;
    let (_dir, store, group, _agent) = workspace();

    let failed = plugins::connect(
        &store,
        group,
        &PluginKind::Neon,
        &format!("{}/mcp", server.base()),
        plugins::Credential::Discover,
        browser(Outcome::Refused),
    )
    .await
    .expect_err("a refusal is not a connection");

    let said = failed.to_string();
    assert!(said.contains("access_denied"), "{said}");
    assert!(said.contains("Not today"), "the reason the server gave has to survive: {said}");
    assert!(store.group_plugins(group).unwrap().is_empty());
}

#[tokio::test]
async fn a_server_with_no_registration_says_so_rather_than_failing_obscurely() {
    // Publishing a registration endpoint is the rule that decides who can be a
    // plugin at all. This is the error an operator gets on the day a vendor
    // stops, and it has to say what is actually wrong rather than fail at the
    // next leg with a client id nobody issued.
    let server = serve(Rules { registers: false, ..Default::default() }).await;
    let (_dir, store, group, _agent) = workspace();

    let failed = plugins::connect(
        &store,
        group,
        &PluginKind::Neon,
        &format!("{}/mcp", server.base()),
        plugins::Credential::Discover,
        browser(Outcome::Allowed),
    )
    .await
    .expect_err("there is no way to register");

    assert!(failed.to_string().contains("register itself"), "{failed}");
}

#[tokio::test]
async fn a_tool_call_carries_the_grant_and_the_answer_comes_back() {
    let server = serve(Rules::default()).await;
    let endpoint = format!("{}/mcp", server.base());
    let (_dir, store, group, agent) = workspace();

    plugins::connect(
        &store,
        group,
        &PluginKind::Neon,
        &endpoint,
        plugins::Credential::Discover,
        browser(Outcome::Allowed),
    )
    .await
    .unwrap();

    let answer = plugins::call(
        &store,
        plugins::Target {
            group,
            agent,
            kind: &PluginKind::Neon,
            endpoint: &endpoint,
            account: None,
        },
        "run_sql",
        &serde_json::json!({ "sql": "select 1" }),
    )
    .await
    .expect("the call goes through");

    assert_eq!(answer, "run_sql ran");
    assert_eq!(
        server.called.lock().first().map(|(name, _)| name.clone()),
        Some("run_sql".to_string()),
        "the tool is called by the server's own name, without the plugin prefix"
    );
    assert!(
        server.seen.lock().iter().flatten().any(|token| token == "access-0"),
        "the grant has to reach the server, or it is only being stored"
    );
}

/// A plugin whose credential is the operator's own Guaca account.
///
/// Google is the one of these today. It is not a vendor's server the crew signs
/// in to: it is `guaca.bot`, which already holds the Google grant and already
/// refreshes it, so the sign-in is the account's and the only decision left is
/// the group's. Everything else about a plugin has to keep working unchanged,
/// and that is what these check: the tool list is read the same way, the reach
/// rule still decides who may call it, and the token still never appears
/// anywhere an agent could read it.
mod account_backed {
    use super::*;

    const ACCOUNT: &str = "account-access-token";

    async fn account_server() -> Server {
        serve(Rules { account_token: Some(ACCOUNT.to_string()), ..Default::default() }).await
    }

    #[tokio::test]
    async fn it_connects_with_the_account_and_never_opens_a_browser() {
        let server = account_server().await;
        let (_dir, store, group, _agent) = workspace();

        let plugin = plugins::connect(
            &store,
            group,
            &PluginKind::Google,
            &format!("{}/mcp", server.base()),
            plugins::Credential::Account(plugins::AccountUse { token: ACCOUNT, connection: "" }),
            |_| panic!("an account-backed plugin must not send anyone to a browser"),
        )
        .await
        .expect("the account token is the sign-in");

        assert_eq!(plugin.kind, PluginKind::Google);
        assert!(!plugin.tools.is_empty(), "the tool list is read the same way as any other");
        // Nothing was registered, because no client was: the account's own
        // sign-in already happened somewhere this server never saw.
        assert_eq!(server.registrations.load(Ordering::SeqCst), 0);
        assert!(
            server.seen.lock().iter().flatten().any(|token| token == ACCOUNT),
            "the account token has to reach the server or nothing is authenticated"
        );
    }

    #[tokio::test]
    async fn moving_a_crew_to_another_google_keeps_who_may_call_what() {
        // The whole reason changing the identity is its own act rather than
        // Disconnect and Connect. An operator moving a crew from the work
        // mailbox to the personal one is not deciding anything about who may
        // send mail, and a move that quietly handed `gmail_send` back to every
        // agent would undo that decision at the moment they were doing
        // something else.
        //
        // The row keeps its id and the tool list is re-read, so this is the
        // seam where the two could disagree: the narrowings are filed against
        // the plugin and the tool by name, and neither is what changed.
        let server = account_server().await;
        let (_dir, store, group, agent) = workspace();
        let endpoint = format!("{}/mcp", server.base());

        let plugin = plugins::connect(
            &store,
            group,
            &PluginKind::Google,
            &endpoint,
            plugins::Credential::Account(plugins::AccountUse {
                token: ACCOUNT,
                connection: "work",
            }),
            |_| Ok(()),
        )
        .await
        .unwrap();
        // The scripted server publishes the same two tools for every kind, so
        // these stand in for `gmail_search` and `gmail_send`.
        store
            .set_plugin_tool(plugin.id, "run_sql", &PluginAccess::Chosen { agents: vec![agent] })
            .unwrap();
        store.set_plugin_tool(plugin.id, "list_projects", &NOBODY).unwrap();

        let moved = plugins::connect(
            &store,
            group,
            &PluginKind::Google,
            &endpoint,
            plugins::Credential::Account(plugins::AccountUse {
                token: ACCOUNT,
                connection: "personal",
            }),
            |_| Ok(()),
        )
        .await
        .unwrap();

        assert_eq!(moved.id, plugin.id, "the row moves rather than being replaced");
        let mine = moved.tools.iter().find(|t| t.name == "run_sql").expect("still published");
        assert_eq!(mine.access, PluginAccess::Chosen { agents: vec![agent] });
        let off = moved.tools.iter().find(|t| t.name == "list_projects").expect("still published");
        assert_eq!(off.access, NOBODY);
        // And the call path agrees with the panel, which is the half that
        // decides what an agent actually gets.
        assert!(matches!(
            store.plugin_reach(group, agent, &PluginKind::Google, "list_projects").unwrap(),
            PluginReach::ToolDenied
        ));
    }

    #[tokio::test]
    async fn it_stores_no_grant_of_its_own() {
        // The account rotates its own token. A copy on this row would be a
        // second thing to keep fresh and a second thing to be stale, and the
        // renewal path would race the account's.
        let server = account_server().await;
        let (_dir, store, group, agent) = workspace();
        let endpoint = format!("{}/mcp", server.base());

        plugins::connect(
            &store,
            group,
            &PluginKind::Google,
            &endpoint,
            plugins::Credential::Account(plugins::AccountUse { token: ACCOUNT, connection: "" }),
            |_| Ok(()),
        )
        .await
        .unwrap();

        match store.plugin_reach(group, agent, &PluginKind::Google, "gmail_search").unwrap() {
            guac_lib::db::store::PluginReach::Granted { grant, .. } => {
                assert!(grant.is_none(), "an account-backed plugin holds no grant of its own");
            }
            other => panic!("expected the plugin to be reachable, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn connecting_without_an_account_says_what_to_do_about_it() {
        let server = account_server().await;
        let (_dir, store, group, _agent) = workspace();

        let failed = plugins::connect(
            &store,
            group,
            &PluginKind::Google,
            &format!("{}/mcp", server.base()),
            plugins::Credential::Discover,
            |_| Ok(()),
        )
        .await
        .expect_err("there is no account to sign in with");

        let said = failed.to_string();
        assert!(said.contains("Guaca account"), "{said}");
        assert!(said.contains("Settings"), "{said}");
        assert!(said.contains("guaca.bot"), "{said}");
    }

    #[tokio::test]
    async fn a_call_carries_the_account_token() {
        let server = account_server().await;
        let (_dir, store, group, agent) = workspace();
        let endpoint = format!("{}/mcp", server.base());

        plugins::connect(
            &store,
            group,
            &PluginKind::Google,
            &endpoint,
            plugins::Credential::Account(plugins::AccountUse { token: ACCOUNT, connection: "" }),
            |_| Ok(()),
        )
        .await
        .unwrap();

        let answer = plugins::call(
            &store,
            plugins::Target {
                group,
                agent,
                kind: &PluginKind::Google,
                endpoint: &endpoint,
                account: Some(plugins::AccountUse { token: ACCOUNT, connection: "" }),
            },
            "run_sql",
            &serde_json::json!({ "sql": "select 1" }),
        )
        .await
        .expect("the call goes through on the account's token");

        assert_eq!(answer, "run_sql ran");
    }

    #[tokio::test]
    async fn a_call_after_signing_out_says_so_rather_than_failing_on_the_wire() {
        // Signing the account out mid-session reads as no token at all. The
        // agent gets the sentence about signing in, not a transport error it
        // would reword and retry.
        let server = account_server().await;
        let (_dir, store, group, agent) = workspace();
        let endpoint = format!("{}/mcp", server.base());

        plugins::connect(
            &store,
            group,
            &PluginKind::Google,
            &endpoint,
            plugins::Credential::Account(plugins::AccountUse { token: ACCOUNT, connection: "" }),
            |_| Ok(()),
        )
        .await
        .unwrap();

        let failed = plugins::call(
            &store,
            plugins::Target {
                group,
                agent,
                kind: &PluginKind::Google,
                endpoint: &endpoint,
                account: None,
            },
            "run_sql",
            &serde_json::json!({}),
        )
        .await
        .expect_err("there is no account token any more");

        assert!(failed.to_string().contains("Guaca account"), "{failed}");
    }

    #[tokio::test]
    async fn an_agent_the_operator_did_not_choose_still_cannot_call_it() {
        // The whole reason this is a plugin rather than a second kind of
        // credential. The reach rule is the same one every other plugin gets,
        // and the account being the credential must not quietly widen it.
        let server = account_server().await;
        let (_dir, store, group, agent) = workspace();
        let endpoint = format!("{}/mcp", server.base());

        let plugin = plugins::connect(
            &store,
            group,
            &PluginKind::Google,
            &endpoint,
            plugins::Credential::Account(plugins::AccountUse { token: ACCOUNT, connection: "" }),
            |_| Ok(()),
        )
        .await
        .unwrap();

        // Chosen, and this agent is not among them.
        store
            .set_plugin_access(
                plugin.id,
                &guac_lib::domain::plugin::PluginAccess::Chosen { agents: Vec::new() },
            )
            .unwrap();

        let failed = plugins::call(
            &store,
            plugins::Target {
                group,
                agent,
                kind: &PluginKind::Google,
                endpoint: &endpoint,
                account: Some(plugins::AccountUse { token: ACCOUNT, connection: "" }),
            },
            "run_sql",
            &serde_json::json!({}),
        )
        .await
        .expect_err("this agent was not chosen");

        let said = failed.to_string();
        assert!(said.contains("not for you"), "{said}");
        assert_eq!(
            server.called.lock().len(),
            0,
            "a refusal must happen before the server is dialled, or the check is decoration"
        );
    }
}

/// The real client against a real `guaca.bot`, which no stub can stand in for.
///
/// Everything above drives `plugins::connect` against a scripted server that
/// this repository also wrote, so the two agree by construction. The failure
/// worth catching is the one where they stop agreeing with the service: a
/// header the Worker does not read, a content type Guaca does not sniff, a
/// session id one side invents. It reaches the network, authorizes nothing and
/// spends nothing.
///
/// Needs an account token, because the sign-in behind one is a browser and a
/// person. `scripts/account.sh` in guaca-bot prints one against a local Worker.
///
/// ```sh
/// GUACA_ACCOUNT_ORIGIN=http://localhost:8787 GUACA_ACCOUNT_TOKEN=... \
///   cargo test --manifest-path src-tauri/Cargo.toml --test plugins -- --ignored
/// ```
#[tokio::test]
#[ignore = "reaches a running guaca.bot"]
async fn the_real_account_server_still_speaks_what_this_client_sends() {
    let Ok(token) = std::env::var("GUACA_ACCOUNT_TOKEN") else {
        panic!("set GUACA_ACCOUNT_TOKEN to an account access token");
    };
    let origin = std::env::var("GUACA_ACCOUNT_ORIGIN")
        .unwrap_or_else(|_| guac_lib::account::DEFAULT_ORIGIN.to_string());
    let endpoint = format!("{}/mcp", origin.trim_end_matches('/'));

    let (_dir, store, group, agent) = workspace();

    let plugin = plugins::connect(
        &store,
        group,
        &PluginKind::Google,
        &endpoint,
        plugins::Credential::Account(plugins::AccountUse { token: &token, connection: "" }),
        |_| panic!("an account-backed plugin must not open a browser"),
    )
    .await
    .expect("the account token should connect");

    // A grant with nothing authorized offers nothing, which is a real state and
    // not a failure: it means the operator has not authorized Google yet.
    let offered: Vec<&str> = plugin.tools.iter().map(|card| card.name.as_str()).collect();
    println!("connected with {} tool(s): {offered:?}", offered.len());

    if let Some(tool) = offered.first().copied() {
        // Whatever it answers, it has to answer as MCP rather than as a
        // transport failure. A refusal from Google is a legitimate result here.
        let called = plugins::call(
            &store,
            plugins::Target {
                group,
                agent,
                kind: &PluginKind::Google,
                endpoint: &endpoint,
                account: Some(plugins::AccountUse { token: &token, connection: "" }),
            },
            tool,
            &serde_json::json!({}),
        )
        .await;
        match called {
            Ok(answer) => {
                println!("{tool} answered: {}", answer.chars().take(200).collect::<String>())
            }
            // A tool that ran and said no is an MCP answer, not a broken one:
            // calling it with no arguments is exactly how a tool refuses. What
            // must not happen is a transport or protocol failure, which is
            // every other variant.
            Err(guac_lib::plugins::PluginError::Server(guac_lib::mcp::McpError::Rejected {
                message,
            })) => println!("{tool} refused, which is an answer: {message}"),
            Err(err) => panic!("{tool} did not answer as MCP: {err}"),
        }
    }
}

// ---- servers the operator added -----------------------------------------

/// Everything below is a server nobody vouched for, and the point of every one
/// of them is that nothing else about a plugin changes because of it: the same
/// probe, the same sign-in, the same tool list, the same per-agent and per-tool
/// answers, and a grant that never leaves the store except onto the wire.
mod added {
    use super::*;

    /// The kind an operator would end up with, as `PluginKind::custom` builds
    /// it. Built through the constructor rather than by hand, so a test cannot
    /// use a name or an address the app itself would refuse.
    fn mine(server: &Server) -> PluginKind {
        PluginKind::custom("Home Assistant", &format!("{}/mcp", server.base())).unwrap()
    }

    #[tokio::test]
    async fn a_server_the_operator_typed_signs_in_like_any_other() {
        let server = serve(Rules::default()).await;
        let (_dir, store, group, agent) = workspace();
        let kind = mine(&server);

        let plugin = plugins::connect(
            &store,
            group,
            &kind,
            kind.endpoint(),
            plugins::Credential::Discover,
            browser(Outcome::Allowed),
        )
        .await
        .expect("a server nobody vouched for signs in exactly as a vendor's does");

        // The name is the one the crew will call it by, not the one that was
        // typed, and the row says nobody checked it.
        assert_eq!(plugin.name, "home_assistant");
        assert_eq!(plugin.kind.slug(), "home_assistant");
        assert!(plugin.custom);
        assert!(plugin.signed_in);
        assert_eq!(plugin.endpoint, kind.endpoint());
        assert_eq!(server.registrations.load(Ordering::SeqCst), 1);

        // And it comes back out of the store as the same server, address and
        // all: the address is on the row, because there is no catalog entry to
        // look it up in.
        let held = store.group_plugins(group).unwrap();
        assert_eq!(held.len(), 1);
        assert_eq!(held[0].kind, kind);

        // Offered to the crew under its own prefix, and callable by it.
        let connected = store.plugin_tools(group, agent).unwrap();
        let specs = tools::plugin_specs(&connected);
        let names: Vec<&str> = specs.iter().map(|spec| spec.name.as_str()).collect();
        assert_eq!(names, vec!["home_assistant__run_sql", "home_assistant__list_projects"]);

        let answer = plugins::call(
            &store,
            plugins::Target { group, agent, kind: &kind, endpoint: kind.endpoint(), account: None },
            "run_sql",
            &serde_json::json!({ "sql": "select 1" }),
        )
        .await
        .expect("the crew's own server answers");
        assert_eq!(answer, "run_sql ran");
        assert_eq!(server.seen.lock().last().cloned().flatten(), Some("access-0".to_string()));
    }

    #[tokio::test]
    async fn its_name_only_resolves_against_a_crew_that_has_it() {
        // The one place a custom server differs from a catalog one at parse
        // time. `neon__` is a name this build knows whether or not anybody
        // connected it, so an agent calling it gets a refusal that says who can
        // connect it. A name nobody has ever heard of is not a plugin call, and
        // must not be reported as a plugin that is merely not connected: that
        // sends the operator looking for a server nobody has ever named.
        let server = serve(Rules::default()).await;
        let (_dir, store, group, _agent) = workspace();
        let kind = mine(&server);

        let call = |name: &str| guac_lib::llm::openrouter::ToolCall {
            id: "1".into(),
            name: name.into(),
            arguments: "{}".into(),
        };

        // Before it is connected: not a plugin, and not a tool either.
        let before = store.group_plugin_kinds(group).unwrap();
        assert!(tools::parse(&call("home_assistant__run_sql"), &before).is_err());
        // A catalog name is still recognized with nothing connected at all.
        assert!(matches!(
            tools::parse(&call("neon__run_sql"), &before),
            Ok(ToolInvocation::Plugin { .. })
        ));
        // And a model composing two of the app's own tool names is still an
        // unknown tool rather than a server nobody has.
        assert!(tools::parse(&call("use_screen__click"), &before).is_err());

        plugins::connect(
            &store,
            group,
            &kind,
            kind.endpoint(),
            plugins::Credential::Discover,
            browser(Outcome::Allowed),
        )
        .await
        .unwrap();

        // After: the name resolves, and it carries the address it will be
        // dialled at, because that is the only place the address exists.
        let after = store.group_plugin_kinds(group).unwrap();
        match tools::parse(&call("home_assistant__run_sql"), &after) {
            Ok(ToolInvocation::Plugin { kind: parsed, tool, .. }) => {
                assert_eq!(parsed, kind);
                assert_eq!(parsed.endpoint(), kind.endpoint());
                assert_eq!(tool, "run_sql");
            }
            other => panic!("expected a plugin call, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn an_agent_it_was_narrowed_away_from_is_sent_to_a_peer_rather_than_told_it_does_not_exist(
    ) {
        // The reason the list a name is resolved against is the crew's and not
        // the agent's. Being able to name a thing and being allowed to call it
        // are two questions, and the second one has the useful answer in it: a
        // peer has this. Resolved against the agent's own plugins, the name
        // would not resolve at all and the turn would be told the tool does not
        // exist — which names no way forward and is not what the six say.
        let server = serve(Rules::default()).await;
        let (_dir, store, group, chosen) = workspace();
        let left_out = crew(&store, group, "Scribe");
        let kind = mine(&server);

        let plugin = plugins::connect(
            &store,
            group,
            &kind,
            kind.endpoint(),
            plugins::Credential::Discover,
            browser(Outcome::Allowed),
        )
        .await
        .unwrap();
        store.set_plugin_access(plugin.id, &PluginAccess::Chosen { agents: vec![chosen] }).unwrap();

        // It is not in this agent's own list, which is what decides the tool
        // definitions and is right: it must not be offered the tool.
        assert!(store.plugin_tools(group, left_out).unwrap().is_empty());

        // And the name still resolves, because the crew has the server.
        let call = guac_lib::llm::openrouter::ToolCall {
            id: "1".into(),
            name: "home_assistant__run_sql".into(),
            arguments: "{}".into(),
        };
        let named = store.group_plugin_kinds(group).unwrap();
        assert!(matches!(tools::parse(&call, &named), Ok(ToolInvocation::Plugin { .. })));

        // So the refusal is the one with a peer in it.
        let refused = plugins::call(
            &store,
            plugins::Target {
                group,
                agent: left_out,
                kind: &kind,
                endpoint: kind.endpoint(),
                account: None,
            },
            "run_sql",
            &serde_json::json!({}),
        )
        .await
        .expect_err("an agent the operator did not choose cannot call it");
        assert!(refused.to_string().contains("not for you"), "{refused}");
        assert!(refused.to_string().contains("peer"), "{refused}");
    }

    #[tokio::test]
    async fn a_pasted_key_is_spent_as_a_bearer_and_never_opens_a_browser() {
        // The case the catalog never has: a server somebody wrote, with a token
        // minted by hand and no authorization server behind it at all. Asking
        // one of those to discover a sign-in is a round trip whose only outcome
        // is a 401 with nothing in it.
        let server =
            serve(Rules { account_token: Some("hand-minted".to_string()), ..Default::default() })
                .await;
        let (_dir, store, group, agent) = workspace();
        let kind = mine(&server);

        let plugin = plugins::connect(
            &store,
            group,
            &kind,
            kind.endpoint(),
            plugins::Credential::Key("hand-minted"),
            |_| panic!("a pasted key must not send anybody to a browser"),
        )
        .await
        .expect("a key connects");

        assert!(plugin.signed_in, "a key is a sign-in: the server accepted it");
        assert_eq!(server.registrations.load(Ordering::SeqCst), 0, "nothing to register with");

        // It is stored where a grant is stored and hidden the way one is.
        let json = serde_json::to_string(&store.group_plugins(group).unwrap()).unwrap();
        assert!(!json.contains("hand-minted"), "a key must not cross IPC: {json}");

        // And spent the same way.
        plugins::call(
            &store,
            plugins::Target { group, agent, kind: &kind, endpoint: kind.endpoint(), account: None },
            "run_sql",
            &serde_json::json!({}),
        )
        .await
        .expect("the key reaches the server");
        assert_eq!(server.seen.lock().last().cloned().flatten(), Some("hand-minted".to_string()));
    }

    #[tokio::test]
    async fn a_key_the_server_stops_taking_says_to_paste_another_rather_than_retrying_forever() {
        // There is no refresh token behind a pasted key, so the one retry the
        // OAuth path gets does not apply and must not be attempted: a refresh
        // against an empty token endpoint is a request to nowhere.
        let server =
            serve(Rules { account_token: Some("hand-minted".to_string()), ..Default::default() })
                .await;
        let (_dir, store, group, agent) = workspace();
        let kind = mine(&server);

        plugins::connect(
            &store,
            group,
            &kind,
            kind.endpoint(),
            plugins::Credential::Key("hand-minted"),
            |_| Ok(()),
        )
        .await
        .unwrap();

        server.revoked.lock().push("hand-minted".to_string());
        let refused = plugins::call(
            &store,
            plugins::Target { group, agent, kind: &kind, endpoint: kind.endpoint(), account: None },
            "run_sql",
            &serde_json::json!({}),
        )
        .await
        .expect_err("a key the server stopped taking is not a working plugin");

        let said = refused.to_string();
        assert!(said.contains("home_assistant"), "{said}");
        assert!(said.contains("connect it again"), "{said}");
        assert_eq!(server.refreshes.load(Ordering::SeqCst), 0, "there is nothing to refresh");
    }

    #[tokio::test]
    async fn it_is_narrowed_by_agent_and_by_tool_exactly_as_a_vendor_s_is() {
        // The whole claim this feature makes: after the address, nothing is
        // different. If any of these three had to be special-cased for a server
        // the operator added, this would be a second code path pretending to be
        // one.
        let server = serve(Rules::default()).await;
        let (_dir, store, group, triage) = workspace();
        let answerer = crew(&store, group, "Answerer");
        let kind = mine(&server);

        let plugin = plugins::connect(
            &store,
            group,
            &kind,
            kind.endpoint(),
            plugins::Credential::Discover,
            browser(Outcome::Allowed),
        )
        .await
        .unwrap();

        // One tool for one agent, and the plugin itself for both.
        store
            .set_plugin_tool(plugin.id, "run_sql", &PluginAccess::Chosen { agents: vec![answerer] })
            .unwrap();

        assert!(matches!(
            store.plugin_reach(group, answerer, &kind, "run_sql").unwrap(),
            PluginReach::Granted { .. }
        ));
        assert!(matches!(
            store.plugin_reach(group, triage, &kind, "run_sql").unwrap(),
            PluginReach::ToolNotChosen
        ));

        // The refusal names the server by the name its tools are called by,
        // because that is the only name the agent has ever seen.
        let refused = plugins::call(
            &store,
            plugins::Target {
                group,
                agent: triage,
                kind: &kind,
                endpoint: kind.endpoint(),
                account: None,
            },
            "run_sql",
            &serde_json::json!({}),
        )
        .await
        .expect_err("a tool narrowed away is refused here, not at the server");
        assert!(refused.to_string().contains("home_assistant"), "{refused}");

        // And what each is offered differs, on one sign-in.
        let theirs = store.plugin_tools(group, answerer).unwrap();
        let mine_now = store.plugin_tools(group, triage).unwrap();
        assert!(theirs[0].offered.iter().any(|tool| tool.name == "run_sql"));
        assert!(!mine_now[0].offered.iter().any(|tool| tool.name == "run_sql"));
        assert_eq!(mine_now[0].elsewhere, vec!["run_sql"]);
    }

    #[tokio::test]
    async fn two_of_them_in_one_crew_keep_their_own_addresses() {
        // Which is what the name being the prefix buys, and what the unique
        // index over (group, kind) is now also doing: two servers under one
        // name would put two tool lists behind one prefix, and which one a call
        // landed on would depend on row order.
        let first = serve(Rules { needs_token: false, ..Default::default() }).await;
        let second = serve(Rules { needs_token: false, ..Default::default() }).await;
        let (_dir, store, group, agent) = workspace();

        let vault = PluginKind::custom("Vault", &format!("{}/mcp", first.base())).unwrap();
        let notes = PluginKind::custom("Notes", &format!("{}/mcp", second.base())).unwrap();
        for kind in [&vault, &notes] {
            plugins::connect(
                &store,
                group,
                kind,
                kind.endpoint(),
                plugins::Credential::Discover,
                |_| Ok(()),
            )
            .await
            .unwrap();
        }

        assert_eq!(store.group_plugins(group).unwrap().len(), 2);
        let specs = tools::plugin_specs(&store.plugin_tools(group, agent).unwrap());
        assert!(specs.iter().any(|spec| spec.name == "vault__run_sql"));
        assert!(specs.iter().any(|spec| spec.name == "notes__run_sql"));

        // And a call reaches the one it named rather than the other.
        plugins::call(
            &store,
            plugins::Target {
                group,
                agent,
                kind: &notes,
                endpoint: notes.endpoint(),
                account: None,
            },
            "run_sql",
            &serde_json::json!({}),
        )
        .await
        .unwrap();
        assert_eq!(second.called.lock().len(), 1);
        assert!(first.called.lock().is_empty());
    }
}

// ---- the two protocol eras ----------------------------------------------

/// A client that speaks one era is a client that cannot reach half the servers
/// in the field, so it speaks both. The legacy half is every other test in this
/// file — every vendor on the list shakes hands today — and this is the other.
mod eras {
    use super::*;

    fn modern(rules: Rules) -> Rules {
        Rules { era: Era::Modern, ..rules }
    }

    #[tokio::test]
    async fn a_modern_server_is_reached_with_no_handshake_at_all() {
        let server = serve(modern(Rules { needs_token: false, ..Default::default() })).await;
        let (_dir, store, group, agent) = workspace();
        let kind = PluginKind::custom("modern", &format!("{}/mcp", server.base())).unwrap();

        let plugin = plugins::connect(
            &store,
            group,
            &kind,
            kind.endpoint(),
            plugins::Credential::Discover,
            |_| panic!("this server authorizes everybody"),
        )
        .await
        .expect("a modern server connects");

        let named: Vec<&str> = plugin.tools.iter().map(|tool| tool.name.as_str()).collect();
        assert_eq!(named, vec!["run_sql", "list_projects"]);
        // It named itself through `server/discover`, which is where the modern
        // revision moved `serverInfo` to.
        assert_eq!(plugin.account, "Scripted MCP Server");

        plugins::call(
            &store,
            plugins::Target { group, agent, kind: &kind, endpoint: kind.endpoint(), account: None },
            "run_sql",
            &serde_json::json!({ "sql": "select 1" }),
        )
        .await
        .expect("a modern call goes through");

        let noted = server.noted.lock().clone();
        // Nothing shook hands, and nothing was asked to.
        assert!(!noted.is_empty());
        // Every request declared its version in the header beside the body, and
        // named its own method. The server refuses the request outright if the
        // two disagree, so reaching this line at all is most of the assertion.
        for seen in &noted {
            assert_eq!(
                seen.get("mcp-protocol-version").map(String::as_str),
                Some(PROTOCOL_VERSION)
            );
            assert!(seen.contains_key("mcp-method"), "{seen:?}");
            assert!(!seen.contains_key("mcp-session-id"), "a modern server mints none: {seen:?}");
        }
        let called = noted
            .iter()
            .find(|seen| seen.get("mcp-method").map(String::as_str) == Some("tools/call"))
            .unwrap();
        assert_eq!(called.get("mcp-name").map(String::as_str), Some("run_sql"));
    }

    #[tokio::test]
    async fn a_legacy_server_is_still_shaken_hands_with_after_the_modern_probe_fails() {
        // The spec's own fallback rule, and the case a status-code-only reading
        // gets wrong: this server answers an unknown method with 200 and a
        // JSON-RPC error, which is neither a 400 nor a modern error shape.
        let server = serve(Rules { needs_token: false, ..Default::default() }).await;
        let (_dir, store, group, _agent) = workspace();
        let kind = PluginKind::custom("legacy", &format!("{}/mcp", server.base())).unwrap();

        plugins::connect(
            &store,
            group,
            &kind,
            kind.endpoint(),
            plugins::Credential::Discover,
            |_| Ok(()),
        )
        .await
        .expect("a legacy server connects through the fallback");

        let noted = server.noted.lock().clone();
        // Probed once, then handed the handshake. Nothing after the probe
        // carries the modern headers, which a strict legacy server would refuse.
        assert!(noted.iter().all(|seen| !seen.contains_key("mcp-name")), "{noted:?}");
        // And the version every later request declares is the one the handshake
        // settled on, not the one this build would have preferred.
        assert!(
            noted
                .iter()
                .filter(|seen| seen.get("mcp-protocol-version").is_some())
                .any(|seen| seen["mcp-protocol-version"] == "2025-06-18"),
            "the negotiated revision has to be the one sent afterward: {noted:?}"
        );
    }

    #[tokio::test]
    async fn a_modern_server_connected_twice_signs_in_twice() {
        // The bug the era cache introduced, and it only ever showed on the
        // second connect. A modern server has no handshake, so a remembered era
        // makes `open` return a session with no request in it — and the
        // deliberate unauthenticated first call, which is the whole way Guaca
        // finds out a grant is wanted, never went out. The first crew signed in
        // and every crew after it got a raw 401 out of the tool list.
        let server = serve(modern(Rules::default())).await;
        let (_dir, store, group, _agent) = workspace();
        let kind = PluginKind::custom("twice", &format!("{}/mcp", server.base())).unwrap();

        for _ in 0..2 {
            plugins::connect(
                &store,
                group,
                &kind,
                kind.endpoint(),
                plugins::Credential::Discover,
                browser(Outcome::Allowed),
            )
            .await
            .expect("a modern server signs in every time it is connected");
        }

        assert_eq!(store.group_plugins(group).unwrap().len(), 1);
        assert_eq!(server.registrations.load(Ordering::SeqCst), 2, "each connect is its own");
    }

    #[tokio::test]
    async fn a_server_that_shares_no_revision_says_so_rather_than_reading_as_a_bad_address() {
        // Nothing an operator can do fixes this one, and it must not look like
        // a typo in the URL, which is the other reason a connect fails.
        let server = serve(modern(Rules {
            needs_token: false,
            versions: vec!["2099-01-01".to_string()],
            ..Default::default()
        }))
        .await;
        let (_dir, store, group, _agent) = workspace();
        let kind = PluginKind::custom("future", &format!("{}/mcp", server.base())).unwrap();

        let refused = plugins::connect(
            &store,
            group,
            &kind,
            kind.endpoint(),
            plugins::Credential::Discover,
            |_| Ok(()),
        )
        .await
        .expect_err("a revision this build does not have is not a connection");

        let said = refused.to_string();
        assert!(said.contains("2099-01-01"), "{said}");
        assert!(said.contains("update Guaca"), "{said}");
        assert!(store.group_plugins(group).unwrap().is_empty());
    }

    #[tokio::test]
    async fn an_argument_the_server_wants_in_a_header_is_put_there() {
        // Optional for a server and mandatory for a client, which means the
        // only proof is a server that validates it and refuses a call that
        // arrives without it. This one does exactly what the spec tells it to.
        let server =
            serve(modern(Rules { needs_token: false, mirrors: true, ..Default::default() })).await;
        let (_dir, store, group, agent) = workspace();
        let kind = PluginKind::custom("mirrored", &format!("{}/mcp", server.base())).unwrap();

        let plugin = plugins::connect(
            &store,
            group,
            &kind,
            kind.endpoint(),
            plugins::Credential::Discover,
            |_| Ok(()),
        )
        .await
        .unwrap();

        // The tool whose annotation nothing could honor is dropped rather than
        // offered: a call to it would be refused by the server every time, for
        // a reason no model can act on.
        let named: Vec<&str> = plugin.tools.iter().map(|tool| tool.name.as_str()).collect();
        assert_eq!(named, vec!["run_sql", "list_projects", "deploy"]);
        assert!(!named.contains(&"broken"), "an unreadable annotation drops one tool, not all");

        let answer = plugins::call(
            &store,
            plugins::Target { group, agent, kind: &kind, endpoint: kind.endpoint(), account: None },
            "deploy",
            &serde_json::json!({ "region": "us-west1", "what": "the worker" }),
        )
        .await
        .expect("the header the server asked for is on the request");
        assert_eq!(answer, "deploy ran");

        let noted = server.noted.lock().clone();
        let called = noted
            .iter()
            .find(|seen| seen.get("mcp-method").map(String::as_str) == Some("tools/call"))
            .unwrap();
        assert_eq!(called.get("mcp-param-region").map(String::as_str), Some("us-west1"));
    }
}

#[tokio::test]
async fn a_call_on_an_unconnected_plugin_says_who_can_connect_it() {
    let (_dir, store, group, agent) = workspace();

    let failed = plugins::call(
        &store,
        plugins::Target {
            group,
            agent,
            kind: &PluginKind::Neon,
            endpoint: "http://127.0.0.1:1/mcp",
            account: None,
        },
        "run_sql",
        &serde_json::json!({}),
    )
    .await
    .expect_err("nothing is connected");

    // An agent reads this mid-turn, so it has to close the door rather than
    // invite a retry: nothing the agent can do will connect a plugin.
    let said = failed.to_string();
    assert!(said.contains("not connected"), "{said}");
    assert!(said.contains("operator"), "{said}");
}

#[tokio::test]
async fn a_stale_grant_is_renewed_before_it_is_spent() {
    // The renewal happens ahead of expiry rather than on rejection: a turn that
    // discovers the token expired has already spent the operator's wait.
    let server = serve(Rules { issue_expired: true, ..Default::default() }).await;
    let endpoint = format!("{}/mcp", server.base());
    let (_dir, store, group, agent) = workspace();

    plugins::connect(
        &store,
        group,
        &PluginKind::Neon,
        &endpoint,
        plugins::Credential::Discover,
        browser(Outcome::Allowed),
    )
    .await
    .unwrap();

    // Everything before this line was the sign-in spending a token it had just
    // been handed, which is correct however short its life is. What matters is
    // the next turn.
    let after_connecting = server.seen.lock().len();

    plugins::call(
        &store,
        plugins::Target {
            group,
            agent,
            kind: &PluginKind::Neon,
            endpoint: &endpoint,
            account: None,
        },
        "run_sql",
        &serde_json::json!({}),
    )
    .await
    .expect("a stale grant is renewed rather than refused");

    assert_eq!(server.refreshes.load(Ordering::SeqCst), 1);
    assert!(
        server.seen.lock()[after_connecting..].iter().flatten().all(|token| token != "access-0"),
        "an expired token must not be spent, and must not be discovered by being refused"
    );
}

#[tokio::test]
async fn a_grant_revoked_at_the_vendor_is_renewed_once_and_the_call_retried() {
    // Nothing local says a token was revoked, and the stored expiry still looks
    // fine. The 401 is the only signal there is, and one retry is the whole
    // allowance: a refresh that does not fix it is a sign-in to redo.
    let server = serve(Rules::default()).await;
    let endpoint = format!("{}/mcp", server.base());
    let (_dir, store, group, agent) = workspace();

    plugins::connect(
        &store,
        group,
        &PluginKind::Neon,
        &endpoint,
        plugins::Credential::Discover,
        browser(Outcome::Allowed),
    )
    .await
    .unwrap();

    server.revoked.lock().push("access-0".to_string());

    let answer = plugins::call(
        &store,
        plugins::Target {
            group,
            agent,
            kind: &PluginKind::Neon,
            endpoint: &endpoint,
            account: None,
        },
        "run_sql",
        &serde_json::json!({}),
    )
    .await
    .expect("one refresh and one retry is enough");

    assert_eq!(answer, "run_sql ran");
    assert_eq!(server.refreshes.load(Ordering::SeqCst), 1, "exactly one renewal, not a loop");

    // And the renewed grant is written back, or every later turn pays for the
    // same discovery.
    let PluginReach::Granted { grant, .. } =
        store.plugin_reach(group, agent, &PluginKind::Neon, "run_sql").unwrap()
    else {
        panic!("the plugin is connected and this agent was not excluded from it")
    };
    assert_eq!(grant.unwrap().access_token, "access-1");
}

#[tokio::test]
async fn disconnecting_forgets_the_plugin_and_its_grant() {
    let server = serve(Rules::default()).await;
    let endpoint = format!("{}/mcp", server.base());
    let (_dir, store, group, agent) = workspace();

    let plugin = plugins::connect(
        &store,
        group,
        &PluginKind::Neon,
        &endpoint,
        plugins::Credential::Discover,
        browser(Outcome::Allowed),
    )
    .await
    .unwrap();

    assert!(store.delete_plugin(plugin.id).unwrap());
    assert!(store.group_plugins(group).unwrap().is_empty());
    assert!(matches!(
        store.plugin_reach(group, agent, &PluginKind::Neon, "run_sql").unwrap(),
        PluginReach::NotConnected
    ));
}

#[tokio::test]
async fn connecting_twice_replaces_the_grant_rather_than_refusing() {
    // Connecting again is how an operator fixes a sign-in that was revoked at
    // the vendor. A unique index that rejected the second one would leave them
    // with no way back except disconnecting first.
    let server = serve(Rules::default()).await;
    let endpoint = format!("{}/mcp", server.base());
    let (_dir, store, group, agent) = workspace();

    plugins::connect(
        &store,
        group,
        &PluginKind::Neon,
        &endpoint,
        plugins::Credential::Discover,
        browser(Outcome::Allowed),
    )
    .await
    .unwrap();
    plugins::connect(
        &store,
        group,
        &PluginKind::Neon,
        &endpoint,
        plugins::Credential::Discover,
        browser(Outcome::Allowed),
    )
    .await
    .unwrap();

    assert_eq!(store.group_plugins(group).unwrap().len(), 1);
    let PluginReach::Granted { grant, .. } =
        store.plugin_reach(group, agent, &PluginKind::Neon, "run_sql").unwrap()
    else {
        panic!("connecting again leaves a plugin the crew can use")
    };
    assert_eq!(grant.unwrap().access_token, "access-1", "the newer sign-in is the one held");
}

#[tokio::test]
async fn what_the_crew_connected_is_what_the_model_is_offered() {
    // The seam between the store and the turn. A tool list read from the store
    // and never turned into a definition is a plugin that looks connected and
    // cannot be called.
    let server = serve(Rules::default()).await;
    let endpoint = format!("{}/mcp", server.base());
    let (_dir, store, group, agent) = workspace();

    plugins::connect(
        &store,
        group,
        &PluginKind::Neon,
        &endpoint,
        plugins::Credential::Discover,
        browser(Outcome::Allowed),
    )
    .await
    .unwrap();

    let connected = store.plugin_tools(group, agent).unwrap();
    let specs = tools::plugin_specs(&connected);
    let names: Vec<&str> = specs.iter().map(|spec| spec.name.as_str()).collect();
    assert_eq!(names, vec!["neon__run_sql", "neon__list_projects"]);

    // A tool that declared no schema has to arrive as an object rather than as
    // null, which is a malformed function definition and takes the whole turn's
    // tool list down with it.
    let bare = specs.iter().find(|spec| spec.name == "neon__list_projects").unwrap();
    assert_eq!(bare.parameters["type"], serde_json::json!("object"));
    assert!(
        bare.description.contains("Neon"),
        "a description has to say where the tool reaches: {}",
        bare.description
    );

    // And the name a model calls comes back apart the same way.
    let call = guac_lib::llm::openrouter::ToolCall {
        id: "1".into(),
        name: "neon__run_sql".into(),
        arguments: r#"{"sql":"select 1"}"#.into(),
    };
    match tools::parse(&call, &[]).expect("a plugin tool parses") {
        ToolInvocation::Plugin { kind, tool, arguments } => {
            assert_eq!(kind, PluginKind::Neon);
            assert_eq!(tool, "run_sql");
            assert_eq!(arguments["sql"], serde_json::json!("select 1"));
        }
        other => panic!("expected a plugin call, got {other:?}"),
    }
}

#[tokio::test]
async fn a_crew_with_a_plugin_calls_it_through_a_real_turn() {
    // The seam every other test here stops short of: the tool list on the
    // request, the name coming back apart, the dispatch, the grant being spent,
    // and the answer arriving as a tool result the model can read. Each half is
    // covered above; this is the only test that says they meet.
    let server = serve(Rules::default()).await;
    let endpoint = format!("{}/mcp", server.base());

    let model = harness::serve(|body| {
        if harness::has_tool_result(body) {
            harness::Script::Say("The database says one.".into())
        } else {
            harness::Script::Plugin {
                name: "neon__run_sql".into(),
                arguments: serde_json::json!({ "sql": "select 1" }),
            }
        }
    })
    .await;

    let h =
        harness::harness(&model, &["Manager"], guac_lib::runtime::guard::GuardLimits::default());
    h.runtime.plugins_at(HashMap::from([(PluginKind::Neon.slug().to_string(), endpoint.clone())]));

    let group = h.runtime.store().get_agent(h.id("Manager")).unwrap().unwrap().group_id;
    plugins::connect(
        h.runtime.store(),
        group,
        &PluginKind::Neon,
        &endpoint,
        plugins::Credential::Discover,
        browser(Outcome::Allowed),
    )
    .await
    .unwrap();

    let run = h.runtime.send_from_human(h.id("Manager"), "Check the database.").unwrap();
    h.settle(run).await;

    // The definitions reached the provider, under the prefixed name.
    let offered: Vec<String> = model.transcript.lock()[0]["tools"]
        .as_array()
        .expect("a turn carries tool definitions")
        .iter()
        .map(|tool| tool["function"]["name"].as_str().unwrap_or_default().to_string())
        .collect();
    assert!(offered.contains(&"neon__run_sql".to_string()), "offered {offered:?}");
    // And the app's own tools are still there. A plugin that displaced
    // `send_message` would be a crew that cannot talk to itself.
    assert!(offered.contains(&"send_message".to_string()), "offered {offered:?}");

    // The call landed on the server, unprefixed and with its arguments intact.
    assert_eq!(
        server.called.lock().first().cloned(),
        Some(("run_sql".to_string(), serde_json::json!({ "sql": "select 1" }))),
    );

    // And what the server said came back as the tool result the model read.
    let results = harness::tool_results(&model);
    assert!(results.iter().any(|r| r.contains("run_sql ran")), "got {results:?}");

    h.expect_normal(run, "a turn that calls a plugin");
}

#[tokio::test]
async fn an_agent_calling_a_plugin_its_crew_has_not_connected_is_told_who_can() {
    // A model may emit a plugin name it read about anywhere. The refusal has to
    // be a way forward rather than "unknown tool", which reads to a model as a
    // spelling mistake worth trying again.
    let model = harness::serve(|body| {
        if harness::has_tool_result(body) {
            harness::Script::Say("I cannot reach it.".into())
        } else {
            harness::Script::Plugin {
                name: "neon__run_sql".into(),
                arguments: serde_json::json!({}),
            }
        }
    })
    .await;

    let h =
        harness::harness(&model, &["Manager"], guac_lib::runtime::guard::GuardLimits::default());
    let run = h.runtime.send_from_human(h.id("Manager"), "Check the database.").unwrap();
    h.settle(run).await;

    let results = harness::tool_results(&model);
    assert!(
        results.iter().any(|r| r.contains("not connected") && r.contains("operator")),
        "got {results:?}"
    );
    // The turn still finishes and still answers the operator. A failed tool
    // call is a thing to work around, not a dead turn, so `expect_normal` is
    // deliberately not asserted here: the trajectory suite counts that failure,
    // which is exactly what it is for.
    assert_eq!(h.channel_texts("Manager").len(), 2, "the operator is answered either way");
}

#[tokio::test]
async fn an_agent_the_plugin_was_narrowed_away_from_is_neither_told_nor_allowed() {
    // The whole feature, on the two seams it has to hold at once. A crew signs
    // in once and the operator decides who may spend it: the agent that was not
    // chosen is not offered the tools, is not told the crew has the plugin, and
    // is refused if it names one anyway. The third is not redundant. A model
    // emits a tool name it read somewhere often enough that the tool list is a
    // description of what an agent has, never a fence.
    let server = serve(Rules::default()).await;
    let endpoint = format!("{}/mcp", server.base());

    let model = harness::serve(|body| {
        if harness::has_tool_result(body) {
            harness::Script::Say("I could not do that part.".into())
        } else {
            harness::Script::Plugin {
                name: "neon__run_sql".into(),
                arguments: serde_json::json!({ "sql": "select 1" }),
            }
        }
    })
    .await;

    let h = harness::harness(
        &model,
        &["Revenue", "Scribe"],
        guac_lib::runtime::guard::GuardLimits::default(),
    );
    h.runtime.plugins_at(HashMap::from([(PluginKind::Neon.slug().to_string(), endpoint.clone())]));

    let group = h.runtime.store().get_agent(h.id("Revenue")).unwrap().unwrap().group_id;
    let plugin = plugins::connect(
        h.runtime.store(),
        group,
        &PluginKind::Neon,
        &endpoint,
        plugins::Credential::Discover,
        browser(Outcome::Allowed),
    )
    .await
    .unwrap();
    h.runtime
        .store()
        .set_plugin_access(plugin.id, &PluginAccess::Chosen { agents: vec![h.id("Revenue")] })
        .unwrap();

    let run = h.runtime.send_from_human(h.id("Scribe"), "Check the database.").unwrap();
    h.settle(run).await;

    // Nothing was offered.
    let offered: Vec<String> = model.transcript.lock()[0]["tools"]
        .as_array()
        .expect("a turn carries tool definitions")
        .iter()
        .map(|tool| tool["function"]["name"].as_str().unwrap_or_default().to_string())
        .collect();
    assert!(!offered.iter().any(|name| name.starts_with("neon__")), "offered {offered:?}");
    assert!(offered.contains(&"send_message".to_string()), "the app's own tools stay");

    // And nothing was claimed. The prompt and the tool list are built from one
    // read for exactly this reason: an agent told its crew has Neon and offered
    // no Neon tools spends the turn looking for them.
    let prompt = harness::prompts_by_agent(&model).remove("Scribe").expect("Scribe had a turn");
    assert!(!prompt.contains("plugins connected"), "{prompt}");

    // The call it made anyway was refused here rather than at Neon, and the
    // refusal points at the peer who can, not at the operator.
    let results = harness::tool_results(&model);
    assert!(
        results.iter().any(|r| r.contains("not for you") && r.contains("peer")),
        "got {results:?}"
    );
    assert!(server.called.lock().is_empty(), "an unauthorized call must not reach the vendor");
    assert_eq!(h.channel_texts("Scribe").len(), 2, "the operator is answered either way");
}

#[tokio::test]
async fn a_tool_the_operator_switched_off_is_named_in_the_prompt_and_refused_on_the_call() {
    // The other axis, on the same three seams. Being chosen for a plugin is not
    // being handed every tool on it: the operator decides that per tool, for
    // the whole crew, and the agent has to be able to tell the difference
    // between a capability that does not exist and one that is switched off.
    // Named in the prompt, missing from the definitions, refused on the call.
    let server = serve(Rules::default()).await;
    let endpoint = format!("{}/mcp", server.base());

    let model = harness::serve(|body| {
        if harness::has_tool_result(body) {
            harness::Script::Say("Told the operator.".into())
        } else {
            harness::Script::Plugin {
                name: "neon__run_sql".into(),
                arguments: serde_json::json!({ "sql": "select 1" }),
            }
        }
    })
    .await;

    let h =
        harness::harness(&model, &["Manager"], guac_lib::runtime::guard::GuardLimits::default());
    h.runtime.plugins_at(HashMap::from([(PluginKind::Neon.slug().to_string(), endpoint.clone())]));

    let group = h.runtime.store().get_agent(h.id("Manager")).unwrap().unwrap().group_id;
    let plugin = plugins::connect(
        h.runtime.store(),
        group,
        &PluginKind::Neon,
        &endpoint,
        plugins::Credential::Discover,
        browser(Outcome::Allowed),
    )
    .await
    .unwrap();
    h.runtime.store().set_plugin_tool(plugin.id, "run_sql", &NOBODY).unwrap();

    let run = h.runtime.send_from_human(h.id("Manager"), "Check the database.").unwrap();
    h.settle(run).await;

    // Not offered, and the plugin's other tool still is: switching one off is
    // not disconnecting the plugin.
    let offered: Vec<String> = model.transcript.lock()[0]["tools"]
        .as_array()
        .expect("a turn carries tool definitions")
        .iter()
        .map(|tool| tool["function"]["name"].as_str().unwrap_or_default().to_string())
        .collect();
    assert!(!offered.contains(&"neon__run_sql".to_string()), "offered {offered:?}");
    assert!(offered.contains(&"neon__list_projects".to_string()), "offered {offered:?}");

    // And said out loud, because the alternative is an agent answering "we
    // cannot query the database" to the one person who can switch it back on.
    let prompt = harness::prompts_by_agent(&model).remove("Manager").expect("Manager had a turn");
    assert!(prompt.contains("Switched off by the operator"), "{prompt}");
    assert!(prompt.contains("run_sql"), "{prompt}");

    // The call it made anyway was refused here rather than at Neon, and the
    // refusal does not send it round the crew: nobody has it.
    let results = harness::tool_results(&model);
    assert!(
        results.iter().any(|r| r.contains("switched off") && r.contains("Do not ask a peer")),
        "got {results:?}"
    );
    assert!(server.called.lock().is_empty(), "a switched-off tool must not reach the vendor");
    assert_eq!(h.channel_texts("Manager").len(), 2, "the operator is answered either way");
}

#[tokio::test]
async fn a_narrowed_plugin_shows_up_as_a_peer_who_can_do_it() {
    // What narrowing costs if nothing replaces it: an agent that can only say
    // no, sitting beside the one that can say yes. The roster is where the crew
    // already answers this for browser sign-ins, so it answers it here too, and
    // only for a plugin this agent does not have itself.
    let server = serve(Rules::default()).await;
    let endpoint = format!("{}/mcp", server.base());
    let model = harness::serve(|_| harness::Script::Say("Noted.".into())).await;

    let h = harness::harness(
        &model,
        &["Revenue", "Scribe"],
        guac_lib::runtime::guard::GuardLimits::default(),
    );
    h.runtime.plugins_at(HashMap::from([(PluginKind::Neon.slug().to_string(), endpoint.clone())]));

    let group = h.runtime.store().get_agent(h.id("Revenue")).unwrap().unwrap().group_id;
    let plugin = plugins::connect(
        h.runtime.store(),
        group,
        &PluginKind::Neon,
        &endpoint,
        plugins::Credential::Discover,
        browser(Outcome::Allowed),
    )
    .await
    .unwrap();
    h.runtime
        .store()
        .set_plugin_access(plugin.id, &PluginAccess::Chosen { agents: vec![h.id("Revenue")] })
        .unwrap();

    let run = h.runtime.send_from_human(h.id("Scribe"), "Anything to report?").unwrap();
    h.settle(run).await;
    let scribe = harness::prompts_by_agent(&model).remove("Scribe").expect("Scribe had a turn");
    assert!(scribe.contains("Revenue"), "{scribe}");
    assert!(scribe.contains("the Neon plugin"), "{scribe}");

    // And the agent that holds it is not told to go and ask itself.
    let run = h.runtime.send_from_human(h.id("Revenue"), "Anything to report?").unwrap();
    h.settle(run).await;
    let revenue = harness::prompts_by_agent(&model).remove("Revenue").expect("Revenue had a turn");
    assert!(!revenue.contains("the Neon plugin"), "{revenue}");
}

#[tokio::test]
async fn two_agents_on_one_plugin_are_offered_different_tools_and_told_whose_is_whose() {
    // The whole point of the second axis, end to end. One sign-in, two agents,
    // and the halves they were given: the definitions each is offered, the line
    // each prompt carries about the other half, and the refusal the model gets
    // if it names the tool anyway.
    let server = serve(Rules::default()).await;
    let endpoint = format!("{}/mcp", server.base());
    let model = harness::serve(|body| {
        if harness::has_tool_result(body) {
            harness::Script::Say("Told the operator.".into())
        } else {
            harness::Script::Plugin {
                name: "neon__list_projects".into(),
                arguments: serde_json::json!({}),
            }
        }
    })
    .await;

    let h = harness::harness(
        &model,
        &["Reader", "Writer"],
        guac_lib::runtime::guard::GuardLimits::default(),
    );
    h.runtime.plugins_at(HashMap::from([(PluginKind::Neon.slug().to_string(), endpoint.clone())]));

    let group = h.runtime.store().get_agent(h.id("Reader")).unwrap().unwrap().group_id;
    let plugin = plugins::connect(
        h.runtime.store(),
        group,
        &PluginKind::Neon,
        &endpoint,
        plugins::Credential::Discover,
        browser(Outcome::Allowed),
    )
    .await
    .unwrap();
    // Both are on the plugin. Only one is on each tool.
    h.runtime
        .store()
        .set_plugin_tool(
            plugin.id,
            "run_sql",
            &PluginAccess::Chosen { agents: vec![h.id("Writer")] },
        )
        .unwrap();
    h.runtime
        .store()
        .set_plugin_tool(
            plugin.id,
            "list_projects",
            &PluginAccess::Chosen { agents: vec![h.id("Reader")] },
        )
        .unwrap();

    let run = h.runtime.send_from_human(h.id("Reader"), "What is in the database?").unwrap();
    h.settle(run).await;

    // Offered its own half and not the other's, and the call it made went out.
    let offered: Vec<String> = model.transcript.lock()[0]["tools"]
        .as_array()
        .expect("a turn carries tool definitions")
        .iter()
        .map(|tool| tool["function"]["name"].as_str().unwrap_or_default().to_string())
        .collect();
    assert!(offered.contains(&"neon__list_projects".to_string()), "offered {offered:?}");
    assert!(!offered.contains(&"neon__run_sql".to_string()), "offered {offered:?}");
    assert!(!server.called.lock().is_empty(), "the half it was given still reaches the vendor");

    // And told whose the other half is, in the sentence that sends it to a
    // peer rather than to the operator.
    let prompt = harness::prompts_by_agent(&model).remove("Reader").expect("Reader had a turn");
    assert!(prompt.contains("Someone else's on this plugin"), "{prompt}");
    assert!(prompt.contains("run_sql"), "{prompt}");
    assert!(!prompt.contains("Switched off by the operator"), "{prompt}");
    // The roster names who, because a peer nobody can name is not a way
    // forward. The plugin line alone cannot say this: Reader has Neon.
    assert!(prompt.contains("the Neon plugin's run_sql"), "{prompt}");
}

#[tokio::test]
async fn a_tool_that_is_a_peer_s_is_refused_here_and_not_at_the_vendor() {
    // A model names tools it was never offered, so the definitions are not the
    // enforcement. The refusal has to be the one that sends the turn to a peer,
    // because a peer does have it — which is the difference from a tool
    // switched off for everybody.
    let server = serve(Rules::default()).await;
    let endpoint = format!("{}/mcp", server.base());
    let model = harness::serve(|body| {
        if harness::has_tool_result(body) {
            harness::Script::Say("Handing that to the peer who has it.".into())
        } else {
            harness::Script::Plugin {
                name: "neon__run_sql".into(),
                arguments: serde_json::json!({ "sql": "select 1" }),
            }
        }
    })
    .await;

    let h = harness::harness(
        &model,
        &["Reader", "Writer"],
        guac_lib::runtime::guard::GuardLimits::default(),
    );
    h.runtime.plugins_at(HashMap::from([(PluginKind::Neon.slug().to_string(), endpoint.clone())]));

    let group = h.runtime.store().get_agent(h.id("Reader")).unwrap().unwrap().group_id;
    let plugin = plugins::connect(
        h.runtime.store(),
        group,
        &PluginKind::Neon,
        &endpoint,
        plugins::Credential::Discover,
        browser(Outcome::Allowed),
    )
    .await
    .unwrap();
    h.runtime
        .store()
        .set_plugin_tool(
            plugin.id,
            "run_sql",
            &PluginAccess::Chosen { agents: vec![h.id("Writer")] },
        )
        .unwrap();

    let run = h.runtime.send_from_human(h.id("Reader"), "Run a query.").unwrap();
    h.settle(run).await;

    let results = harness::tool_results(&model);
    assert!(
        results.iter().any(|r| r.contains("is not") && r.contains("hand that part over")),
        "got {results:?}"
    );
    // And it is not the sentence that tells an agent to stop asking, because
    // there is somebody to ask.
    assert!(!results.iter().any(|r| r.contains("Do not ask a peer")), "got {results:?}");
    assert!(server.called.lock().is_empty(), "a tool that is a peer's must not reach the vendor");
    assert_eq!(h.channel_texts("Reader").len(), 2, "the operator is answered either way");
}

#[tokio::test]
async fn a_plugin_with_everything_switched_off_is_not_a_peer_worth_asking() {
    // The roster's whole job is to stop work being routed to an agent that
    // cannot do it. A plugin narrowed to one agent and then switched off tool
    // by tool is exactly that case from the other end: the peer holds it and
    // can call none of it, so naming them costs two turns to find out.
    let server = serve(Rules::default()).await;
    let endpoint = format!("{}/mcp", server.base());
    let model = harness::serve(|_| harness::Script::Say("Noted.".into())).await;

    let h = harness::harness(
        &model,
        &["Revenue", "Scribe"],
        guac_lib::runtime::guard::GuardLimits::default(),
    );
    h.runtime.plugins_at(HashMap::from([(PluginKind::Neon.slug().to_string(), endpoint.clone())]));

    let group = h.runtime.store().get_agent(h.id("Revenue")).unwrap().unwrap().group_id;
    let plugin = plugins::connect(
        h.runtime.store(),
        group,
        &PluginKind::Neon,
        &endpoint,
        plugins::Credential::Discover,
        browser(Outcome::Allowed),
    )
    .await
    .unwrap();
    h.runtime
        .store()
        .set_plugin_access(plugin.id, &PluginAccess::Chosen { agents: vec![h.id("Revenue")] })
        .unwrap();
    for tool in ["run_sql", "list_projects"] {
        h.runtime.store().set_plugin_tool(plugin.id, tool, &NOBODY).unwrap();
    }

    let run = h.runtime.send_from_human(h.id("Scribe"), "Anything to report?").unwrap();
    h.settle(run).await;
    let scribe = harness::prompts_by_agent(&model).remove("Scribe").expect("Scribe had a turn");
    assert!(!scribe.contains("the Neon plugin"), "{scribe}");
}

#[tokio::test]
async fn a_plugin_call_from_an_agent_outside_the_crew_is_refused() {
    // The store's own boundary, under the call path rather than under a query.
    // An agent id from another group names nobody on this plugin and is in no
    // crew that connected it, so the grant is not there to be spent.
    let server = serve(Rules::default()).await;
    let endpoint = format!("{}/mcp", server.base());
    let (_dir, store, group, agent) = workspace();

    plugins::connect(
        &store,
        group,
        &PluginKind::Neon,
        &endpoint,
        plugins::Credential::Discover,
        browser(Outcome::Allowed),
    )
    .await
    .unwrap();

    let other = store
        .create_group(&CleanGroup { name: "Elsewhere".to_string(), ..Default::default() })
        .unwrap()
        .id;
    let outsider = crew(&store, other, "Outsider");

    let failed = plugins::call(
        &store,
        plugins::Target {
            group: other,
            agent: outsider,
            kind: &PluginKind::Neon,
            endpoint: &endpoint,
            account: None,
        },
        "run_sql",
        &serde_json::json!({}),
    )
    .await
    .expect_err("another crew's sign-in is not this agent's to spend");
    assert!(failed.to_string().contains("not connected"), "{failed}");

    // And the crew that did connect it is unaffected.
    plugins::call(
        &store,
        plugins::Target {
            group,
            agent,
            kind: &PluginKind::Neon,
            endpoint: &endpoint,
            account: None,
        },
        "run_sql",
        &serde_json::json!({}),
    )
    .await
    .expect("the crew that signed in still has it");
}

// ---- live --------------------------------------------------------------

/// What the vendors on the list actually publish, right now.
///
/// Everything above is a stub agreeing with what this app believes MCP
/// authorization looks like, and the failure worth catching is that belief
/// going stale: a vendor can move an endpoint, stop offering dynamic client
/// registration, or start requiring a token on a server that used to be open,
/// and every offline test here keeps passing while no operator can connect.
///
/// Discovery is `oauth::discover`, the same call a sign-in makes, rather than
/// metadata URLs rebuilt beside it. A test with its own copy of RFC 8414 passes
/// on a vendor this build cannot reach: Stripe's authorization server is
/// `https://access.stripe.com/mcp`, and the well-known segment goes before that
/// path, not after it.
///
/// Run with `./scripts/plugins.sh`. It reaches the real internet and spends
/// nothing: no account is authorized and no tool is called.
#[tokio::test]
#[ignore = "reaches the real vendors; run ./scripts/plugins.sh"]
async fn every_server_on_the_list_still_publishes_what_this_build_expects() {
    for kind in PluginKind::ALL {
        // Except the one whose sign-in is not its own. Google's server is the
        // operator's Guaca account, which is already signed in and publishes no
        // protected-resource metadata because there is no per-group grant to
        // issue: `PluginKind::account_backed`, and *Google is a plugin whose
        // sign-in is the account's* in `docs/PLUGINS.md`. Asking the OAuth
        // question of it fails every run and always has, which is worse than
        // not asking — a live gate that is permanently red is one nobody reads,
        // and the vendor regression it exists to catch goes with it.
        // `tests/account.rs` is where that server's own contract is checked.
        if kind.account_backed() {
            println!(
                "{}: signs in with the machine's Guaca account, not with a grant of its own",
                kind.label()
            );
            continue;
        }
        let endpoint = kind.endpoint();

        // 1. An unauthenticated open, which is how Guaca finds out whether this
        //    server wants a grant at all, and where the address of the sign-in
        //    comes from when it does.
        // This is also, and mainly, the era probe against a real server. Every
        // offline test of it is a scripted server agreeing with what this build
        // believes the fallback rule is; whether six real vendors actually
        // refuse `server/discover` in a shape that reads as legacy is the thing
        // only this can answer.
        let opened = guac_lib::mcp::open(endpoint, None).await;
        let challenge = match &opened {
            Ok(session) => {
                let tools = guac_lib::mcp::list_tools(session).await.unwrap_or_else(|err| {
                    panic!("{} authorized us and then refused a tool list: {err}", kind.label())
                });
                assert!(!tools.is_empty(), "{} offered no tools", kind.label());
                println!("{}: open, {} tools", kind.label(), tools.len());
                continue;
            }
            Err(err) if err.is_unauthorized() => match err {
                guac_lib::mcp::McpError::Unauthorized { challenge, .. } => challenge.clone(),
                _ => None,
            },
            Err(err) => panic!("{} answered something unexpected: {err}", kind.label()),
        };

        // 2. Discovery, through the code a sign-in runs. A vendor that has moved
        //    its metadata somewhere none of the fallbacks look fails here, which
        //    is the whole point of doing it this way.
        let found = guac_lib::oauth::discover(endpoint, challenge.as_deref())
            .await
            .unwrap_or_else(|err| panic!("{} cannot be discovered: {err}", kind.label()));

        // 3. And that it still lets an application register itself. Without this
        //    Guaca cannot sign in at all, and the plugin has to be withdrawn
        //    rather than debugged.
        assert!(
            found.server.registration_endpoint.is_some(),
            "{} no longer lets an application register itself, so Guaca cannot sign in to it",
            kind.label()
        );
        assert!(
            found.server.code_challenge_methods_supported.iter().any(|m| m == "S256"),
            "{} no longer supports the only PKCE method this build sends",
            kind.label()
        );

        // 4. And that the scope this build would send is one the vendor
        //    publishes. Everything above passed while AgentMail refused every
        //    sign-in: Guaca asked its Clerk instance for all seven scopes the
        //    *authorization server* lists, and four of those are ones a
        //    registered client may not have. Discovery is not the only thing
        //    that goes stale, and `invalid_scope` arrives in the operator's
        //    browser where no error message can reach it.
        let asked = found.requested_scope();
        for scope in asked.iter().flat_map(|s| s.split(' ')) {
            // Whichever of the three lists this build would have taken. The
            // assertion is the same either way and it is the one that matters:
            // every scope Guaca would send is one this vendor published today.
            let published = if !found.challenge_scopes.is_empty() {
                found.challenge_scopes.iter().any(|s| s == scope)
                    || (scope == "offline_access"
                        && found.server.scopes_supported.iter().any(|s| s == scope))
            } else if found.resource_scopes.is_empty() {
                found.server.scopes_supported.iter().any(|s| s == scope)
            } else {
                // `offline_access` is the one this build adds, and only ever on
                // the authorization server's say-so.
                found.resource_scopes.iter().any(|s| s == scope)
                    || (scope == "offline_access"
                        && found.server.scopes_supported.iter().any(|s| s == scope))
            };
            assert!(
                published,
                "{} would be asked for `{scope}`, which it does not publish: \
                 resource {:?}, server {:?}",
                kind.label(),
                found.resource_scopes,
                found.server.scopes_supported
            );
            assert_ne!(scope, "*", "{} would be asked for everything", kind.label());
        }

        println!(
            "{}: signs in at {}, asking for {}",
            kind.label(),
            found.issuer,
            asked.as_deref().unwrap_or("nothing (the server's own default)")
        );
    }
}
