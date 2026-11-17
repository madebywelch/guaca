mod harness;

use std::collections::{BTreeSet, HashMap};
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
use guac_lib::domain::plugin::{HeaderPair, Headers, PluginAccess, PluginKind, SigninNeed};
use guac_lib::llm::tools::{self, ToolInvocation};
use guac_lib::mcp::PROTOCOL_VERSION;
use guac_lib::oauth::Landing;
use guac_lib::plugins;

const NOBODY: PluginAccess = PluginAccess::Chosen { agents: Vec::new() };

#[derive(Debug, Clone)]
struct Rules {
    needs_token: bool,
    registers: bool,
    expires_in: Option<i64>,
    issue_expired: bool,
    rotating: bool,
    refresh_pause: Option<Arc<tokio::sync::Semaphore>>,
    refresh_status: Option<Arc<AtomicUsize>>,
    account_token: Option<String>,
    era: Era,
    versions: Vec<String>,
    mirrors: bool,
    gated: Option<(String, String)>,
}

#[derive(Debug, Clone, Copy, PartialEq)]
enum Era {
    Legacy,
    Modern,
}

impl Default for Rules {
    fn default() -> Self {
        Rules {
            needs_token: true,
            registers: true,
            expires_in: Some(3600),
            issue_expired: false,
            rotating: false,
            refresh_pause: None,
            refresh_status: None,
            account_token: None,
            era: Era::Legacy,
            versions: Vec::new(),
            mirrors: false,
            gated: None,
        }
    }
}

#[derive(Clone)]
struct Server {
    rules: Rules,
    base: Arc<Mutex<String>>,
    issued: Arc<Mutex<Vec<String>>>,
    revoked: Arc<Mutex<Vec<String>>>,
    seen: Arc<Mutex<Vec<Option<String>>>>,
    called: Arc<Mutex<Vec<(String, serde_json::Value)>>>,
    noted: Arc<Mutex<Vec<HashMap<String, String>>>>,
    registrations: Arc<AtomicUsize>,
    refreshes: Arc<AtomicUsize>,
}

impl Server {
    fn base(&self) -> String {
        self.base.lock().clone()
    }

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
        .route("/token", post(token))
        .route("/mcp", post(rpc))
        .layer(axum::middleware::from_fn_with_state(server.clone(), gate))
        .route("/authorize", get(authorize))
        .with_state(server.clone());

    let listener = listen().await;
    let addr = listener.local_addr().unwrap();
    *server.base.lock() = format!("http://{addr}");
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    server
}

async fn listen() -> tokio::net::TcpListener {
    static USED: Mutex<BTreeSet<u16>> = Mutex::new(BTreeSet::new());
    let mut held = Vec::new();
    loop {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        if USED.lock().insert(listener.local_addr().unwrap().port()) {
            return listener;
        }
        held.push(listener);
    }
}

async fn gate(
    State(server): State<Server>,
    request: axum::extract::Request,
    next: axum::middleware::Next,
) -> axum::response::Response {
    if let Some((name, value)) = &server.rules.gated {
        let sent = request.headers().get(name.as_str()).and_then(|v| v.to_str().ok());
        if sent != Some(value.as_str()) {
            return (axum::http::StatusCode::FORBIDDEN, "no").into_response();
        }
    }
    next.run(request).await
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
    let redirect = body["redirect_uris"][0].as_str().unwrap_or_default().to_string();
    assert!(redirect.starts_with("http://127.0.0.1:"), "registered {redirect}");
    axum::Json(serde_json::json!({
        "client_id": "scripted-client",
        "client_secret": "must-not-be-sent",
        "token_endpoint_auth_method": "none",
        "redirect_uris": [redirect],
    }))
}

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
        if let Some(pause) = &server.rules.refresh_pause {
            pause.acquire().await.unwrap().forget();
        }
        if let Some(status) = &server.rules.refresh_status {
            let status = status.load(Ordering::SeqCst);
            if status != 0 {
                return (
                    axum::http::StatusCode::from_u16(status as u16).unwrap(),
                    r#"{"error":"invalid_grant","detail":"private-value"}"#,
                )
                    .into_response();
            }
        }
        if server.rules.rotating {
            let expected = format!("refresh-{}", server.issued.lock().len() - 1);
            if form.get("refresh_token") != Some(&expected) {
                return (axum::http::StatusCode::BAD_REQUEST, "invalid_grant").into_response();
            }
        }
    } else {
        assert!(form.contains_key("code_verifier"), "the exchange has to prove the PKCE challenge");
        assert!(form.contains_key("resource"), "the exchange has to name the resource");
    }

    let serial = server.issued.lock().len();
    let access = format!("access-{serial}");
    server.issued.lock().push(access.clone());

    let mut issued = serde_json::json!({
        "access_token": access,
        "refresh_token": if server.rules.rotating { format!("refresh-{serial}") } else { "refresh-token".to_string() },
        "token_type": "Bearer",
    });
    let seconds = if server.rules.issue_expired && grant != "refresh_token" {
        Some(0)
    } else {
        server.rules.expires_in
    };
    if let Some(seconds) = seconds {
        issued["expires_in"] = serde_json::json!(seconds);
    }
    axum::Json(issued).into_response()
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
            if server.rules.mirrors
                && name == "deploy"
                && noted.get("mcp-param-region").map(String::as_str) != arguments["region"].as_str()
            {
                return refusal(
                    axum::http::StatusCode::BAD_REQUEST,
                    -32020,
                    "Mcp-Param-Region does not match the body".into(),
                    serde_json::Value::Null,
                );
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
            fetch(&redirect.replace("/callback", "/favicon.ico"));
            fetch(&back);
        });
        Ok(())
    }
}

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
            subscription_model: None,
            reasoning_effort: None,
            system_prompt: "be useful".to_string(),
            skills: Vec::new(),
        })
        .unwrap()
        .id
}

#[tokio::test]
async fn a_server_that_asks_for_nothing_connects_without_sending_anybody_to_a_browser() {
    let server = serve(Rules { needs_token: false, ..Default::default() }).await;
    let (_dir, store, group, _agent) = workspace();

    let plugin = plugins::connect(
        &store,
        group,
        &PluginKind::Neon,
        &format!("{}/mcp", server.base()),
        plugins::Credential::Discover,
        &Headers::none(),
        &Landing::Loopback,
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
        &Headers::none(),
        &Landing::Loopback,
        browser(Outcome::Allowed),
    )
    .await
    .expect("the whole dance completes");

    assert!(plugin.signed_in);
    assert_eq!(server.registrations.load(Ordering::SeqCst), 1, "registered once, not per leg");

    let held = store.group_plugins(group).unwrap();
    let json = serde_json::to_string(&held).unwrap();
    assert!(!json.contains("access-0"), "a grant must not be serializable to the webview: {json}");
    assert!(!json.contains("refresh-token"));
    assert!(!json.contains("must-not-be-sent"));
}

