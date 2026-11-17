use std::collections::HashMap;
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use reqwest::RequestBuilder;
use serde::Deserialize;
use sha2::{Digest, Sha256};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;

const WAIT_FOR_OPERATOR: Duration = Duration::from_secs(5 * 60);

const HTTP_TIMEOUT: Duration = Duration::from_secs(30);

pub const REFRESH_SKEW_MS: i64 = 5 * 60 * 1000;

#[derive(Debug, thiserror::Error)]
pub enum OauthError {
    #[error("could not reach {what} at {url}: {source}")]
    Transport {
        what: &'static str,
        url: String,
        #[source]
        source: reqwest::Error,
    },
    #[error("{url} answered HTTP {status} when asked {what}: {body}")]
    Status { what: &'static str, url: String, status: u16, body: String },
    #[error(
        "{url} does not publish where to sign in ({detail}); this connector cannot be connected"
    )]
    NoMetadata { url: String, detail: String },
    #[error(
        "{issuer} does not let an application register itself, so Guaca cannot sign in to it \
         without being registered there first"
    )]
    NoRegistration { issuer: String },
    #[error("could not listen for the answer from your browser: {source}")]
    NoPort {
        #[source]
        source: std::io::Error,
    },
    #[error("could not open your browser: {detail}")]
    NoBrowser { detail: String },
    #[error("nothing came back from your browser within five minutes; the sign-in was abandoned")]
    TimedOut,
    #[error("the sign-in was refused: {error}{}", .description.as_deref().map(|d| format!(": {d}")).unwrap_or_default())]
    Refused { error: String, description: Option<String> },
    #[error("the answer from your browser did not match the sign-in that was started")]
    StateMismatch,
    #[error(
        "the answer came back naming {named}, and the sign-in was started at {expected}. \
         Nothing was connected."
    )]
    IssuerMismatch { expected: String, named: String },
    #[error("{issuer} issued a grant with no access token in it")]
    NoToken { issuer: String },
    #[error("this connector has no refresh token, so its sign-in cannot be renewed")]
    NoRefreshToken,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Grant {
    pub access_token: String,
    pub refresh_token: Option<String>,
    pub expires_at: Option<i64>,
    pub client_id: String,
    pub client_secret: Option<String>,
    pub token_endpoint: String,
}

impl Grant {
    pub fn key(pasted: &str) -> Grant {
        Grant {
            access_token: pasted.to_string(),
            refresh_token: None,
            expires_at: None,
            client_id: String::new(),
            client_secret: None,
            token_endpoint: String::new(),
        }
    }

    pub fn stale(&self, now_ms: i64) -> bool {
        match self.expires_at {
            Some(at) => at - REFRESH_SKEW_MS <= now_ms,
            None => false,
        }
    }
}

#[derive(Clone, Default)]
pub struct Gate<'a> {
    origin: String,
    headers: &'a [(String, String)],
}

impl<'a> Gate<'a> {
    pub fn none() -> Gate<'static> {
        Gate::default()
    }

    pub fn on(resource: &str, headers: &'a [(String, String)]) -> Gate<'a> {
        Gate {
            origin: split_origin(resource).map(|(origin, _)| origin).unwrap_or_default(),
            headers,
        }
    }

    fn apply(&self, request: RequestBuilder, url: &str) -> RequestBuilder {
        if self.headers.is_empty() || self.origin.is_empty() {
            return request;
        }
        let Some((origin, _)) = split_origin(url) else { return request };
        if !origin.eq_ignore_ascii_case(&self.origin) {
            return request;
        }
        let mut request = request;
        for (name, value) in self.headers {
            request = request.header(name.as_str(), value.as_str());
        }
        request
    }
}

pub async fn authorize(
    resource: &str,
    challenge_header: Option<&str>,
    gate: &Gate<'_>,
    landing: &Landing,
    open: impl FnOnce(&str) -> Result<(), String>,
    now_ms: impl Fn() -> i64,
) -> Result<Grant, OauthError> {
    let http = http()?;
    let discovered = discover_through(resource, challenge_header, gate).await?;
    let scope = discovered.requested_scope();
    let Discovered { issuer, server, .. } = discovered;

    let Some(registration_endpoint) = server.registration_endpoint.clone() else {
        return Err(OauthError::NoRegistration { issuer });
    };

    let verifier = secret();
    let challenge = pkce_challenge(&verifier);
    let state = secret();

    let opened = landing.open(&state).await?;
    let redirect_uri = opened.redirect_uri.clone();

    let client = register(&http, &registration_endpoint, &redirect_uri, gate).await?;

    let mut url = format!(
        "{}?response_type=code&client_id={}&redirect_uri={}&state={}\
         &code_challenge={}&code_challenge_method=S256&resource={}",
        server.authorization_endpoint,
        encode(&client.client_id),
        encode(&redirect_uri),
        encode(&state),
        encode(&challenge),
        encode(resource),
    );
    if let Some(scope) = scope {
        url.push_str(&format!("&scope={}", encode(&scope)));
    }

    open(&url).map_err(|detail| OauthError::NoBrowser { detail })?;

    let code = opened.wait(&state, &issuer).await?;
    exchange(&http, &server, &client, &code, &verifier, &redirect_uri, resource, gate, &now_ms)
        .await
}

