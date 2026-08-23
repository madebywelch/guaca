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
//! protected-resource metadata, RFC 8414 authorisation-server metadata, an
//! RFC 7591 registration endpoint, an authorisation endpoint, a token endpoint
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
use guac_lib::plugins;

/// How the scripted server behaves. Every field is something a real one does.
#[derive(Debug, Clone)]
struct Rules {
    /// False is a server that authorises everybody and asks for nothing. None
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
}

impl Default for Rules {
    fn default() -> Self {
        Rules {
            needs_token: true,
            registers: true,
            expires_in: Some(3600),
            issue_expired: false,
            account_token: None,
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
    registrations: Arc<AtomicUsize>,
    refreshes: Arc<AtomicUsize>,
}

impl Server {
    fn base(&self) -> String {
        self.base.lock().clone()
    }

    /// Whether this token is one the server still accepts.
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
        // the way into the authorisation URL, and this is where that is proved.
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
    let method = body.get("method").and_then(|m| m.as_str()).unwrap_or_default();
    let result = match method {
        "initialize" => serde_json::json!({
            "protocolVersion": "2025-06-18",
            "capabilities": { "tools": {} },
            "serverInfo": { "name": "Scripted MCP Server" },
        }),
        "notifications/initialized" => return axum::http::StatusCode::ACCEPTED.into_response(),
        "tools/list" => serde_json::json!({
            "tools": [
                {
                    "name": "run_sql",
                    "description": "Run a query.",
                    "inputSchema": { "type": "object", "properties": { "sql": { "type": "string" } } },
                },
                // No schema at all, which is legal and means no arguments. It
                // has to reach the model as an empty object rather than as null.
                { "name": "list_projects", "description": "" },
            ],
        }),
        "tools/call" => {
            let name = body["params"]["name"].as_str().unwrap_or_default().to_string();
            let arguments = body["params"]["arguments"].clone();
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

    // Answered as an event stream rather than as JSON, because a real server on
    // the list does and parsing only JSON made a working server look broken.
    (
        [("content-type", "text/event-stream")],
        format!(
            "event: message\ndata: {}\n\n",
            serde_json::json!({ "jsonrpc": "2.0", "id": id, "result": result })
        ),
    )
        .into_response()
}

/// Plays the browser: visits the authorisation page, then calls the app back.
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
    // An operator sent to authorise a server that authorises everybody is a
    // consent prompt for nothing, and the row would claim a sign-in that never
    // happened. No vendor on the list is public today; this is the behaviour if
    // one becomes it, and the live test is what would say so.
    let server = serve(Rules { needs_token: false, ..Default::default() }).await;
    let (_dir, store, group, _agent) = workspace();

    let plugin = plugins::connect(
        &store,
        group,
        PluginKind::Neon,
        &format!("{}/mcp", server.base()),
        None,
        |_| panic!("a public server must not open a browser"),
    )
    .await
    .expect("a public server connects");

    assert!(!plugin.signed_in, "nothing was authorised, so nothing may claim to have been");
    let named: Vec<&str> = plugin.tools.iter().map(|tool| tool.name.as_str()).collect();
    assert_eq!(named, vec!["run_sql", "list_projects"]);
    assert!(plugin.tools.iter().all(|tool| tool.allowed), "a plugin arrives with nothing off");
    assert_eq!(server.registrations.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn a_protected_server_signs_in_and_the_grant_stays_in_the_store() {
    let server = serve(Rules::default()).await;
    let (_dir, store, group, _agent) = workspace();

    let plugin = plugins::connect(
        &store,
        group,
        PluginKind::Neon,
        &format!("{}/mcp", server.base()),
        None,
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
    assert!(!json.contains("access-0"), "a grant must not be serialisable to the webview: {json}");
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
        PluginKind::Neon,
        &format!("{}/mcp", server.base()),
        None,
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
        PluginKind::Neon,
        &format!("{}/mcp", server.base()),
        None,
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
        PluginKind::Neon,
        &format!("{}/mcp", server.base()),
        None,
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

    plugins::connect(&store, group, PluginKind::Neon, &endpoint, None, browser(Outcome::Allowed))
        .await
        .unwrap();

    let answer = plugins::call(
        &store,
        plugins::Target {
            group,
            agent,
            kind: PluginKind::Neon,
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
            PluginKind::Google,
            &format!("{}/mcp", server.base()),
            Some(plugins::AccountUse { token: ACCOUNT, connection: "" }),
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
            PluginKind::Google,
            &endpoint,
            Some(plugins::AccountUse { token: ACCOUNT, connection: "" }),
            |_| Ok(()),
        )
        .await
        .unwrap();

        match store.plugin_reach(group, agent, PluginKind::Google, "gmail_search").unwrap() {
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
            PluginKind::Google,
            &format!("{}/mcp", server.base()),
            None,
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
            PluginKind::Google,
            &endpoint,
            Some(plugins::AccountUse { token: ACCOUNT, connection: "" }),
            |_| Ok(()),
        )
        .await
        .unwrap();

        let answer = plugins::call(
            &store,
            plugins::Target {
                group,
                agent,
                kind: PluginKind::Google,
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
            PluginKind::Google,
            &endpoint,
            Some(plugins::AccountUse { token: ACCOUNT, connection: "" }),
            |_| Ok(()),
        )
        .await
        .unwrap();

        let failed = plugins::call(
            &store,
            plugins::Target {
                group,
                agent,
                kind: PluginKind::Google,
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
            PluginKind::Google,
            &endpoint,
            Some(plugins::AccountUse { token: ACCOUNT, connection: "" }),
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
                kind: PluginKind::Google,
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
/// session id one side invents. It reaches the network, authorises nothing and
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
        PluginKind::Google,
        &endpoint,
        Some(plugins::AccountUse { token: &token, connection: "" }),
        |_| panic!("an account-backed plugin must not open a browser"),
    )
    .await
    .expect("the account token should connect");

    // A grant with nothing authorised offers nothing, which is a real state and
    // not a failure: it means the operator has not authorised Google yet.
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
                kind: PluginKind::Google,
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

#[tokio::test]
async fn a_call_on_an_unconnected_plugin_says_who_can_connect_it() {
    let (_dir, store, group, agent) = workspace();

    let failed = plugins::call(
        &store,
        plugins::Target {
            group,
            agent,
            kind: PluginKind::Neon,
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

    plugins::connect(&store, group, PluginKind::Neon, &endpoint, None, browser(Outcome::Allowed))
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
            kind: PluginKind::Neon,
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

    plugins::connect(&store, group, PluginKind::Neon, &endpoint, None, browser(Outcome::Allowed))
        .await
        .unwrap();

    server.revoked.lock().push("access-0".to_string());

    let answer = plugins::call(
        &store,
        plugins::Target {
            group,
            agent,
            kind: PluginKind::Neon,
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
        store.plugin_reach(group, agent, PluginKind::Neon, "run_sql").unwrap()
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
        PluginKind::Neon,
        &endpoint,
        None,
        browser(Outcome::Allowed),
    )
    .await
    .unwrap();

    assert!(store.delete_plugin(plugin.id).unwrap());
    assert!(store.group_plugins(group).unwrap().is_empty());
    assert!(matches!(
        store.plugin_reach(group, agent, PluginKind::Neon, "run_sql").unwrap(),
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

    plugins::connect(&store, group, PluginKind::Neon, &endpoint, None, browser(Outcome::Allowed))
        .await
        .unwrap();
    plugins::connect(&store, group, PluginKind::Neon, &endpoint, None, browser(Outcome::Allowed))
        .await
        .unwrap();

    assert_eq!(store.group_plugins(group).unwrap().len(), 1);
    let PluginReach::Granted { grant, .. } =
        store.plugin_reach(group, agent, PluginKind::Neon, "run_sql").unwrap()
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

    plugins::connect(&store, group, PluginKind::Neon, &endpoint, None, browser(Outcome::Allowed))
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
    match tools::parse(&call).expect("a plugin tool parses") {
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
    h.runtime.plugins_at(HashMap::from([(PluginKind::Neon, endpoint.clone())]));

    let group = h.runtime.store().get_agent(h.id("Manager")).unwrap().unwrap().group_id;
    plugins::connect(
        h.runtime.store(),
        group,
        PluginKind::Neon,
        &endpoint,
        None,
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
    h.runtime.plugins_at(HashMap::from([(PluginKind::Neon, endpoint.clone())]));

    let group = h.runtime.store().get_agent(h.id("Revenue")).unwrap().unwrap().group_id;
    let plugin = plugins::connect(
        h.runtime.store(),
        group,
        PluginKind::Neon,
        &endpoint,
        None,
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
    assert!(server.called.lock().is_empty(), "an unauthorised call must not reach the vendor");
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
    h.runtime.plugins_at(HashMap::from([(PluginKind::Neon, endpoint.clone())]));

    let group = h.runtime.store().get_agent(h.id("Manager")).unwrap().unwrap().group_id;
    let plugin = plugins::connect(
        h.runtime.store(),
        group,
        PluginKind::Neon,
        &endpoint,
        None,
        browser(Outcome::Allowed),
    )
    .await
    .unwrap();
    h.runtime.store().set_plugin_tool(plugin.id, "run_sql", false).unwrap();

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
    h.runtime.plugins_at(HashMap::from([(PluginKind::Neon, endpoint.clone())]));

    let group = h.runtime.store().get_agent(h.id("Revenue")).unwrap().unwrap().group_id;
    let plugin = plugins::connect(
        h.runtime.store(),
        group,
        PluginKind::Neon,
        &endpoint,
        None,
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
    h.runtime.plugins_at(HashMap::from([(PluginKind::Neon, endpoint.clone())]));

    let group = h.runtime.store().get_agent(h.id("Revenue")).unwrap().unwrap().group_id;
    let plugin = plugins::connect(
        h.runtime.store(),
        group,
        PluginKind::Neon,
        &endpoint,
        None,
        browser(Outcome::Allowed),
    )
    .await
    .unwrap();
    h.runtime
        .store()
        .set_plugin_access(plugin.id, &PluginAccess::Chosen { agents: vec![h.id("Revenue")] })
        .unwrap();
    for tool in ["run_sql", "list_projects"] {
        h.runtime.store().set_plugin_tool(plugin.id, tool, false).unwrap();
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

    plugins::connect(&store, group, PluginKind::Neon, &endpoint, None, browser(Outcome::Allowed))
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
            kind: PluginKind::Neon,
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
            kind: PluginKind::Neon,
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
/// authorisation looks like, and the failure worth catching is that belief
/// going stale: a vendor can move an endpoint, stop offering dynamic client
/// registration, or start requiring a token on a server that used to be open,
/// and every offline test here keeps passing while no operator can connect.
///
/// Discovery is `oauth::discover`, the same call a sign-in makes, rather than
/// metadata URLs rebuilt beside it. A test with its own copy of RFC 8414 passes
/// on a vendor this build cannot reach: Stripe's authorisation server is
/// `https://access.stripe.com/mcp`, and the well-known segment goes before that
/// path, not after it.
///
/// Run with `./scripts/plugins.sh`. It reaches the real internet and spends
/// nothing: no account is authorised and no tool is called.
#[tokio::test]
#[ignore = "reaches the real vendors; run ./scripts/plugins.sh"]
async fn every_server_on_the_list_still_publishes_what_this_build_expects() {
    for kind in PluginKind::ALL {
        let endpoint = kind.endpoint();

        // 1. An unauthenticated open, which is how Guaca finds out whether this
        //    server wants a grant at all, and where the address of the sign-in
        //    comes from when it does.
        let opened = guac_lib::mcp::open(endpoint, None).await;
        let challenge = match &opened {
            Ok(session) => {
                let tools = guac_lib::mcp::list_tools(session).await.unwrap_or_else(|err| {
                    panic!("{} authorised us and then refused a tool list: {err}", kind.label())
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
        //    *authorisation server* lists, and four of those are ones a
        //    registered client may not have. Discovery is not the only thing
        //    that goes stale, and `invalid_scope` arrives in the operator's
        //    browser where no error message can reach it.
        let asked = found.requested_scope();
        for scope in asked.iter().flat_map(|s| s.split(' ')) {
            let published = if found.resource_scopes.is_empty() {
                found.server.scopes_supported.iter().any(|s| s == scope)
            } else {
                // `offline_access` is the one this build adds, and only ever on
                // the authorisation server's say-so.
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