#[tokio::test]
async fn a_wrong_state_is_rejected_before_a_valid_plugin_callback_completes() {
    let server = serve(Rules::default()).await;
    let (_dir, store, group, _agent) = workspace();
    let issued = server.issued.clone();
    let (done, browser_done) = tokio::sync::oneshot::channel();
    let plugin = plugins::connect(
        &store,
        group,
        &PluginKind::Neon,
        &format!("{}/mcp", server.base()),
        plugins::Credential::Discover,
        &Headers::none(),
        &Landing::Loopback,
        move |url| {
            let url = url.to_owned();
            tokio::spawn(async move {
                let client = reqwest::Client::new();
                let parsed = reqwest::Url::parse(&url).unwrap();
                let fields: HashMap<_, _> = parsed.query_pairs().into_owned().collect();
                let redirect = &fields["redirect_uri"];
                let state = &fields["state"];
                assert!(client.get(&url).send().await.unwrap().status().is_success());
                let wrong = match Outcome::WrongState {
                    Outcome::WrongState => "somebody-elses",
                    _ => unreachable!(),
                };
                let rejected = client
                    .get(redirect)
                    .query(&[("code", "forged-code"), ("state", wrong)])
                    .send()
                    .await
                    .unwrap();
                assert_eq!(rejected.status(), reqwest::StatusCode::BAD_REQUEST);
                assert!(issued.lock().is_empty(), "the forged code must not be exchanged");
                let accepted = client
                    .get(redirect)
                    .query(&[("code", "the-code"), ("state", state.as_str())])
                    .send()
                    .await
                    .unwrap();
                assert_eq!(accepted.status(), reqwest::StatusCode::OK);
                done.send(()).unwrap();
            });
            Ok(())
        },
    )
    .await
    .expect("the valid callback connects");
    browser_done.await.unwrap();
    assert!(plugin.signed_in);
    assert_eq!(server.issued.lock().len(), 1);
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
        &Headers::none(),
        &Landing::Loopback,
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
    let server = serve(Rules { registers: false, ..Default::default() }).await;
    let (_dir, store, group, _agent) = workspace();

    let failed = plugins::connect(
        &store,
        group,
        &PluginKind::Neon,
        &format!("{}/mcp", server.base()),
        plugins::Credential::Discover,
        &Headers::none(),
        &Landing::Loopback,
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
        &Headers::none(),
        &Landing::Loopback,
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
            account: plugins::Held::Absent,
        },
        "run_sql",
        &serde_json::json!({ "sql": "select 1" }),
    )
    .await
    .expect("the call goes through");

    assert_eq!(answer.text, "run_sql ran");
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
            &Headers::none(),
            &Landing::Loopback,
            |_| panic!("an account-backed plugin must not send anyone to a browser"),
        )
        .await
        .expect("the account token is the sign-in");

        assert_eq!(plugin.kind, PluginKind::Google);
        assert!(!plugin.tools.is_empty(), "the tool list is read the same way as any other");
        assert_eq!(server.registrations.load(Ordering::SeqCst), 0);
        assert!(
            server.seen.lock().iter().flatten().any(|token| token == ACCOUNT),
            "the account token has to reach the server or nothing is authenticated"
        );
    }

    #[tokio::test]
    async fn moving_a_crew_to_another_google_keeps_who_may_call_what() {
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
            &Headers::none(),
            &Landing::Loopback,
            |_| Ok(()),
        )
        .await
        .unwrap();
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
            &Headers::none(),
            &Landing::Loopback,
            |_| Ok(()),
        )
        .await
        .unwrap();

        assert_eq!(moved.id, plugin.id, "the row moves rather than being replaced");
        let mine = moved.tools.iter().find(|t| t.name == "run_sql").expect("still published");
        assert_eq!(mine.access, PluginAccess::Chosen { agents: vec![agent] });
        let off = moved.tools.iter().find(|t| t.name == "list_projects").expect("still published");
        assert_eq!(off.access, NOBODY);
        assert!(matches!(
            store.plugin_reach(group, agent, &PluginKind::Google, "list_projects").unwrap(),
            PluginReach::ToolDenied
        ));
    }

    #[tokio::test]
    async fn it_stores_no_grant_of_its_own() {
        let server = account_server().await;
        let (_dir, store, group, agent) = workspace();
        let endpoint = format!("{}/mcp", server.base());

        plugins::connect(
            &store,
            group,
            &PluginKind::Google,
            &endpoint,
            plugins::Credential::Account(plugins::AccountUse { token: ACCOUNT, connection: "" }),
            &Headers::none(),
            &Landing::Loopback,
            |_| Ok(()),
        )
        .await
        .unwrap();

        match store.plugin_reach(group, agent, &PluginKind::Google, "run_sql").unwrap() {
            guac_lib::db::store::PluginReach::Granted(reached) => {
                assert!(
                    reached.grant.is_none(),
                    "an account-backed plugin holds no grant of its own"
                );
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
            &Headers::none(),
            &Landing::Loopback,
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
            &Headers::none(),
            &Landing::Loopback,
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
                account: plugins::Held::Token(plugins::AccountUse {
                    token: ACCOUNT,
                    connection: "",
                }),
            },
            "run_sql",
            &serde_json::json!({ "sql": "select 1" }),
        )
        .await
        .expect("the call goes through on the account's token");

        assert_eq!(answer.text, "run_sql ran");
    }

    #[tokio::test]
    async fn a_call_after_signing_out_says_so_rather_than_failing_on_the_wire() {
        let server = account_server().await;
        let (_dir, store, group, agent) = workspace();
        let endpoint = format!("{}/mcp", server.base());

        plugins::connect(
            &store,
            group,
            &PluginKind::Google,
            &endpoint,
            plugins::Credential::Account(plugins::AccountUse { token: ACCOUNT, connection: "" }),
            &Headers::none(),
            &Landing::Loopback,
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
                account: plugins::Held::Absent,
            },
            "run_sql",
            &serde_json::json!({}),
        )
        .await
        .expect_err("there is no account token any more");

        assert!(failed.to_string().contains("Guaca account"), "{failed}");
    }

    #[tokio::test]
    async fn an_account_that_could_not_renew_is_not_reported_as_no_account() {
        let server = account_server().await;
        let (_dir, store, group, agent) = workspace();
        let endpoint = format!("{}/mcp", server.base());

        plugins::connect(
            &store,
            group,
            &PluginKind::Google,
            &endpoint,
            plugins::Credential::Account(plugins::AccountUse { token: ACCOUNT, connection: "" }),
            &Headers::none(),
            &guac_lib::oauth::Landing::Loopback,
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
                account: plugins::Held::Refused(
                    "https://guaca.bot answered HTTP 400: invalid_grant".to_string(),
                ),
            },
            "run_sql",
            &serde_json::json!({}),
        )
        .await
        .expect_err("the account had nothing to spend");

        let said = failed.to_string();
        assert!(said.contains("HTTP 400"), "{said}");
        assert!(said.contains("signed in"), "{said}");
        assert!(!said.contains("not signed in"), "the machine is signed in: {said}");
        assert!(!said.contains("Settings"), "there is nothing to do in Settings: {said}");
    }

    #[tokio::test]
    async fn an_agent_the_operator_did_not_choose_still_cannot_call_it() {
        let server = account_server().await;
        let (_dir, store, group, agent) = workspace();
        let endpoint = format!("{}/mcp", server.base());

        let plugin = plugins::connect(
            &store,
            group,
            &PluginKind::Google,
            &endpoint,
            plugins::Credential::Account(plugins::AccountUse { token: ACCOUNT, connection: "" }),
            &Headers::none(),
            &Landing::Loopback,
            |_| Ok(()),
        )
        .await
        .unwrap();

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
                account: plugins::Held::Token(plugins::AccountUse {
                    token: ACCOUNT,
                    connection: "",
                }),
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
        &Headers::none(),
        &Landing::Loopback,
        |_| panic!("an account-backed plugin must not open a browser"),
    )
    .await
    .expect("the account token should connect");

    let offered: Vec<&str> = plugin.tools.iter().map(|card| card.name.as_str()).collect();
    println!("connected with {} tool(s): {offered:?}", offered.len());

    if let Some(tool) = offered.first().copied() {
        let called = plugins::call(
            &store,
            plugins::Target {
                group,
                agent,
                kind: &PluginKind::Google,
                endpoint: &endpoint,
                account: plugins::Held::Token(plugins::AccountUse {
                    token: &token,
                    connection: "",
                }),
            },
            tool,
            &serde_json::json!({}),
        )
        .await;
        match called {
            Ok(answer) => {
                println!("{tool} answered: {}", answer.text.chars().take(200).collect::<String>())
            }
            Err(guac_lib::plugins::PluginError::Server(guac_lib::mcp::McpError::Rejected {
                message,
            })) => println!("{tool} refused, which is an answer: {message}"),
            Err(err) => panic!("{tool} did not answer as MCP: {err}"),
        }
    }
}