pub struct Answer {
    pub fields: Vec<(String, String)>,
    pub reply: tokio::sync::oneshot::Sender<(u16, &'static str)>,
}

pub type Callbacks = Arc<Mutex<HashMap<String, tokio::sync::mpsc::UnboundedSender<Answer>>>>;

#[derive(Clone)]
pub enum Landing {
    Loopback,
    Served { origin: String, callbacks: Callbacks },
}

pub const CALLBACK_ROUTE: &str = "/v1/oauth/callback";

impl Landing {
    pub async fn open(&self, state: &str) -> Result<Opened, OauthError> {
        match self {
            Landing::Loopback => {
                let listener = TcpListener::bind("127.0.0.1:0")
                    .await
                    .map_err(|source| OauthError::NoPort { source })?;
                let port =
                    listener.local_addr().map_err(|source| OauthError::NoPort { source })?.port();
                Ok(Opened {
                    redirect_uri: format!("http://127.0.0.1:{port}/callback"),
                    waiter: Waiter::Loopback(listener),
                })
            }
            Landing::Served { origin, callbacks } => {
                let (sender, receiver) = tokio::sync::mpsc::unbounded_channel();
                callbacks
                    .lock()
                    .unwrap_or_else(PoisonError::into_inner)
                    .insert(state.to_string(), sender);
                let mut callback_origin = origin.trim_end_matches('/').to_string();
                if let Ok(mut url) = reqwest::Url::parse(&callback_origin) {
                    if url.scheme() == "http" && url.host_str() == Some("localhost") {
                        url.set_host(Some("127.0.0.1")).expect("a loopback IP is a valid host");
                        callback_origin = url.as_str().trim_end_matches('/').to_string();
                    }
                }
                Ok(Opened {
                    redirect_uri: format!("{callback_origin}{CALLBACK_ROUTE}"),
                    waiter: Waiter::Served {
                        receiver,
                        filed: Filed { state: state.to_string(), in_: callbacks.clone() },
                    },
                })
            }
        }
    }

    pub async fn deliver(&self, fields: Vec<(String, String)>) -> Option<(u16, &'static str)> {
        let Landing::Served { callbacks, .. } = self else { return None };
        let state = fields.iter().find(|(k, _)| k == "state").map(|(_, v)| v.clone())?;
        let sender =
            callbacks.lock().unwrap_or_else(PoisonError::into_inner).get(&state).cloned()?;
        let (reply, page) = tokio::sync::oneshot::channel();
        sender.send(Answer { fields, reply }).ok()?;
        page.await.ok()
    }
}

pub struct Opened {
    pub redirect_uri: String,
    waiter: Waiter,
}

enum Waiter {
    Loopback(TcpListener),
    Served { receiver: tokio::sync::mpsc::UnboundedReceiver<Answer>, filed: Filed },
}

struct Filed {
    state: String,
    in_: Callbacks,
}

impl Drop for Filed {
    fn drop(&mut self) {
        self.in_.lock().unwrap_or_else(PoisonError::into_inner).remove(&self.state);
    }
}

impl Opened {
    pub async fn wait(self, state: &str, issuer: &str) -> Result<String, OauthError> {
        match self.waiter {
            Waiter::Loopback(listener) => wait_for_redirect(listener, state, issuer).await,
            Waiter::Served { mut receiver, filed: _filed } => {
                let deadline = tokio::time::sleep(WAIT_FOR_OPERATOR);
                tokio::pin!(deadline);
                loop {
                    let answer = tokio::select! {
                        _ = &mut deadline => return Err(OauthError::TimedOut),
                        answer = receiver.recv() => answer,
                    };
                    let Some(Answer { fields, reply }) = answer else {
                        return Err(OauthError::TimedOut);
                    };
                    match read_answer(&fields, state, issuer) {
                        Callback::Complete(read) => {
                            let _ = reply.send(page_for(&read));
                            return read;
                        }
                        Callback::Unrelated => {
                            let _ = reply.send((404, NOT_THE_SIGNIN));
                        }
                        Callback::Rejected => {
                            tracing::warn!("rejected OAuth callback with mismatched state");
                            let _ = reply.send(page_for(&Err(OauthError::StateMismatch)));
                        }
                    }
                }
            }
        }
    }
}

const NOT_THE_SIGNIN: &str = "Not the sign-in.";

fn page_for(read: &Result<String, OauthError>) -> (u16, &'static str) {
    match read {
        Ok(_) => (200, "Connected. You can close this tab and go back to Guaca."),
        Err(OauthError::Refused { .. }) => (200, "Sign-in refused. You can close this tab."),
        Err(_) => (400, "That did not match. Nothing was connected."),
    }
}

enum Callback {
    Unrelated,
    Rejected,
    Complete(Result<String, OauthError>),
}

fn read_answer(fields: &[(String, String)], state: &str, issuer: &str) -> Callback {
    let has_answer = fields.iter().any(|(k, _)| k == "code" || k == "error");
    if !has_answer {
        return Callback::Unrelated;
    }
    if fields.iter().find(|(k, _)| k == "state").map(|(_, v)| v.as_str()) != Some(state) {
        return Callback::Rejected;
    }
    let named = fields.iter().find(|(k, _)| k == "iss").map(|(_, v)| v.as_str());
    if let Some(named) = named {
        if named != issuer {
            return Callback::Complete(Err(OauthError::IssuerMismatch {
                expected: issuer.to_string(),
                named: named.to_string(),
            }));
        }
    }

    if let Some(error) = fields.iter().find(|(k, _)| k == "error").map(|(_, v)| v.clone()) {
        let description =
            fields.iter().find(|(k, _)| k == "error_description").map(|(_, v)| v.clone());
        return Callback::Complete(Err(OauthError::Refused { error, description }));
    }

    let code = fields.iter().find(|(k, _)| k == "code").map(|(_, v)| v.clone()).unwrap();
    Callback::Complete(Ok(code))
}

pub async fn refresh(
    grant: &Grant,
    gate: &Gate<'_>,
    now_ms: impl Fn() -> i64,
) -> Result<Grant, OauthError> {
    let Some(refresh_token) = grant.refresh_token.clone() else {
        return Err(OauthError::NoRefreshToken);
    };
    let http = http()?;

    let mut form = vec![
        ("grant_type".to_string(), "refresh_token".to_string()),
        ("refresh_token".to_string(), refresh_token.clone()),
        ("client_id".to_string(), grant.client_id.clone()),
    ];
    if let Some(secret) = &grant.client_secret {
        form.push(("client_secret".to_string(), secret.clone()));
    }

    let issued = post_token(&http, &grant.token_endpoint, &form, gate).await?;
    Ok(Grant {
        access_token: issued.access_token,
        refresh_token: issued.refresh_token.or(Some(refresh_token)),
        expires_at: issued.expires_in.map(|secs| now_ms() + secs * 1000),
        client_id: grant.client_id.clone(),
        client_secret: grant.client_secret.clone(),
        token_endpoint: grant.token_endpoint.clone(),
    })
}

#[derive(Debug, Clone)]
pub struct Discovered {
    pub issuer: String,
    pub server: ServerMetadata,
    pub resource_scopes: Vec<String>,
    pub challenge_scopes: Vec<String>,
}

pub async fn discover(
    resource: &str,
    challenge_header: Option<&str>,
) -> Result<Discovered, OauthError> {
    discover_through(resource, challenge_header, &Gate::none()).await
}

async fn discover_through(
    resource: &str,
    challenge_header: Option<&str>,
    gate: &Gate<'_>,
) -> Result<Discovered, OauthError> {
    let http = http()?;
    let Resource { issuer, scopes } = resource_for(&http, resource, challenge_header, gate).await?;
    let server = server_metadata(&http, &issuer, gate).await?;
    Ok(Discovered {
        issuer,
        server,
        resource_scopes: scopes,
        challenge_scopes: challenge_header.map(challenge_scope).unwrap_or_default(),
    })
}

fn challenge_scope(challenge: &str) -> Vec<String> {
    let Some(at) = challenge.find("scope=") else { return Vec::new() };
    if !challenge[..at].is_empty()
        && !challenge[..at].ends_with([' ', ','])
        && !challenge[..at].ends_with("Bearer ")
    {
        return Vec::new();
    }
    let rest = &challenge[at + "scope=".len()..];
    let value = match rest.strip_prefix('"') {
        Some(quoted) => quoted.split('"').next().unwrap_or_default(),
        None => rest.split([',', ' ']).next().unwrap_or_default(),
    };
    value.split_whitespace().map(str::to_string).collect()
}

struct Resource {
    issuer: String,
    scopes: Vec<String>,
}

async fn resource_for(
    http: &reqwest::Client,
    resource: &str,
    challenge: Option<&str>,
    gate: &Gate<'_>,
) -> Result<Resource, OauthError> {
    let mut tried = Vec::new();

    if let Some(url) = challenge.and_then(resource_metadata_url) {
        tried.push(url.clone());
        if let Some(found) = protected_resource(http, &url, gate).await {
            return Ok(found);
        }
    }

    for url in well_known(resource, "oauth-protected-resource") {
        tried.push(url.clone());
        if let Some(found) = protected_resource(http, &url, gate).await {
            return Ok(found);
        }
    }

    Err(OauthError::NoMetadata {
        url: resource.to_string(),
        detail: format!("nothing at {}", tried.join(", ")),
    })
}

async fn protected_resource(
    http: &reqwest::Client,
    url: &str,
    gate: &Gate<'_>,
) -> Option<Resource> {
    #[derive(Deserialize)]
    struct Metadata {
        #[serde(default)]
        authorization_servers: Vec<String>,
        #[serde(default)]
        scopes_supported: Vec<String>,
    }

    let response = gate.apply(http.get(url), url).timeout(HTTP_TIMEOUT).send().await.ok()?;
    if !response.status().is_success() {
        return None;
    }
    let metadata: Metadata = response.json().await.ok()?;
    let issuer = metadata.authorization_servers.into_iter().next()?;
    Some(Resource { issuer, scopes: metadata.scopes_supported })
}

#[derive(Debug, Clone, Deserialize)]
pub struct ServerMetadata {
    #[serde(default)]
    pub issuer: String,
    pub authorization_endpoint: String,
    pub token_endpoint: String,
    #[serde(default)]
    pub registration_endpoint: Option<String>,
    #[serde(default)]
    pub scopes_supported: Vec<String>,
    #[serde(default)]
    pub code_challenge_methods_supported: Vec<String>,
}

pub(crate) async fn server_metadata(
    http: &reqwest::Client,
    issuer: &str,
    gate: &Gate<'_>,
) -> Result<ServerMetadata, OauthError> {
    let mut tried = Vec::new();
    for name in ["oauth-authorization-server", "openid-configuration"] {
        for url in well_known(issuer, name) {
            tried.push(url.clone());
            let Ok(response) = gate.apply(http.get(&url), &url).timeout(HTTP_TIMEOUT).send().await
            else {
                continue;
            };
            if !response.status().is_success() {
                continue;
            }
            if let Ok(metadata) = response.json::<ServerMetadata>().await {
                return Ok(metadata);
            }
        }
    }
    Err(OauthError::NoMetadata {
        url: issuer.to_string(),
        detail: format!("nothing at {}", tried.join(", ")),
    })
}

fn well_known(url: &str, name: &str) -> Vec<String> {
    let Some((origin, path)) = split_origin(url) else {
        return Vec::new();
    };
    let path = path.trim_end_matches('/');
    let mut out = Vec::new();
    if !path.is_empty() {
        out.push(format!("{origin}/.well-known/{name}{path}"));
        out.push(format!("{origin}{path}/.well-known/{name}"));
    }
    out.push(format!("{origin}/.well-known/{name}"));
    out
}

pub(crate) fn split_origin(url: &str) -> Option<(String, String)> {
    let (scheme, rest) = url.split_once("://")?;
    match rest.find('/') {
        Some(at) => Some((format!("{scheme}://{}", &rest[..at]), rest[at..].to_string())),
        None => Some((format!("{scheme}://{rest}"), String::new())),
    }
}

fn resource_metadata_url(challenge: &str) -> Option<String> {
    let at = challenge.find("resource_metadata=")? + "resource_metadata=".len();
    let rest = &challenge[at..];
    let value = match rest.strip_prefix('"') {
        Some(quoted) => quoted.split('"').next()?,
        None => rest.split(&[',', ' '][..]).next()?,
    };
    (!value.is_empty()).then(|| value.to_string())
}

impl Discovered {
    pub fn requested_scope(&self) -> Option<String> {
        let published = if !self.challenge_scopes.is_empty() {
            &self.challenge_scopes
        } else if !self.resource_scopes.is_empty() {
            &self.resource_scopes
        } else {
            &self.server.scopes_supported
        };

        let mut wanted: Vec<&str> =
            published.iter().map(String::as_str).filter(|s| *s != "*").collect();

        let offline = "offline_access";
        if !wanted.contains(&offline) && self.server.scopes_supported.iter().any(|s| s == offline) {
            wanted.push(offline);
        }

        (!wanted.is_empty()).then(|| wanted.join(" "))
    }
}

#[derive(Debug, Clone, Deserialize)]
struct Registered {
    client_id: String,
    #[serde(default)]
    client_secret: Option<String>,
    #[serde(default)]
    token_endpoint_auth_method: Option<String>,
}

impl Registered {
    fn secret(&self) -> Option<String> {
        match self.token_endpoint_auth_method.as_deref() {
            Some("none") | None => None,
            _ => self.client_secret.clone(),
        }
    }
}

async fn register(
    http: &reqwest::Client,
    endpoint: &str,
    redirect_uri: &str,
    gate: &Gate<'_>,
) -> Result<Registered, OauthError> {
    let body = serde_json::json!({
        "client_name": "Guaca",
        "redirect_uris": [redirect_uri],
        "grant_types": ["authorization_code", "refresh_token"],
        "response_types": ["code"],
        "token_endpoint_auth_method": "none",
    });

    let response = gate
        .apply(http.post(endpoint), endpoint)
        .timeout(HTTP_TIMEOUT)
        .json(&body)
        .send()
        .await
        .map_err(|source| OauthError::Transport {
            what: "to register",
            url: endpoint.to_string(),
            source,
        })?;

    let status = response.status();
    let text = response.text().await.unwrap_or_default();
    if !status.is_success() {
        return Err(OauthError::Status {
            what: "to register",
            url: endpoint.to_string(),
            status: status.as_u16(),
            body: text.chars().take(300).collect(),
        });
    }

    serde_json::from_str(&text).map_err(|err| OauthError::NoMetadata {
        url: endpoint.to_string(),
        detail: format!("its registration answer did not parse: {err}"),
    })
}

pub(crate) async fn wait_for_redirect(
    listener: TcpListener,
    state: &str,
    issuer: &str,
) -> Result<String, OauthError> {
    tokio::time::timeout(WAIT_FOR_OPERATOR, async {
        loop {
            let accepted = listener.accept().await;
            let Ok((mut socket, _)) = accepted else { continue };

            let mut buffer = [0u8; 8192];
            let Ok(read) = socket.read(&mut buffer).await else { continue };
            let request = String::from_utf8_lossy(&buffer[..read]).to_string();
            let Some(target) = request.split_whitespace().nth(1) else {
                let _ = socket.shutdown().await;
                continue;
            };

            let Some(query) = target.split_once('?').map(|(_, q)| q) else {
                let _ = reply(&mut socket, 404, NOT_THE_SIGNIN).await;
                continue;
            };

            match read_answer(&parse_query(query), state, issuer) {
                Callback::Complete(read) => {
                    let (status, page) = page_for(&read);
                    let _ = reply(&mut socket, status, page).await;
                    return read;
                }
                Callback::Unrelated => {
                    let _ = reply(&mut socket, 404, NOT_THE_SIGNIN).await;
                    continue;
                }
                Callback::Rejected => {
                    tracing::warn!("rejected OAuth callback with mismatched state");
                    let (status, page) = page_for(&Err(OauthError::StateMismatch));
                    let _ = reply(&mut socket, status, page).await;
                }
            }
        }
    })
    .await
    .unwrap_or(Err(OauthError::TimedOut))
}

async fn reply(
    socket: &mut tokio::net::TcpStream,
    status: u16,
    message: &str,
) -> std::io::Result<()> {
    let body = format!(
        "<!doctype html><meta charset=\"utf-8\"><title>Guaca</title>\
         <body style=\"font:16px system-ui;padding:3rem;color:#1c1c1c\">{message}</body>"
    );
    let head = format!(
        "HTTP/1.1 {status} OK\r\ncontent-type: text/html; charset=utf-8\r\n\
         content-length: {}\r\nconnection: close\r\n\r\n",
        body.len()
    );
    socket.write_all(head.as_bytes()).await?;
    socket.write_all(body.as_bytes()).await?;
    socket.flush().await?;
    socket.shutdown().await
}

pub fn parse_query(query: &str) -> Vec<(String, String)> {
    query
        .split('&')
        .filter_map(|pair| pair.split_once('='))
        .map(|(key, value)| (key.to_string(), decode(value)))
        .collect()
}

#[derive(Debug, Deserialize)]
pub(crate) struct Issued {
    pub access_token: String,
    #[serde(default)]
    pub refresh_token: Option<String>,
    #[serde(default)]
    pub expires_in: Option<i64>,
}

#[allow(clippy::too_many_arguments)]
async fn exchange(
    http: &reqwest::Client,
    server: &ServerMetadata,
    client: &Registered,
    code: &str,
    verifier: &str,
    redirect_uri: &str,
    resource: &str,
    gate: &Gate<'_>,
    now_ms: &impl Fn() -> i64,
) -> Result<Grant, OauthError> {
    let secret = client.secret();
    let mut form = vec![
        ("grant_type".to_string(), "authorization_code".to_string()),
        ("code".to_string(), code.to_string()),
        ("redirect_uri".to_string(), redirect_uri.to_string()),
        ("client_id".to_string(), client.client_id.clone()),
        ("code_verifier".to_string(), verifier.to_string()),
        ("resource".to_string(), resource.to_string()),
    ];
    if let Some(secret) = &secret {
        form.push(("client_secret".to_string(), secret.clone()));
    }

    let issued = post_token(http, &server.token_endpoint, &form, gate).await?;
    if issued.access_token.is_empty() {
        return Err(OauthError::NoToken { issuer: server.token_endpoint.clone() });
    }

    Ok(Grant {
        access_token: issued.access_token,
        refresh_token: issued.refresh_token,
        expires_at: issued.expires_in.map(|secs| now_ms() + secs * 1000),
        client_id: client.client_id.clone(),
        client_secret: secret,
        token_endpoint: server.token_endpoint.clone(),
    })
}

pub(crate) async fn post_token(
    http: &reqwest::Client,
    endpoint: &str,
    form: &[(String, String)],
    gate: &Gate<'_>,
) -> Result<Issued, OauthError> {
    let response = gate
        .apply(http.post(endpoint), endpoint)
        .timeout(HTTP_TIMEOUT)
        .form(form)
        .send()
        .await
        .map_err(|source| OauthError::Transport {
            what: "for a token",
            url: endpoint.to_string(),
            source,
        })?;

    let status = response.status();
    let text = response.text().await.unwrap_or_default();
    if !status.is_success() {
        return Err(OauthError::Status {
            what: "for a token",
            url: endpoint.to_string(),
            status: status.as_u16(),
            body: text.chars().take(300).collect(),
        });
    }

    serde_json::from_str(&text).map_err(|err| OauthError::NoMetadata {
        url: endpoint.to_string(),
        detail: format!("its token answer did not parse: {err}"),
    })
}

pub(crate) fn http() -> Result<reqwest::Client, OauthError> {
    reqwest::Client::builder().build().map_err(|source| OauthError::Transport {
        what: "to build a client",
        url: String::new(),
        source,
    })
}

pub(crate) fn secret() -> String {
    let mut bytes = [0u8; 32];
    bytes[..16].copy_from_slice(uuid::Uuid::new_v4().as_bytes());
    bytes[16..].copy_from_slice(uuid::Uuid::new_v4().as_bytes());
    base64url(&bytes)
}

pub(crate) fn pkce_challenge(verifier: &str) -> String {
    base64url(&Sha256::digest(verifier.as_bytes()))
}

fn base64url(raw: &[u8]) -> String {
    const TABLE: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-_";
    let mut out = String::new();
    for chunk in raw.chunks(3) {
        let b = [chunk[0], *chunk.get(1).unwrap_or(&0), *chunk.get(2).unwrap_or(&0)];
        let n = ((b[0] as u32) << 16) | ((b[1] as u32) << 8) | b[2] as u32;
        out.push(TABLE[(n >> 18) as usize & 63] as char);
        out.push(TABLE[(n >> 12) as usize & 63] as char);
        if chunk.len() > 1 {
            out.push(TABLE[(n >> 6) as usize & 63] as char);
        }
        if chunk.len() > 2 {
            out.push(TABLE[n as usize & 63] as char);
        }
    }
    out
}

pub(crate) fn encode(raw: &str) -> String {
    let mut out = String::with_capacity(raw.len());
    for byte in raw.as_bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'.' | b'_' | b'~' => {
                out.push(*byte as char)
            }
            other => out.push_str(&format!("%{other:02X}")),
        }
    }
    out
}