mod added {
    use super::*;

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
            &Headers::none(),
            &Landing::Loopback,
            browser(Outcome::Allowed),
        )
        .await
        .expect("a server nobody vouched for signs in exactly as a vendor's does");

        assert_eq!(plugin.name, "home_assistant");
        assert_eq!(plugin.kind.slug(), "home_assistant");
        assert!(plugin.custom);
        assert!(plugin.signed_in);
        assert_eq!(plugin.endpoint, kind.endpoint());
        assert_eq!(server.registrations.load(Ordering::SeqCst), 1);

        let held = store.group_plugins(group).unwrap();
        assert_eq!(held.len(), 1);
        assert_eq!(held[0].kind, kind);

        let connected = store.plugin_tools(group, agent).unwrap();
        let specs = tools::plugin_specs(&connected);
        let names: Vec<&str> = specs.iter().map(|spec| spec.name.as_str()).collect();
        assert_eq!(names, vec!["home_assistant__run_sql", "home_assistant__list_projects"]);

        let answer = plugins::call(
            &store,
            plugins::Target {
                group,
                agent,
                kind: &kind,
                endpoint: kind.endpoint(),
                account: plugins::Held::Absent,
            },
            "run_sql",
            &serde_json::json!({ "sql": "select 1" }),
        )
        .await
        .expect("the crew's own server answers");
        assert_eq!(answer.text, "run_sql ran");
        assert_eq!(server.seen.lock().last().cloned().flatten(), Some("access-0".to_string()));
    }

    #[tokio::test]
    async fn its_name_only_resolves_against_a_crew_that_has_it() {
        let server = serve(Rules::default()).await;
        let (_dir, store, group, _agent) = workspace();
        let kind = mine(&server);

        let call = |name: &str| guac_lib::llm::openrouter::ToolCall {
            id: "1".into(),
            name: name.into(),
            arguments: "{}".into(),
        };

        let before = store.group_plugin_kinds(group).unwrap();
        assert!(tools::parse(&call("home_assistant__run_sql"), &before).is_err());
        assert!(matches!(
            tools::parse(&call("neon__run_sql"), &before),
            Ok(ToolInvocation::Plugin { .. })
        ));
        assert!(tools::parse(&call("use_screen__click"), &before).is_err());

        plugins::connect(
            &store,
            group,
            &kind,
            kind.endpoint(),
            plugins::Credential::Discover,
            &Headers::none(),
            &Landing::Loopback,
            browser(Outcome::Allowed),
        )
        .await
        .unwrap();

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
            &Headers::none(),
            &Landing::Loopback,
            browser(Outcome::Allowed),
        )
        .await
        .unwrap();
        store.set_plugin_access(plugin.id, &PluginAccess::Chosen { agents: vec![chosen] }).unwrap();

        assert!(store.plugin_tools(group, left_out).unwrap().is_empty());

        let call = guac_lib::llm::openrouter::ToolCall {
            id: "1".into(),
            name: "home_assistant__run_sql".into(),
            arguments: "{}".into(),
        };
        let named = store.group_plugin_kinds(group).unwrap();
        assert!(matches!(tools::parse(&call, &named), Ok(ToolInvocation::Plugin { .. })));

        let refused = plugins::call(
            &store,
            plugins::Target {
                group,
                agent: left_out,
                kind: &kind,
                endpoint: kind.endpoint(),
                account: plugins::Held::Absent,
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
            &Headers::none(),
            &Landing::Loopback,
            |_| panic!("a pasted key must not send anybody to a browser"),
        )
        .await
        .expect("a key connects");

        assert!(plugin.signed_in, "a key is a sign-in: the server accepted it");
        assert_eq!(server.registrations.load(Ordering::SeqCst), 0, "nothing to register with");

        let json = serde_json::to_string(&store.group_plugins(group).unwrap()).unwrap();
        assert!(!json.contains("hand-minted"), "a key must not cross IPC: {json}");

        plugins::call(
            &store,
            plugins::Target {
                group,
                agent,
                kind: &kind,
                endpoint: kind.endpoint(),
                account: plugins::Held::Absent,
            },
            "run_sql",
            &serde_json::json!({}),
        )
        .await
        .expect("the key reaches the server");
        assert_eq!(server.seen.lock().last().cloned().flatten(), Some("hand-minted".to_string()));
    }

    #[tokio::test]
    async fn a_key_the_server_stops_taking_says_to_paste_another_rather_than_retrying_forever() {
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
            &Headers::none(),
            &Landing::Loopback,
            |_| Ok(()),
        )
        .await
        .unwrap();

        server.revoked.lock().push("hand-minted".to_string());
        let refused = plugins::call(
            &store,
            plugins::Target {
                group,
                agent,
                kind: &kind,
                endpoint: kind.endpoint(),
                account: plugins::Held::Absent,
            },
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
            &Headers::none(),
            &Landing::Loopback,
            browser(Outcome::Allowed),
        )
        .await
        .unwrap();

        store
            .set_plugin_tool(plugin.id, "run_sql", &PluginAccess::Chosen { agents: vec![answerer] })
            .unwrap();

        assert!(matches!(
            store.plugin_reach(group, answerer, &kind, "run_sql").unwrap(),
            PluginReach::Granted(_)
        ));
        assert!(matches!(
            store.plugin_reach(group, triage, &kind, "run_sql").unwrap(),
            PluginReach::ToolNotChosen
        ));

        let refused = plugins::call(
            &store,
            plugins::Target {
                group,
                agent: triage,
                kind: &kind,
                endpoint: kind.endpoint(),
                account: plugins::Held::Absent,
            },
            "run_sql",
            &serde_json::json!({}),
        )
        .await
        .expect_err("a tool narrowed away is refused here, not at the server");
        assert!(refused.to_string().contains("home_assistant"), "{refused}");

        let theirs = store.plugin_tools(group, answerer).unwrap();
        let mine_now = store.plugin_tools(group, triage).unwrap();
        assert!(theirs[0].offered.iter().any(|tool| tool.name == "run_sql"));
        assert!(!mine_now[0].offered.iter().any(|tool| tool.name == "run_sql"));
        assert_eq!(mine_now[0].elsewhere, vec!["run_sql"]);
    }

    #[tokio::test]
    async fn two_of_them_in_one_crew_keep_their_own_addresses() {
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
                &Headers::none(),
                &Landing::Loopback,
                |_| Ok(()),
            )
            .await
            .unwrap();
        }

        assert_eq!(store.group_plugins(group).unwrap().len(), 2);
        let specs = tools::plugin_specs(&store.plugin_tools(group, agent).unwrap());
        assert!(specs.iter().any(|spec| spec.name == "vault__run_sql"));
        assert!(specs.iter().any(|spec| spec.name == "notes__run_sql"));

        plugins::call(
            &store,
            plugins::Target {
                group,
                agent,
                kind: &notes,
                endpoint: notes.endpoint(),
                account: plugins::Held::Absent,
            },
            "run_sql",
            &serde_json::json!({}),
        )
        .await
        .unwrap();
        assert_eq!(second.called.lock().len(), 1);
        assert!(first.called.lock().is_empty());
    }

    #[tokio::test]
    async fn a_gated_server_that_also_signs_in_gets_the_headers_on_the_whole_dance() {
        let server = serve(Rules {
            gated: Some(("cf-access-client-id".into(), "abc.access".into())),
            issue_expired: true,
            ..Rules::default()
        })
        .await;
        let (_dir, store, group, agent) = workspace();
        let kind = PluginKind::custom("Home Assistant", &format!("{}/mcp", server.base())).unwrap();
        let headers = Headers::parse(&[HeaderPair {
            name: "Cf-Access-Client-Id".into(),
            value: "abc.access".into(),
        }])
        .unwrap();

        let plugin = plugins::connect(
            &store,
            group,
            &kind,
            kind.endpoint(),
            plugins::Credential::Discover,
            &headers,
            &Landing::Loopback,
            browser(Outcome::Allowed),
        )
        .await
        .expect("past the gate, and then signed in from behind it");

        assert!(plugin.signed_in, "the 401 behind the gate is still a sign-in");
        assert_eq!(server.registrations.load(Ordering::SeqCst), 1);

        let answer = plugins::call(
            &store,
            plugins::Target {
                group,
                agent,
                kind: &kind,
                endpoint: kind.endpoint(),
                account: plugins::Held::Absent,
            },
            "run_sql",
            &serde_json::json!({ "sql": "select 1" }),
        )
        .await
        .expect("the grant was stale, so this had to renew before it could call");
        assert_eq!(answer.text, "run_sql ran");
        assert_eq!(server.refreshes.load(Ordering::SeqCst), 1);
    }
}

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
            &Headers::none(),
            &Landing::Loopback,
            |_| panic!("this server authorizes everybody"),
        )
        .await
        .expect("a modern server connects");

        let named: Vec<&str> = plugin.tools.iter().map(|tool| tool.name.as_str()).collect();
        assert_eq!(named, vec!["run_sql", "list_projects"]);
        assert_eq!(plugin.account, "Scripted MCP Server");

        plugins::call(
            &store,
            plugins::Target {
                group,
                agent,
                kind: &kind,
                endpoint: kind.endpoint(),
                account: plugins::Held::Absent,
            },
            "run_sql",
            &serde_json::json!({ "sql": "select 1" }),
        )
        .await
        .expect("a modern call goes through");

        let noted = server.noted.lock().clone();
        assert!(!noted.is_empty());
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
        let server = serve(Rules { needs_token: false, ..Default::default() }).await;
        let (_dir, store, group, _agent) = workspace();
        let kind = PluginKind::custom("legacy", &format!("{}/mcp", server.base())).unwrap();

        plugins::connect(
            &store,
            group,
            &kind,
            kind.endpoint(),
            plugins::Credential::Discover,
            &Headers::none(),
            &Landing::Loopback,
            |_| Ok(()),
        )
        .await
        .expect("a legacy server connects through the fallback");

        let noted = server.noted.lock().clone();
        assert!(noted.iter().all(|seen| !seen.contains_key("mcp-name")), "{noted:?}");
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
                &Headers::none(),
                &Landing::Loopback,
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
            &Headers::none(),
            &Landing::Loopback,
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
            &Headers::none(),
            &Landing::Loopback,
            |_| Ok(()),
        )
        .await
        .unwrap();

        let named: Vec<&str> = plugin.tools.iter().map(|tool| tool.name.as_str()).collect();
        assert_eq!(named, vec!["run_sql", "list_projects", "deploy"]);
        assert!(!named.contains(&"broken"), "an unreadable annotation drops one tool, not all");

        let answer = plugins::call(
            &store,
            plugins::Target {
                group,
                agent,
                kind: &kind,
                endpoint: kind.endpoint(),
                account: plugins::Held::Absent,
            },
            "deploy",
            &serde_json::json!({ "region": "us-west1", "what": "the worker" }),
        )
        .await
        .expect("the header the server asked for is on the request");
        assert_eq!(answer.text, "deploy ran");

        let noted = server.noted.lock().clone();
        let called = noted
            .iter()
            .find(|seen| seen.get("mcp-method").map(String::as_str) == Some("tools/call"))
            .unwrap();
        assert_eq!(called.get("mcp-param-region").map(String::as_str), Some("us-west1"));
    }
}