fn decode(raw: &str) -> String {
    let bytes = raw.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut at = 0;
    while at < bytes.len() {
        match bytes[at] {
            b'%' if at + 2 < bytes.len() => match u8::from_str_radix(&raw[at + 1..at + 3], 16) {
                Ok(byte) => {
                    out.push(byte);
                    at += 3;
                }
                Err(_) => {
                    out.push(b'%');
                    at += 1;
                }
            },
            b'+' => {
                out.push(b' ');
                at += 1;
            }
            byte => {
                out.push(byte);
                at += 1;
            }
        }
    }
    String::from_utf8_lossy(&out).to_string()
}

#[cfg(test)]
mod landing_tests {
    use super::*;

    #[test]
    fn unrelated_error_does_not_end_the_signin() {
        for query in ["error=denied", "error=denied&state=wrong"] {
            assert!(
                matches!(
                    read_answer(&parse_query(query), "expected", "issuer"),
                    Callback::Rejected
                ),
                "{query}"
            );
        }
    }

    #[test]
    fn callback_transitions_validate_state_before_code_error_and_issuer() {
        for answer in ["code=ok", "error=denied"] {
            for state in ["", "&state=wrong"] {
                let query = format!("{answer}{state}&iss=other");
                assert!(
                    matches!(
                        read_answer(&parse_query(&query), "expected", "issuer"),
                        Callback::Rejected
                    ),
                    "{query}"
                );
            }
        }
        assert!(matches!(
            read_answer(&parse_query("favicon=1"), "expected", "issuer"),
            Callback::Unrelated
        ));
        assert!(
            matches!(read_answer(&parse_query("code=ok&state=expected"), "expected", "issuer"), Callback::Complete(Ok(code)) if code == "ok")
        );
        assert!(matches!(
            read_answer(&parse_query("error=denied&state=expected"), "expected", "issuer"),
            Callback::Complete(Err(OauthError::Refused { .. }))
        ));
        for answer in ["code=ok", "error=denied"] {
            let query = format!("{answer}&state=expected&iss=other");
            assert!(matches!(
                read_answer(&parse_query(&query), "expected", "issuer"),
                Callback::Complete(Err(OauthError::IssuerMismatch { .. }))
            ));
        }
    }