#[tokio::test]
async fn a_connector_run_on_the_host_is_called_with_the_environment_it_was_given() {
    let (_dir, store, group, agent) = workspace();
    let command =
        format!("stdio:python3 {}/tests/fixtures/mcp-stdio.py", env!("CARGO_MANIFEST_DIR"));
    let kind = PluginKind::custom("fixture", &command).unwrap();
    let env =
        Headers::parse_env(&[HeaderPair { name: "FIXTURE_TOKEN".into(), value: "abc".into() }])
            .unwrap();

    let plugin = plugins::connect(
        &store,
        group,
        &kind,
        kind.endpoint(),
        plugins::Credential::Discover,
        &env,
        &Landing::Loopback,
        browser(Outcome::Refused),
    )
    .await
    .expect("a program connects without a sign-in");
    assert_eq!(plugin.tools.len(), 3);

    let answer = plugins::call(
        &store,
        plugins::Target {
            group,
            agent,
            kind: &kind,
            endpoint: kind.endpoint(),
            account: plugins::Held::Absent,
        },
        "whoami",
        &serde_json::json!({}),
    )
    .await
    .expect("the call goes through");
    assert!(answer.text.starts_with("token=abc leaked=False"), "{}", answer.text);
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
            account: plugins::Held::Absent,
        },
        "run_sql",
        &serde_json::json!({}),
    )
    .await
    .expect_err("nothing is connected");

    let said = failed.to_string();
    assert!(said.contains("not connected"), "{said}");
    assert!(said.contains("operator"), "{said}");
}

#[tokio::test]
async fn parallel_stale_callers_share_one_rotating_refresh() {
    parallel_rotating_refresh(true).await;
}

#[tokio::test]
async fn parallel_unauthorized_callers_reuse_the_peer_refresh() {
    parallel_rotating_refresh(false).await;
}

async fn parallel_rotating_refresh(expired: bool) {
    let pause = Arc::new(tokio::sync::Semaphore::new(0));
    let server = serve(Rules {
        issue_expired: expired,
        rotating: true,
        refresh_pause: Some(pause.clone()),
        ..Default::default()
    })
    .await;
    let endpoint = format!("{}/mcp", server.base());
    let (_dir, store, group, agent) = workspace();
    plugins::connect(
        &store,
        group,
        &PluginKind::Neon,
        &endpoint,
        plugins::Credential::Discover,
        &Headers::none(),
        &Landing::Loopback,
        browser(Outcome::Allowed),
    )
    .await
    .unwrap();
    if !expired {
        server.revoked.lock().push("access-0".into());
    }
    let args = serde_json::json!({});
    let target = || plugins::Target {
        group,
        agent,
        kind: &PluginKind::Neon,
        endpoint: &endpoint,
        account: plugins::Held::Absent,
    };
    let release = async {
        tokio::time::timeout(std::time::Duration::from_secs(5), async {
            while server.refreshes.load(Ordering::SeqCst) == 0 {
                tokio::time::sleep(std::time::Duration::from_millis(5)).await;
            }
        })
        .await
        .unwrap();
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
        pause.add_permits(2);
    };
    let (a, b, ()) = tokio::join!(
        plugins::call(&store, target(), "run_sql", &args),
        plugins::call(&store, target(), "run_sql", &args),
        release
    );
    assert!(a.is_ok(), "{a:?}");
    assert!(b.is_ok(), "{b:?}");
    assert_eq!(server.refreshes.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn reconnect_with_identical_grant_fences_a_stalled_refresh() {
    replaced_grant_fences_stalled_refresh(false).await;
}

#[tokio::test]
async fn disconnect_and_reconnect_fences_a_stalled_refresh() {
    replaced_grant_fences_stalled_refresh(true).await;
}

async fn replaced_grant_fences_stalled_refresh(disconnect: bool) {
    let pause = Arc::new(tokio::sync::Semaphore::new(0));
    let server = serve(Rules {
        issue_expired: true,
        refresh_pause: Some(pause.clone()),
        ..Default::default()
    })
    .await;
    let endpoint = format!("{}/mcp", server.base());
    let (_dir, store, group, agent) = workspace();
    let plugin = plugins::connect(
        &store,
        group,
        &PluginKind::Neon,
        &endpoint,
        plugins::Credential::Discover,
        &Headers::none(),
        &Landing::Loopback,
        browser(Outcome::Allowed),
    )
    .await
    .unwrap();
    let original = store.plugin_dial(plugin.id).unwrap().unwrap().grant.unwrap();
    let discovered = store
        .plugin_tools(group, agent)
        .unwrap()
        .into_iter()
        .find(|set| set.kind == PluginKind::Neon)
        .unwrap()
        .offered;
    assert!(discovered.iter().any(|tool| tool.name == "run_sql"));
    let args = serde_json::json!({});
    let call = plugins::call(
        &store,
        plugins::Target {
            group,
            agent,
            kind: &PluginKind::Neon,
            endpoint: &endpoint,
            account: plugins::Held::Absent,
        },
        "run_sql",
        &args,
    );
    let reconnect = async {
        tokio::time::timeout(std::time::Duration::from_secs(5), async {
            while server.refreshes.load(Ordering::SeqCst) == 0 {
                tokio::time::sleep(std::time::Duration::from_millis(5)).await;
            }
        })
        .await
        .unwrap();
        if disconnect {
            assert!(store.delete_plugin(plugin.id).unwrap());
        }
        let replacement = store
            .save_plugin(
                group,
                &PluginKind::Neon,
                "",
                &discovered,
                Some(&original),
                "",
                &Headers::none(),
            )
            .unwrap();
        pause.add_permits(1);
        replacement.id
    };
    let (result, replacement) = tokio::join!(call, reconnect);
    assert!(result.is_err());
    assert_eq!(store.plugin_dial(replacement).unwrap().unwrap().grant.unwrap(), original);
}

#[tokio::test]
async fn failed_refresh_preserves_grant_for_later_recovery() {
    for status in [400, 503] {
        let failure = Arc::new(AtomicUsize::new(status));
        let server = serve(Rules {
            issue_expired: true,
            refresh_status: Some(failure.clone()),
            ..Default::default()
        })
        .await;
        let endpoint = format!("{}/mcp", server.base());
        let (_dir, store, group, agent) = workspace();
        let plugin = plugins::connect(
            &store,
            group,
            &PluginKind::Neon,
            &endpoint,
            plugins::Credential::Discover,
            &Headers::none(),
            &Landing::Loopback,
            browser(Outcome::Allowed),
        )
        .await
        .unwrap();
        let original = store.plugin_dial(plugin.id).unwrap().unwrap().grant.unwrap();
        let args = serde_json::json!({});
        let target = || plugins::Target {
            group,
            agent,
            kind: &PluginKind::Neon,
            endpoint: &endpoint,
            account: plugins::Held::Absent,
        };
        let error =
            plugins::call(&store, target(), "run_sql", &args).await.unwrap_err().to_string();
        if status == 503 {
            assert!(error.contains("could not confirm whether its sign-in was renewed"), "{error}");
            assert!(!error.contains("Try again later"), "{error}");
        } else {
            assert!(error.contains("connect it again"), "{error}");
        }
        assert!(!error.contains("private-value"), "{error}");
        assert_eq!(store.plugin_dial(plugin.id).unwrap().unwrap().grant.unwrap(), original);
        failure.store(0, Ordering::SeqCst);
        assert!(plugins::call(&store, target(), "run_sql", &args).await.is_ok());
        assert_eq!(server.refreshes.load(Ordering::SeqCst), 2);
    }
}

#[tokio::test]
async fn a_stale_grant_is_renewed_before_it_is_spent() {
    let server = serve(Rules { issue_expired: true, ..Default::default() }).await;
    let endpoint = format!("{}/mcp", server.base());
    let (_dir, store, group, agent) = workspace();

    plugins::connect(
        &store,
        group,
        &PluginKind::Neon,
        &endpoint,
        plugins::Credential::Discover,
        &Headers::none(),
        &Landing::Loopback,
        browser(Outcome::Allowed),
    )
    .await
    .unwrap();

    let after_connecting = server.seen.lock().len();

    plugins::call(
        &store,
        plugins::Target {
            group,
            agent,
            kind: &PluginKind::Neon,
            endpoint: &endpoint,
            account: plugins::Held::Absent,
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
    let server = serve(Rules::default()).await;
    let endpoint = format!("{}/mcp", server.base());
    let (_dir, store, group, agent) = workspace();

    plugins::connect(
        &store,
        group,
        &PluginKind::Neon,
        &endpoint,
        plugins::Credential::Discover,
        &Headers::none(),
        &Landing::Loopback,
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
            account: plugins::Held::Absent,
        },
        "run_sql",
        &serde_json::json!({}),
    )
    .await
    .expect("one refresh and one retry is enough");

    assert_eq!(answer.text, "run_sql ran");
    assert_eq!(server.refreshes.load(Ordering::SeqCst), 1, "exactly one renewal, not a loop");

    let PluginReach::Granted(reached) =
        store.plugin_reach(group, agent, &PluginKind::Neon, "run_sql").unwrap()
    else {
        panic!("the plugin is connected and this agent was not excluded from it")
    };
    assert_eq!(reached.grant.unwrap().access_token, "access-1");
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
        &Headers::none(),
        &Landing::Loopback,
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
    let server = serve(Rules::default()).await;
    let endpoint = format!("{}/mcp", server.base());
    let (_dir, store, group, agent) = workspace();

    plugins::connect(
        &store,
        group,
        &PluginKind::Neon,
        &endpoint,
        plugins::Credential::Discover,
        &Headers::none(),
        &Landing::Loopback,
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
        &Headers::none(),
        &Landing::Loopback,
        browser(Outcome::Allowed),
    )
    .await
    .unwrap();

    assert_eq!(store.group_plugins(group).unwrap().len(), 1);
    let PluginReach::Granted(reached) =
        store.plugin_reach(group, agent, &PluginKind::Neon, "run_sql").unwrap()
    else {
        panic!("connecting again leaves a plugin the crew can use")
    };
    assert_eq!(
        reached.grant.unwrap().access_token,
        "access-1",
        "the newer sign-in is the one held"
    );
}

#[tokio::test]
async fn what_the_crew_connected_is_what_the_model_is_offered() {
    let server = serve(Rules::default()).await;
    let endpoint = format!("{}/mcp", server.base());
    let (_dir, store, group, agent) = workspace();

    plugins::connect(
        &store,
        group,
        &PluginKind::Neon,
        &endpoint,
        plugins::Credential::Discover,
        &Headers::none(),
        &Landing::Loopback,
        browser(Outcome::Allowed),
    )
    .await
    .unwrap();

    let connected = store.plugin_tools(group, agent).unwrap();
    let specs = tools::plugin_specs(&connected);
    let names: Vec<&str> = specs.iter().map(|spec| spec.name.as_str()).collect();
    assert_eq!(names, vec!["neon__run_sql", "neon__list_projects"]);

    let bare = specs.iter().find(|spec| spec.name == "neon__list_projects").unwrap();
    assert_eq!(bare.parameters["type"], serde_json::json!("object"));
    assert!(
        bare.description.contains("Neon"),
        "a description has to say where the tool reaches: {}",
        bare.description
    );

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
        &Headers::none(),
        &Landing::Loopback,
        browser(Outcome::Allowed),
    )
    .await
    .unwrap();

    let run = h.runtime.send_from_human(h.id("Manager"), "Check the database.").unwrap();
    h.settle(run).await;

    let offered: Vec<String> = model.transcript.lock()[0]["tools"]
        .as_array()
        .expect("a turn carries tool definitions")
        .iter()
        .map(|tool| tool["function"]["name"].as_str().unwrap_or_default().to_string())
        .collect();
    assert!(offered.contains(&"neon__run_sql".to_string()), "offered {offered:?}");
    assert!(offered.contains(&"send_message".to_string()), "offered {offered:?}");

    assert_eq!(
        server.called.lock().first().cloned(),
        Some(("run_sql".to_string(), serde_json::json!({ "sql": "select 1" }))),
    );

    let results = harness::tool_results(&model);
    assert!(results.iter().any(|r| r.contains("run_sql ran")), "got {results:?}");

    h.expect_normal(run, "a turn that calls a plugin");
}

#[tokio::test]
async fn a_kept_page_reads_what_the_operator_allowed_with_no_model_in_the_loop() {
    let server = serve(Rules::default()).await;
    let endpoint = format!("{}/mcp", server.base());

    let model = harness::serve(|body| {
        if harness::has_tool_result(body) {
            harness::Script::Say("The board reads the database.".into())
        } else {
            harness::Script::Plugin {
                name: "artifact".into(),
                arguments: serde_json::json!({
                    "action": "create",
                    "title": "Rows",
                    "page": "<p>rows</p>",
                    "sources": [{
                        "name": "rows",
                        "tool": "neon__run_sql",
                        "arguments": { "sql": "select 1" }
                    }],
                }),
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
        &Headers::none(),
        &Landing::Loopback,
        browser(Outcome::Allowed),
    )
    .await
    .unwrap();

    let run = h.runtime.send_from_human(h.id("Manager"), "Make me a board of rows.").unwrap();
    h.settle(run).await;

    let page = h.runtime.store().artifacts(Some(group), 10).unwrap().pop().expect("a page kept");
    assert_eq!(page.sources.len(), 1);
    assert!(!page.sources_allowed, "declared is not allowed");
    let results = harness::tool_results(&model).join("\n");
    assert!(results.contains("wait for the operator to allow them"), "got {results:?}");

    let refused = h.runtime.read_artifact(page.id, None).await.unwrap().unwrap();
    assert!(
        refused[0].error.as_deref().is_some_and(|why| why.contains("not allowed")),
        "{refused:?}"
    );
    assert!(server.called.lock().is_empty(), "nothing is read before the operator allows it");

    h.runtime.store().allow_artifact_sources(page.id).unwrap();
    let asked = model.transcript.lock().len();
    let read = h.runtime.read_artifact(page.id, None).await.unwrap().unwrap();
    assert_eq!(read[0].name, "rows");
    assert_eq!(read[0].text.as_deref(), Some("run_sql ran"), "{read:?}");
    assert_eq!(read[0].error, None);
    assert_eq!(
        server.called.lock().first().cloned(),
        Some(("run_sql".to_string(), serde_json::json!({ "sql": "select 1" }))),
        "exactly the arguments the operator allowed"
    );
    assert_eq!(model.transcript.lock().len(), asked, "and no model was asked anything");
}

#[tokio::test]
async fn a_page_cannot_declare_a_read_its_owner_could_not_make() {
    let model = harness::serve(|body| {
        if harness::has_tool_result(body) {
            harness::Script::Say("It cannot read that.".into())
        } else {
            harness::Script::Plugin {
                name: "artifact".into(),
                arguments: serde_json::json!({
                    "action": "create",
                    "title": "Rows",
                    "page": "<p>rows</p>",
                    "sources": [{ "name": "rows", "tool": "neon__run_sql" }],
                }),
            }
        }
    })
    .await;

    let h =
        harness::harness(&model, &["Manager"], guac_lib::runtime::guard::GuardLimits::default());
    let run = h.runtime.send_from_human(h.id("Manager"), "Make me a board of rows.").unwrap();
    h.settle(run).await;

    let results = harness::tool_results(&model).join("\n");
    assert!(results.contains("cannot call `neon__run_sql`"), "got {results:?}");
    assert!(h.runtime.store().artifacts(None, 10).unwrap().is_empty(), "and nothing was kept");
}

#[tokio::test]
async fn an_agent_calling_a_plugin_its_crew_has_not_connected_is_told_who_can() {
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
    assert_eq!(h.channel_texts("Manager").len(), 2, "the operator is answered either way");
}

#[tokio::test]
async fn an_agent_the_plugin_was_narrowed_away_from_is_neither_told_nor_allowed() {
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
        &Headers::none(),
        &Landing::Loopback,
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

    let offered: Vec<String> = model.transcript.lock()[0]["tools"]
        .as_array()
        .expect("a turn carries tool definitions")
        .iter()
        .map(|tool| tool["function"]["name"].as_str().unwrap_or_default().to_string())
        .collect();
    assert!(!offered.iter().any(|name| name.starts_with("neon__")), "offered {offered:?}");
    assert!(offered.contains(&"send_message".to_string()), "the app's own tools stay");

    let prompt = harness::prompts_by_agent(&model).remove("Scribe").expect("Scribe had a turn");
    assert!(!prompt.contains("these connectors"), "{prompt}");

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
        &Headers::none(),
        &Landing::Loopback,
        browser(Outcome::Allowed),
    )
    .await
    .unwrap();
    h.runtime.store().set_plugin_tool(plugin.id, "run_sql", &NOBODY).unwrap();

    let run = h.runtime.send_from_human(h.id("Manager"), "Check the database.").unwrap();
    h.settle(run).await;

    let offered: Vec<String> = model.transcript.lock()[0]["tools"]
        .as_array()
        .expect("a turn carries tool definitions")
        .iter()
        .map(|tool| tool["function"]["name"].as_str().unwrap_or_default().to_string())
        .collect();
    assert!(!offered.contains(&"neon__run_sql".to_string()), "offered {offered:?}");
    assert!(offered.contains(&"neon__list_projects".to_string()), "offered {offered:?}");

    let prompt = harness::prompts_by_agent(&model).remove("Manager").expect("Manager had a turn");
    assert!(prompt.contains("Switched off by the operator"), "{prompt}");
    assert!(prompt.contains("run_sql"), "{prompt}");

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
        &Headers::none(),
        &Landing::Loopback,
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
    assert!(scribe.contains("the Neon connector"), "{scribe}");

    let run = h.runtime.send_from_human(h.id("Revenue"), "Anything to report?").unwrap();
    h.settle(run).await;
    let revenue = harness::prompts_by_agent(&model).remove("Revenue").expect("Revenue had a turn");
    assert!(!revenue.contains("the Neon connector"), "{revenue}");
}

#[tokio::test]
async fn two_agents_on_one_plugin_are_offered_different_tools_and_told_whose_is_whose() {
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
        &Headers::none(),
        &Landing::Loopback,
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

    let offered: Vec<String> = model.transcript.lock()[0]["tools"]
        .as_array()
        .expect("a turn carries tool definitions")
        .iter()
        .map(|tool| tool["function"]["name"].as_str().unwrap_or_default().to_string())
        .collect();
    assert!(offered.contains(&"neon__list_projects".to_string()), "offered {offered:?}");
    assert!(!offered.contains(&"neon__run_sql".to_string()), "offered {offered:?}");
    assert!(!server.called.lock().is_empty(), "the half it was given still reaches the vendor");

    let prompt = harness::prompts_by_agent(&model).remove("Reader").expect("Reader had a turn");
    assert!(prompt.contains("Someone else's on this connector"), "{prompt}");
    assert!(prompt.contains("run_sql"), "{prompt}");
    assert!(!prompt.contains("Switched off by the operator"), "{prompt}");
    assert!(prompt.contains("the Neon connector's run_sql"), "{prompt}");
}

#[tokio::test]
async fn a_tool_that_is_a_peer_s_is_refused_here_and_not_at_the_vendor() {
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
        &Headers::none(),
        &Landing::Loopback,
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
    assert!(!results.iter().any(|r| r.contains("Do not ask a peer")), "got {results:?}");
    assert!(server.called.lock().is_empty(), "a tool that is a peer's must not reach the vendor");
    assert_eq!(h.channel_texts("Reader").len(), 2, "the operator is answered either way");
}

#[tokio::test]
async fn a_plugin_with_everything_switched_off_is_not_a_peer_worth_asking() {
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
        &Headers::none(),
        &Landing::Loopback,
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
    assert!(!scribe.contains("the Neon connector"), "{scribe}");
}

#[tokio::test]
async fn a_plugin_call_from_an_agent_outside_the_crew_is_refused() {
    let server = serve(Rules::default()).await;
    let endpoint = format!("{}/mcp", server.base());
    let (_dir, store, group, agent) = workspace();

    plugins::connect(
        &store,
        group,
        &PluginKind::Neon,
        &endpoint,
        plugins::Credential::Discover,
        &Headers::none(),
        &Landing::Loopback,
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
            account: plugins::Held::Absent,
        },
        "run_sql",
        &serde_json::json!({}),
    )
    .await
    .expect_err("another crew's sign-in is not this agent's to spend");
    assert!(failed.to_string().contains("not connected"), "{failed}");

    plugins::call(
        &store,
        plugins::Target {
            group,
            agent,
            kind: &PluginKind::Neon,
            endpoint: &endpoint,
            account: plugins::Held::Absent,
        },
        "run_sql",
        &serde_json::json!({}),
    )
    .await
    .expect("the crew that signed in still has it");
}

mod deployments {
    use super::*;

    use axum::body::Body;
    use axum::extract::Query;
    use axum::http::StatusCode;
    use futures_util::StreamExt;

    #[derive(Clone, Default)]
    struct Wants {
        header: Option<(String, String)>,
        bearer: Option<String>,
        redirect: Option<String>,
    }

    #[derive(Clone)]
    struct Deployment {
        wants: Wants,
        base: Arc<Mutex<String>>,
        streams: Arc<Mutex<HashMap<String, tokio::sync::mpsc::UnboundedSender<String>>>>,
        seen: Arc<Mutex<Vec<HashMap<String, String>>>>,
    }

    impl Deployment {
        fn base(&self) -> String {
            self.base.lock().clone()
        }

        fn endpoint(&self) -> String {
            format!("{}/sse", self.base())
        }
    }

    async fn deploy(wants: Wants) -> Deployment {
        let server = Deployment {
            wants,
            base: Arc::new(Mutex::new(String::new())),
            streams: Arc::new(Mutex::new(HashMap::new())),
            seen: Arc::new(Mutex::new(Vec::new())),
        };
        let app = Router::new()
            .route("/sse", get(stream).post(no_post))
            .route("/messages", post(message))
            .with_state(server.clone());
        let listener = listen().await;
        let addr = listener.local_addr().unwrap();
        *server.base.lock() = format!("http://{addr}");
        tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        server
    }

    async fn no_post() -> impl IntoResponse {
        (StatusCode::METHOD_NOT_ALLOWED, "this endpoint is an event stream")
    }

    fn note(server: &Deployment, headers: &axum::http::HeaderMap) {
        server.seen.lock().push(
            headers
                .iter()
                .filter_map(|(name, value)| {
                    value.to_str().ok().map(|v| (name.as_str().to_string(), v.to_string()))
                })
                .collect(),
        );
    }

    fn refuse(server: &Deployment, headers: &axum::http::HeaderMap) -> Option<StatusCode> {
        if let Some((name, value)) = &server.wants.header {
            let sent = headers.get(name.as_str()).and_then(|v| v.to_str().ok());
            if sent != Some(value.as_str()) {
                return Some(StatusCode::FORBIDDEN);
            }
        }
        if let Some(bearer) = &server.wants.bearer {
            let sent = headers
                .get("authorization")
                .and_then(|v| v.to_str().ok())
                .and_then(|v| v.strip_prefix("Bearer "));
            if sent != Some(bearer.as_str()) {
                return Some(StatusCode::UNAUTHORIZED);
            }
        }
        None
    }

    async fn stream(
        State(server): State<Deployment>,
        headers: axum::http::HeaderMap,
    ) -> axum::response::Response {
        note(&server, &headers);
        if let Some(status) = refuse(&server, &headers) {
            return (status, "").into_response();
        }

        let (tx, rx) = tokio::sync::mpsc::unbounded_channel::<String>();
        let id = format!("s{}", server.streams.lock().len() + 1);
        server.streams.lock().insert(id.clone(), tx);

        let named =
            server.wants.redirect.clone().unwrap_or_else(|| format!("/messages?sessionId={id}"));
        let first = format!("event: endpoint\ndata: {named}\n\n");

        let opening =
            futures_util::stream::once(async move { Ok::<String, std::io::Error>(first) });
        let rest = futures_util::stream::unfold(rx, |mut rx| async move {
            rx.recv().await.map(|line| (Ok::<String, std::io::Error>(line), rx))
        });

        axum::response::Response::builder()
            .header("content-type", "text/event-stream")
            .body(Body::from_stream(opening.chain(rest)))
            .unwrap()
    }

    async fn message(
        State(server): State<Deployment>,
        Query(params): Query<HashMap<String, String>>,
        headers: axum::http::HeaderMap,
        axum::Json(body): axum::Json<serde_json::Value>,
    ) -> axum::response::Response {
        note(&server, &headers);
        if let Some(status) = refuse(&server, &headers) {
            return (status, "").into_response();
        }

        let method = body["method"].as_str().unwrap_or_default().to_string();
        let id = body.get("id").cloned().unwrap_or(serde_json::Value::Null);
        let result = match method.as_str() {
            "initialize" => serde_json::json!({
                "protocolVersion": "2024-11-05",
                "capabilities": { "tools": {} },
                "serverInfo": { "name": "Scripted Deployment" },
            }),
            "notifications/initialized" => return StatusCode::ACCEPTED.into_response(),
            "tools/list" => serde_json::json!({
                "tools": [{
                    "name": "turn_on",
                    "description": "Turn something on.",
                    "inputSchema": { "type": "object", "properties": { "what": { "type": "string" } } },
                }],
            }),
            "tools/call" => serde_json::json!({
                "content": [{
                    "type": "text",
                    "text": format!("{} ran", body["params"]["name"].as_str().unwrap_or_default()),
                }],
            }),
            other => {
                push(
                    &server,
                    &params,
                    serde_json::json!({
                        "jsonrpc": "2.0",
                        "id": id,
                        "error": { "code": -32601, "message": format!("no method {other}") },
                    }),
                );
                return StatusCode::ACCEPTED.into_response();
            }
        };

        push(&server, &params, serde_json::json!({ "jsonrpc": "2.0", "id": id, "result": result }));
        StatusCode::ACCEPTED.into_response()
    }

    fn push(server: &Deployment, params: &HashMap<String, String>, reply: serde_json::Value) {
        let Some(id) = params.get("sessionId") else { return };
        if let Some(sender) = server.streams.lock().get(id) {
            let _ = sender.send(format!("event: message\ndata: {reply}\n\n"));
        }
    }

    fn mine(server: &Deployment) -> PluginKind {
        PluginKind::custom("Home Assistant", &server.endpoint()).unwrap()
    }

    #[tokio::test]
    async fn a_server_on_the_older_transport_connects_and_answers_a_call() {
        let server = deploy(Wants::default()).await;
        let (_dir, store, group, agent) = workspace();
        let kind = mine(&server);

        let plugin = plugins::connect(
            &store,
            group,
            &kind,
            kind.endpoint(),
            plugins::Credential::Discover,
            &Headers::none(),
            &Landing::Loopback,
            |_| panic!("a server that asks for nothing must not open a browser"),
        )
        .await
        .expect("a server on the older transport is still a server");

        assert_eq!(plugin.name, "home_assistant");
        assert!(!plugin.signed_in, "it asked for nothing, so nothing was authorized");
        assert_eq!(
            plugin.tools.iter().map(|tool| tool.name.as_str()).collect::<Vec<_>>(),
            vec!["turn_on"],
        );

        let answer = plugins::call(
            &store,
            plugins::Target {
                group,
                agent,
                kind: &kind,
                endpoint: kind.endpoint(),
                account: plugins::Held::Absent,
            },
            "turn_on",
            &serde_json::json!({ "what": "the kettle" }),
        )
        .await
        .expect("and its tools are callable");
        assert_eq!(answer.text, "turn_on ran");
    }

    #[tokio::test]
    async fn a_catalog_server_is_never_offered_the_older_transport() {
        let server = deploy(Wants::default()).await;
        let (_dir, store, group, _agent) = workspace();

        let refused = plugins::connect(
            &store,
            group,
            &PluginKind::Neon,
            &server.endpoint(),
            plugins::Credential::Discover,
            &Headers::none(),
            &Landing::Loopback,
            |_| panic!("nothing here signs in"),
        )
        .await;

        assert!(refused.is_err(), "a vendor is held to the transport it publishes");
        assert!(server.streams.lock().is_empty(), "and the event stream was never even opened",);
    }

    #[tokio::test]
    async fn a_header_the_operator_wrote_is_on_every_request_including_the_stream() {
        let server = deploy(Wants {
            header: Some(("cf-access-client-id".into(), "abc.access".into())),
            ..Wants::default()
        })
        .await;
        let (_dir, store, group, agent) = workspace();
        let kind = mine(&server);
        let headers = Headers::parse(&[HeaderPair {
            name: "Cf-Access-Client-Id".into(),
            value: "abc.access".into(),
        }])
        .unwrap();

        let plugin = plugins::connect(
            &store,
            group,
            &kind,
            kind.endpoint(),
            plugins::Credential::Discover,
            &headers,
            &Landing::Loopback,
            |_| panic!("a gate is not a sign-in"),
        )
        .await
        .expect("the headers get the request past the gate");

        assert_eq!(plugin.headers, vec!["cf-access-client-id".to_string()]);

        plugins::call(
            &store,
            plugins::Target {
                group,
                agent,
                kind: &kind,
                endpoint: kind.endpoint(),
                account: plugins::Held::Absent,
            },
            "turn_on",
            &serde_json::json!({ "what": "the kettle" }),
        )
        .await
        .expect("and stay on every later request too");

        let seen = server.seen.lock().clone();
        assert!(seen.len() > 3, "the stream, the handshake and the calls: {}", seen.len());
        assert!(
            seen.iter().all(|headers| headers.get("cf-access-client-id").map(String::as_str)
                == Some("abc.access")),
            "every request has to carry it, including the GET that opens the stream",
        );
    }

    #[tokio::test]
    async fn a_key_in_a_header_the_operator_named_authenticates_without_a_sign_in() {
        let server = deploy(Wants {
            header: Some(("x-api-key".into(), "hand-minted".into())),
            ..Wants::default()
        })
        .await;
        let (_dir, store, group, _agent) = workspace();
        let kind = mine(&server);
        let headers =
            Headers::parse(&[HeaderPair { name: "X-API-Key".into(), value: "hand-minted".into() }])
                .unwrap();

        let plugin = plugins::connect(
            &store,
            group,
            &kind,
            kind.endpoint(),
            plugins::Credential::Discover,
            &headers,
            &Landing::Loopback,
            |_| panic!("nothing here has an authorization server"),
        )
        .await
        .expect("a header is a credential when that is what the server reads");

        assert!(!plugin.signed_in);
    }

    #[tokio::test]
    async fn a_message_endpoint_on_another_host_is_refused_rather_than_followed() {
        let server = deploy(Wants {
            redirect: Some("https://evil.example/messages".into()),
            ..Wants::default()
        })
        .await;
        let (_dir, store, group, _agent) = workspace();
        let kind = mine(&server);

        let refused = plugins::connect(
            &store,
            group,
            &kind,
            kind.endpoint(),
            plugins::Credential::Discover,
            &Headers::none(),
            &Landing::Loopback,
            |_| panic!("nothing signs in"),
        )
        .await
        .expect_err("a server may not redirect a session onto another host");
        let message = refused.to_string();
        assert!(message.contains("different server"), "{message}");
    }

    #[tokio::test]
    async fn testing_an_address_says_what_it_is_without_connecting_it() {
        let server = deploy(Wants::default()).await;

        let report = plugins::inspect(true, &server.endpoint(), None, &Headers::none())
            .await
            .expect("a reachable server reports");

        assert_eq!(report.transport, "HTTP+SSE (2024-11-05)");
        assert_eq!(report.protocol, "2024-11-05");
        assert!(report.handshake);
        assert_eq!(report.signin, SigninNeed::None);
        assert_eq!(report.server, "Scripted Deployment");
        let again = plugins::inspect(true, &server.endpoint(), None, &Headers::none())
            .await
            .expect("a second dial of the same address");
        assert_eq!(again.server, "Scripted Deployment");
        assert_eq!(again.protocol, "2024-11-05");
        assert_eq!(report.tools, vec!["turn_on".to_string()]);

        let (_dir, store, group, _agent) = workspace();
        assert!(store.group_plugins(group).unwrap().is_empty());
    }

    #[tokio::test]
    async fn testing_tells_a_wrong_key_apart_from_a_server_that_wants_a_sign_in() {
        let server = deploy(Wants { bearer: Some("right".into()), ..Wants::default() }).await;

        let asked = plugins::inspect(true, &server.endpoint(), None, &Headers::none())
            .await
            .expect("a 401 is a finding rather than a failure");
        assert_eq!(asked.signin, SigninNeed::Wanted);
        assert!(asked.tools.is_empty(), "nothing has been signed in to");

        let wrong = plugins::inspect(true, &server.endpoint(), Some("wrong"), &Headers::none())
            .await
            .expect("and so is a refused key");
        assert_eq!(wrong.signin, SigninNeed::Refused);

        let right = plugins::inspect(true, &server.endpoint(), Some("right"), &Headers::none())
            .await
            .expect("a key the server takes");
        assert_eq!(right.signin, SigninNeed::Accepted);
        assert_eq!(right.tools, vec!["turn_on".to_string()]);
    }

    #[tokio::test]
    async fn testing_a_connected_plugin_spends_the_grant_that_is_already_there() {
        let server = deploy(Wants { bearer: Some("hand-minted".into()), ..Wants::default() }).await;
        let (_dir, store, group, _agent) = workspace();
        let kind = mine(&server);

        let plugin = plugins::connect(
            &store,
            group,
            &kind,
            kind.endpoint(),
            plugins::Credential::Key("hand-minted"),
            &Headers::none(),
            &Landing::Loopback,
            |_| panic!("a pasted key skips discovery"),
        )
        .await
        .expect("connected with a key");

        let dialed = store.plugin_dial(plugin.id).unwrap().expect("the row it was written as");
        let report =
            plugins::check(&store, plugin.id, dialed, kind.endpoint(), plugins::Held::Absent)
                .await
                .expect("and the key is still accepted");
        assert_eq!(report.signin, SigninNeed::Accepted);
        assert_eq!(report.tools, vec!["turn_on".to_string()]);
    }

    #[tokio::test]
    async fn a_server_that_is_not_there_reports_the_address_rather_than_a_shape() {
        let refused =
            plugins::inspect(true, "http://127.0.0.1:1/mcp", None, &Headers::none()).await;
        assert!(refused.is_err(), "an address nothing answers on is an error, not a report");
        let message = refused.unwrap_err().to_string();
        assert!(message.contains("127.0.0.1:1"), "{message}");
    }
}

#[tokio::test]
#[ignore = "reaches the real vendors; run ./scripts/plugins.sh"]
async fn every_server_on_the_list_still_publishes_what_this_build_expects() {
    for kind in PluginKind::ALL {
        if kind.account_backed() {
            println!(
                "{}: signs in with the machine's Guaca account, not with a grant of its own",
                kind.label()
            );
            continue;
        }
        let endpoint = kind.endpoint();

        let opened = guac_lib::mcp::open(guac_lib::mcp::Dial::to(endpoint)).await;
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

        let found = guac_lib::oauth::discover(endpoint, challenge.as_deref())
            .await
            .unwrap_or_else(|err| panic!("{} cannot be discovered: {err}", kind.label()));

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

        let asked = found.requested_scope();
        for scope in asked.iter().flat_map(|s| s.split(' ')) {
            let published = if !found.challenge_scopes.is_empty() {
                found.challenge_scopes.iter().any(|s| s == scope)
                    || (scope == "offline_access"
                        && found.server.scopes_supported.iter().any(|s| s == scope))
            } else if found.resource_scopes.is_empty() {
                found.server.scopes_supported.iter().any(|s| s == scope)
            } else {
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