    #[tokio::test(start_paused = true)]
    async fn idle_loopback_client_cannot_outlive_the_deadline() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let client = tokio::net::TcpStream::connect(address).await.unwrap();
        let task = tokio::spawn(wait_for_redirect(listener, "expected", "issuer"));
        tokio::task::yield_now().await;
        tokio::time::advance(WAIT_FOR_OPERATOR).await;
        tokio::task::yield_now().await;
        let finished = task.is_finished();
        if !finished {
            task.abort();
            panic!("idle socket held the sign-in beyond its deadline");
        }
        assert!(matches!(task.await.unwrap(), Err(OauthError::TimedOut)));
        drop(client);
    }

    #[tokio::test(start_paused = true)]
    async fn unrelated_requests_do_not_restart_the_deadline() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let task = tokio::spawn(wait_for_redirect(listener, "expected", "issuer"));
        tokio::task::yield_now().await;
        tokio::time::advance(WAIT_FOR_OPERATOR / 2).await;
        let mut client = tokio::net::TcpStream::connect(address).await.unwrap();
        client.write_all(b"GET /favicon.ico HTTP/1.1\r\n\r\n").await.unwrap();
        let mut response = Vec::new();
        client.read_to_end(&mut response).await.unwrap();
        assert!(response.starts_with(b"HTTP/1.1 404"));
        tokio::time::advance(WAIT_FOR_OPERATOR / 2).await;
        assert!(matches!(task.await.unwrap(), Err(OauthError::TimedOut)));
    }

    #[tokio::test]
    async fn local_hosted_callbacks_use_a_loopback_ip_for_native_oauth() {
        for (origin, expected) in [
            ("http://localhost:8788", "http://127.0.0.1:8788"),
            ("http://127.0.0.1:9999", "http://127.0.0.1:9999"),
            ("https://localhost:8788", "https://localhost:8788"),
            ("https://workspace.example", "https://workspace.example"),
        ] {
            let landing = Landing::Served { origin: origin.into(), callbacks: Default::default() };
            assert_eq!(
                landing.open("test").await.unwrap().redirect_uri,
                format!("{expected}{CALLBACK_ROUTE}")
            );
        }
    }

    fn served() -> Landing {
        Landing::Served { origin: "https://box.example".into(), callbacks: Default::default() }
    }

    #[tokio::test]
    async fn a_served_landing_names_the_route_on_the_origin_it_was_reached_at() {
        let opened = served().open("s1").await.unwrap();
        assert_eq!(opened.redirect_uri, "https://box.example/v1/oauth/callback");
        let landing = Landing::Served {
            origin: "https://box.example/".into(),
            callbacks: Default::default(),
        };
        let opened = landing.open("s2").await.unwrap();
        assert_eq!(opened.redirect_uri, "https://box.example/v1/oauth/callback");
    }

    #[tokio::test]
    async fn a_browser_delivered_to_a_served_landing_reaches_the_flow_waiting_on_its_state() {
        let landing = served();
        let opened = landing.open("expected").await.unwrap();
        let deliver = {
            let landing = landing.clone();
            tokio::spawn(async move {
                landing
                    .deliver(vec![
                        ("state".into(), "expected".into()),
                        ("code".into(), "the-code".into()),
                    ])
                    .await
            })
        };
        let code = opened.wait("expected", "https://issuer.example").await.unwrap();
        assert_eq!(code, "the-code");
        let page = deliver.await.unwrap().expect("somebody was waiting");
        assert_eq!(page.0, 200);
        assert!(page.1.contains("Connected"), "{}", page.1);
    }

    #[tokio::test]
    async fn a_callback_nobody_is_waiting_for_reaches_nobody() {
        let landing = served();
        let page = landing
            .deliver(vec![("state".into(), "stale".into()), ("code".into(), "x".into())])
            .await;
        assert!(page.is_none());
        assert!(landing.deliver(vec![("code".into(), "x".into())]).await.is_none());
    }

    #[tokio::test]
    async fn a_served_flow_that_ends_takes_its_state_out_of_the_map() {
        let landing = served();
        let Landing::Served { callbacks, .. } = &landing else { unreachable!() };
        {
            let _opened = landing.open("brief").await.unwrap();
            assert_eq!(callbacks.lock().unwrap().len(), 1);
        }
        assert!(callbacks.lock().unwrap().is_empty(), "the flow was dropped without answering");
    }

    #[tokio::test]
    async fn a_mix_up_on_a_served_landing_is_refused_and_the_browser_is_told_nothing_connected() {
        let landing = served();
        let opened = landing.open("expected").await.unwrap();
        let deliver = {
            let landing = landing.clone();
            tokio::spawn(async move {
                landing
                    .deliver(vec![
                        ("state".into(), "expected".into()),
                        ("iss".into(), "https://someone-else.example".into()),
                        ("code".into(), "the-code".into()),
                    ])
                    .await
            })
        };
        let err = opened.wait("expected", "https://issuer.example").await.unwrap_err();
        assert!(matches!(err, OauthError::IssuerMismatch { .. }), "{err}");
        let page = deliver.await.unwrap().unwrap();
        assert_eq!(page.0, 400);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_pkce_challenge_matches_the_rfc_example() {
        let verifier = "dBjftJeZ4CVP-mB92K27uhbUJU1p1r_wW1gFWFOEjXk";
        assert_eq!(pkce_challenge(verifier), "E9Melhoa2OwvFrEMTJguCHaoeK1t8URWbuGJSstw-cM");
    }

    #[test]
    fn base64url_never_emits_a_character_a_url_would_eat() {
        for length in 0..64usize {
            let raw: Vec<u8> = (0..length).map(|i| (i * 7 + 3) as u8).collect();
            let encoded = base64url(&raw);
            assert!(
                encoded.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_'),
                "{encoded} has to survive being a query parameter"
            );
        }
    }

    #[test]
    fn a_secret_is_long_enough_to_be_a_verifier() {
        let secret = secret();
        assert_eq!(secret.len(), 43);
        assert_ne!(secret, super::secret());
    }

    #[test]
    fn a_query_parameter_survives_the_round_trip() {
        for raw in ["https://mcp.neon.tech/mcp", "a b&c=d", "ünïcode", ""] {
            assert_eq!(decode(&encode(raw)), raw);
        }
    }

    #[test]
    fn a_plus_in_a_redirect_is_a_space_and_not_a_plus() {
        assert_eq!(decode("one+two"), "one two");
        assert_eq!(decode("one%2Btwo"), "one+two");
    }

    #[test]
    fn a_truncated_escape_is_left_alone_rather_than_dropped() {
        assert_eq!(decode("100%"), "100%");
        assert_eq!(decode("%zz"), "%zz");
    }

    #[test]
    fn metadata_addresses_put_the_well_known_segment_before_the_path() {
        assert_eq!(
            well_known("https://example.test/tenant", "oauth-authorization-server"),
            vec![
                "https://example.test/.well-known/oauth-authorization-server/tenant",
                "https://example.test/tenant/.well-known/oauth-authorization-server",
                "https://example.test/.well-known/oauth-authorization-server",
            ]
        );
    }

    #[test]
    fn an_issuer_with_no_path_has_one_address() {
        assert_eq!(
            well_known("https://mcp.neon.tech", "oauth-protected-resource"),
            vec!["https://mcp.neon.tech/.well-known/oauth-protected-resource"]
        );
    }

    #[test]
    fn a_challenge_names_where_the_metadata_is() {
        let challenge = r#"Bearer error="invalid_token", resource_metadata="https://x.test/.well-known/oauth-protected-resource""#;
        assert_eq!(
            resource_metadata_url(challenge).as_deref(),
            Some("https://x.test/.well-known/oauth-protected-resource")
        );
        assert_eq!(resource_metadata_url("Bearer").as_deref(), None);
    }

    fn server_offering(scopes: &[&str]) -> ServerMetadata {
        ServerMetadata {
            issuer: "https://x.test".into(),
            authorization_endpoint: "https://x.test/a".into(),
            token_endpoint: "https://x.test/t".into(),
            registration_endpoint: None,
            scopes_supported: scopes.iter().map(|s| (*s).to_string()).collect(),
            code_challenge_methods_supported: vec!["S256".into()],
        }
    }

    fn scopes(resource: &[&str], server: &[&str]) -> Option<String> {
        challenged(&[], resource, server)
    }

    fn challenged(challenge: &[&str], resource: &[&str], server: &[&str]) -> Option<String> {
        Discovered {
            issuer: "https://x.test".into(),
            server: server_offering(server),
            resource_scopes: resource.iter().map(|s| (*s).to_string()).collect(),
            challenge_scopes: challenge.iter().map(|s| (*s).to_string()).collect(),
        }
        .requested_scope()
    }

    #[test]
    fn what_the_refusal_itself_named_beats_both_documents() {
        assert_eq!(
            challenged(&["files:read"], &["read", "write"], &["read", "write", "admin"]).as_deref(),
            Some("files:read")
        );
    }

    #[test]
    fn a_scope_is_read_out_of_a_challenge_only_where_the_challenge_put_one() {
        assert_eq!(
            challenge_scope(r#"Bearer error="invalid_token", scope="files:read files:write""#),
            vec!["files:read", "files:write"]
        );
        assert_eq!(challenge_scope("Bearer scope=files:read"), vec!["files:read"]);
        assert!(challenge_scope(
            r#"Bearer resource_metadata="https://x.test/.well-known/oauth-protected-resource""#
        )
        .is_empty());
        assert!(challenge_scope("Bearer").is_empty());
        assert!(challenge_scope(r#"Bearer myscope="nope""#).is_empty());
    }

    #[test]
    fn a_wildcard_scope_is_never_asked_for() {
        assert_eq!(scopes(&[], &["read", "write", "*"]).as_deref(), Some("read write"));
    }

    #[test]
    fn a_server_that_publishes_no_scopes_is_not_asked_for_one() {
        assert_eq!(scopes(&[], &[]), None);
    }

    #[test]
    fn the_resource_decides_what_is_asked_for_and_not_its_authorization_server() {
        let asked = scopes(
            &["openid", "email", "profile"],
            &[
                "openid",
                "profile",
                "email",
                "public_metadata",
                "private_metadata",
                "offline_access",
                "user:org:read",
            ],
        );
        assert_eq!(asked.as_deref(), Some("openid email profile offline_access"));
    }

    #[test]
    fn a_refresh_token_is_asked_for_whenever_the_server_can_issue_one() {
        assert_eq!(
            scopes(&["read"], &["read", "offline_access"]).as_deref(),
            Some("read offline_access")
        );
        assert_eq!(
            scopes(&["read", "offline_access"], &["read", "offline_access"]).as_deref(),
            Some("read offline_access"),
            "already asked for is not asked for twice"
        );
        assert_eq!(
            scopes(&["read"], &["read", "write"]).as_deref(),
            Some("read"),
            "a server that cannot issue a refresh token is not asked for one"
        );
    }

    #[test]
    fn a_public_client_never_sends_the_secret_it_was_given() {
        let public = Registered {
            client_id: "abc".into(),
            client_secret: Some("shh".into()),
            token_endpoint_auth_method: Some("none".into()),
        };
        let confidential = Registered {
            token_endpoint_auth_method: Some("client_secret_post".into()),
            ..public.clone()
        };
        assert_eq!(public.secret(), None);
        assert_eq!(confidential.secret().as_deref(), Some("shh"));
    }

    #[test]
    fn a_grant_is_renewed_before_it_expires_and_never_after() {
        let grant = |expires_at| Grant {
            access_token: "t".into(),
            refresh_token: None,
            expires_at,
            client_id: "c".into(),
            client_secret: None,
            token_endpoint: "https://x.test/t".into(),
        };
        let now = 1_000_000_000_000;
        assert!(!grant(Some(now + REFRESH_SKEW_MS + 1)).stale(now));
        assert!(grant(Some(now + REFRESH_SKEW_MS)).stale(now));
        assert!(grant(Some(now - 1)).stale(now));
        assert!(!grant(None).stale(now));
    }
}
