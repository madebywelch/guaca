//! Signing in to a plugin's MCP server, on behalf of a group.
//!
//! This is the second OAuth flow in the app and it is deliberately not the
//! first one. [`crate::subscription`] uses the device flow and argues against a
//! loopback redirect: a fixed port is one that something else may already own,
//! and a URL scheme is claimed by whichever build registered last. Both of
//! those objections are about a port or a scheme chosen in advance.
//!
//! Nothing here is chosen in advance. The listener binds `127.0.0.1:0`, the
//! operating system hands back a port that is free by construction, and *then*
//! the client is registered with that port in its redirect URI. Dynamic client
//! registration is what makes the ordering possible, and it is also why there
//! is no client id in this file: an MCP server issues one on the spot, so Guaca
//! does not have to be a registered application at every vendor on the list
//! before an operator can use it.
//!
//! The device flow is not available here anyway. None of the servers on the
//! list advertises it, and the MCP authorization spec mandates authorization
//! code with PKCE, so this is the flow or there is no flow.
//!
//! ## The order of the dance
//!
//! 1. Ask the resource who can authorize for it (RFC 9728).
//! 2. Ask that authorization server where its endpoints are (RFC 8414).
//! 3. Bind a loopback port.
//! 4. Register as a client that redirects to it (RFC 7591).
//! 5. Send the operator to the authorization endpoint with a PKCE challenge.
//! 6. Catch the redirect, check the state and the issuer, trade the code for a
//!    grant.
//!
//! Steps 1 and 2 are [`discover`], and every one of their fallbacks is a real
//! server rather than defensiveness. Stripe's authorization server is
//! `https://access.stripe.com/mcp`, and RFC 8414 says the well-known segment
//! goes *before* that path; Linear publishes its resource metadata under the
//! endpoint's path and Neon's under the bare one. Getting any of those wrong is
//! a plugin that cannot be connected at all, which is why `scripts/plugins.sh`
//! runs this same function against the live vendors instead of rebuilding the
//! URLs beside it.

use std::collections::HashMap;
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use reqwest::RequestBuilder;
use serde::Deserialize;
use sha2::{Digest, Sha256};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;

/// How long the operator has to finish in the browser.
///
/// Long enough to find a password and pick an account, short enough that a
/// forgotten tab does not leave a socket bound for the life of the app.
const WAIT_FOR_OPERATOR: Duration = Duration::from_secs(5 * 60);

/// Every other leg of the dance is Guaca talking to a server.
const HTTP_TIMEOUT: Duration = Duration::from_secs(30);

/// Refresh this far ahead of expiry rather than on rejection, for the reason
/// `subscription.rs` gives: a turn that discovers the token expired has already
/// spent the operator's wait on a call that was never going to work.
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
    #[error("{url} does not publish where to sign in ({detail}); this plugin cannot be connected")]
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
    /// The operator pressed Cancel, or the authorization server refused.
    #[error("the sign-in was refused: {error}{}", .description.as_deref().map(|d| format!(" — {d}")).unwrap_or_default())]
    Refused { error: String, description: Option<String> },
    /// The redirect did not match the request that started it. Treated as an
    /// attack rather than as a mistake, because that is the only thing it can
    /// be: nothing else can arrive on that port with the wrong state.
    #[error("the answer from your browser did not match the sign-in that was started")]
    StateMismatch,
    /// The redirect named a different authorization server from the one the
    /// sign-in was started against. RFC 9207, and the attack it exists for is a
    /// mix-up: a code minted by a server the operator does not use, presented
    /// to the one they do.
    #[error(
        "the answer came back naming {named}, and the sign-in was started at {expected}. \
         Nothing was connected."
    )]
    IssuerMismatch { expected: String, named: String },
    #[error("{issuer} issued a grant with no access token in it")]
    NoToken { issuer: String },
    #[error("this plugin has no refresh token, so its sign-in cannot be renewed")]
    NoRefreshToken,
}

/// Everything needed to use a grant, and to renew it.
///
/// Held in the store and never returned over IPC. The client registration is in
/// here with the tokens because a refresh needs it, and re-registering on every
/// refresh would leave a trail of abandoned clients at the vendor.
#[derive(Debug, Clone, PartialEq)]
pub struct Grant {
    pub access_token: String,
    pub refresh_token: Option<String>,
    /// Milliseconds since the epoch, when the server said. Absent means the
    /// server did not say, and a token with no stated expiry is used until it
    /// is refused.
    pub expires_at: Option<i64>,
    pub client_id: String,
    /// Issued by some servers even for a public client. Sent back only when the
    /// registration said the token endpoint expects it.
    pub client_secret: Option<String>,
    pub token_endpoint: String,
}

impl Grant {
    /// A key the operator pasted, as the one shape everything downstream reads.
    ///
    /// Not a second kind of credential and not a second column: it is stored,
    /// spent, hidden from the model and dropped on disconnect by exactly the
    /// code an issued token is. What it has none of is the machinery for
    /// renewing itself, and every one of those absences is the truth about a key
    /// somebody minted by hand — there is no refresh token, no stated expiry, no
    /// client and no token endpoint to present one to. A server that stops
    /// accepting it says so once and the operator pastes another.
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

    /// Whether this should be renewed before it is used again.
    pub fn stale(&self, now_ms: i64) -> bool {
        match self.expires_at {
            Some(at) => at - REFRESH_SKEW_MS <= now_ms,
            None => false,
        }
    }
}

/// Headers the operator gave the resource, and the rule about where they go.
///
/// A sign-in normally reaches several hosts: the resource publishes its
/// metadata, an authorization server publishes its own, and registration and
/// the token exchange happen at that server. Most of the time those are two
/// different origins.
///
/// The operator's headers belong to *their* server and nowhere else. A gate in
/// front of a self-hosted MCP server refuses everything on that host including
/// the metadata document a sign-in has to read first, so leaving them off makes
/// the composition — behind a gate, and signing in — fail at discovery with a
/// `403` nobody can act on. Sending them to every host in the dance instead
/// would hand the operator's gate credential to a vendor's authorization
/// server, which is a credential leak dressed up as a convenience.
///
/// So the rule is the origin: same origin as the resource, headers go on;
/// anywhere else, they do not. That covers the self-hosted server that is its
/// own issuer, where every request in the flow is behind the same gate, and it
/// covers a challenge that names metadata on somebody else's host, where none
/// of them are.
#[derive(Clone, Default)]
pub struct Gate<'a> {
    origin: String,
    headers: &'a [(String, String)],
}

impl<'a> Gate<'a> {
    /// Nothing to send, which is every catalog server and most added ones.
    pub fn none() -> Gate<'static> {
        Gate::default()
    }

    pub fn on(resource: &str, headers: &'a [(String, String)]) -> Gate<'a> {
        Gate {
            origin: split_origin(resource).map(|(origin, _)| origin).unwrap_or_default(),
            headers,
        }
    }

    /// This request, with the operator's headers on it if it is going to their
    /// own server.
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

/// Runs the whole flow and hands back a usable grant.
///
/// `open` is given the URL the operator has to visit. It is a callback rather
/// than a call into the opener plugin because this file has no business knowing
/// the app is a Tauri app, and because a test drives the flow without a browser
/// by answering the URL itself.
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

    // Opened before the client is registered, and that ordering is the whole
    // reason a loopback redirect is acceptable here: the port in the redirect
    // URI is one the operating system has already given us, so it cannot be
    // taken by something else between choosing it and listening on it. A
    // served landing has no port to race for; it is opened first anyway, so
    // the two orders are one order.
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
    // Asked for only when the server publishes what there is to ask for. A
    // scope Guaca invented is a scope the server refuses, and the refusal
    // arrives in the operator's browser rather than here.
    if let Some(scope) = scope {
        url.push_str(&format!("&scope={}", encode(&scope)));
    }

    open(&url).map_err(|detail| OauthError::NoBrowser { detail })?;

    // RFC 9207. The issuer is recorded beside the verifier and the state, and
    // the answer is checked against it before the code is spent: a redirect that
    // names a different issuer is a mix-up attack, where an authorization server
    // the operator does not use hands back a code this client would then present
    // to the one they do.
    let code = opened.wait(&state, &issuer).await?;
    exchange(&http, &server, &client, &code, &verifier, &redirect_uri, resource, gate, &now_ms)
        .await
}

// ---- where the browser lands ---------------------------------------------

/// The query a browser came back with, and a way to tell it what happened.
///
/// What the served route hands to the flow that is waiting. The page is the
/// flow's to choose, because only the flow knows whether the state matched and
/// the issuer was the one it sent the operator to; a route that answered
/// "Connected" on its own would say so to a mix-up.
pub struct Answer {
    pub fields: Vec<(String, String)>,
    pub reply: tokio::sync::oneshot::Sender<(u16, &'static str)>,
}

/// Every sign-in a served host is waiting on, by the `state` it sent.
///
/// One map for the whole daemon, held on the host and handed to every flow.
/// A callback naming a state nobody is waiting for is answered 404 and
/// forgotten: it is a stale tab, or somebody guessing.
pub type Callbacks = Arc<Mutex<HashMap<String, tokio::sync::mpsc::UnboundedSender<Answer>>>>;

/// Where a browser is sent back to after it authorizes, and how its answer
/// reaches the flow that is waiting for it.
///
/// Two, because the two hosts differ in exactly this. A desktop binds a
/// loopback port before naming the redirect, and the browser on the same
/// machine lands on it. A server has no browser on the machine and no port a
/// remote browser could reach, so the redirect is a route on the origin the
/// operator reached the workspace at, and the daemon forwards what arrives to
/// the flow by its state. Everything after the landing (PKCE, the state, the
/// issuer check, the exchange) is one path.
#[derive(Clone)]
pub enum Landing {
    Loopback,
    Served {
        /// Scheme and host the operator's browser reached the workspace at,
        /// with no trailing slash. The redirect URI is this plus the route.
        origin: String,
        callbacks: Callbacks,
    },
}

/// The route a served landing answers on. One string, read by the daemon's
/// router and by the redirect URI it registers, so they cannot disagree.
pub const CALLBACK_ROUTE: &str = "/v1/oauth/callback";

impl Landing {
    /// Opens the landing for one sign-in and names the redirect.
    ///
    /// Takes the state because a served landing is filed under it: that is
    /// how the daemon's route finds the flow a browser came back for.
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
                // Native OAuth clients allow varying ports on loopback IPs,
                // not the DNS name localhost (RFC 8252). Both reach the same
                // local backend; use the literal address in the callback.
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

    /// Hands a browser's answer to the flow waiting for it, and returns the
    /// page to show the browser. None when nobody is waiting on that state.
    ///
    /// Called by the daemon's route. The flow decides the page (see
    /// [`Answer`]) and this waits for it, so the browser is told what actually
    /// happened rather than what a route could guess.
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

/// One sign-in's landing, opened. Holds the port or the registration until
/// the answer arrives or the operator has taken too long.
pub struct Opened {
    pub redirect_uri: String,
    waiter: Waiter,
}

enum Waiter {
    Loopback(TcpListener),
    Served { receiver: tokio::sync::mpsc::UnboundedReceiver<Answer>, filed: Filed },
}

/// Takes the state back out of the map when the flow ends, however it ends.
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
    /// Waits for the browser to come back, and returns the code it carried.
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
                        Some(read) => {
                            let _ = reply.send(page_for(&read));
                            return read;
                        }
                        None => {
                            let _ = reply.send((404, NOT_THE_SIGNIN));
                        }
                    }
                }
            }
        }
    }
}

const NOT_THE_SIGNIN: &str = "Not the sign-in.";

/// What a browser is told, for what its answer turned out to be.
fn page_for(read: &Result<String, OauthError>) -> (u16, &'static str) {
    match read {
        Ok(_) => (200, "Connected. You can close this tab and go back to Guaca."),
        Err(OauthError::Refused { .. }) => (200, "Sign-in refused. You can close this tab."),
        Err(_) => (400, "That did not match. Nothing was connected."),
    }
}

/// Reads one redirect's query against the state and issuer that were sent.
///
/// `None` is a request that is not the sign-in at all: no code and no error,
/// which is a browser asking for `/favicon.ico` while it shows the page. The
/// loopback listener keeps waiting on those and the served waiter does too.
fn read_answer(
    fields: &[(String, String)],
    state: &str,
    issuer: &str,
) -> Option<Result<String, OauthError>> {
    // Before anything in the answer is read, error included. RFC 9207 is
    // explicit that a mismatched `iss` means the client must not act on or
    // display `error`, `error_description` or `error_uri` either: those are
    // attacker-controlled text on a mix-up, and showing them is how an
    // operator is talked into the next step. Compared byte for byte, with
    // no normalization: the spec forbids case folding, default-port elision
    // and trailing-slash tidying, because each one makes two different
    // issuers compare equal.
    let named = fields.iter().find(|(k, _)| k == "iss").map(|(_, v)| v.as_str());
    if let Some(named) = named {
        if named != issuer {
            return Some(Err(OauthError::IssuerMismatch {
                expected: issuer.to_string(),
                named: named.to_string(),
            }));
        }
    }

    if let Some(error) = fields.iter().find(|(k, _)| k == "error").map(|(_, v)| v.clone()) {
        let description =
            fields.iter().find(|(k, _)| k == "error_description").map(|(_, v)| v.clone());
        return Some(Err(OauthError::Refused { error, description }));
    }

    let code = fields.iter().find(|(k, _)| k == "code").map(|(_, v)| v.clone())?;

    // Checked before the page is written, so a mismatched redirect is never
    // told it succeeded.
    if fields.iter().find(|(k, _)| k == "state").map(|(_, v)| v.as_str()) != Some(state) {
        return Some(Err(OauthError::StateMismatch));
    }
    Some(Ok(code))
}

/// Trades a refresh token for a fresh grant.
///
/// Keeps the old refresh token when the server does not issue a new one, which
/// is legal and common: dropping it would make the next renewal impossible and
/// the plugin would quietly stop working a day later.
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

// ---- discovery -----------------------------------------------------------

/// What a server publishes about signing in to it, before anybody signs in.
#[derive(Debug, Clone)]
pub struct Discovered {
    pub issuer: String,
    pub server: ServerMetadata,
    /// What the *resource* said it wants asked for, which is not the same list
    /// as the authorization server's and takes precedence over it. See
    /// [`Discovered::requested_scope`].
    pub resource_scopes: Vec<String>,
    /// What the 401 itself named, which beats both.
    ///
    /// RFC 6750 puts a `scope` on the challenge, and the MCP authorization spec
    /// makes it the first thing a client should ask for: it is the server
    /// saying what *this* request needed, now, rather than what the resource
    /// needs in general. Empty when the challenge said nothing, which is every
    /// vendor on the list today.
    pub challenge_scopes: Vec<String>,
}

/// Steps 1 and 2 of the dance, on their own.
///
/// Split out because the live vendor test needs exactly this and nothing after
/// it: whether the five servers still publish what this build knows how to
/// read. A test that rebuilt the metadata URLs beside these ones would pass
/// while an operator could not connect, which is the only failure it exists to
/// catch.
pub async fn discover(
    resource: &str,
    challenge_header: Option<&str>,
) -> Result<Discovered, OauthError> {
    discover_through(resource, challenge_header, &Gate::none()).await
}

/// The same, for a resource that is behind something.
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

/// The `scope` a `WWW-Authenticate` challenge named, if it named one.
///
/// Space-separated inside quotes, per RFC 6750. Read from the challenge rather
/// than from any document, because it is the only one of the three lists that
/// knows which request was refused.
fn challenge_scope(challenge: &str) -> Vec<String> {
    let Some(at) = challenge.find("scope=") else { return Vec::new() };
    // `resource_metadata=` and `error=` also end in `scope=`-free text, but a
    // parameter named `...scope` would match this: only a value at a parameter
    // boundary counts.
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

/// RFC 9728 protected-resource metadata, as far as this build reads it.
struct Resource {
    /// Who can issue a grant for it.
    issuer: String,
    /// What it says to ask for. Empty when it does not say.
    scopes: Vec<String>,
}

/// What the resource publishes about itself.
///
/// The challenge on a 401 is consulted first because it is the answer the
/// server itself just gave; the well-known paths are what to try when a server
/// refuses without saying where to go, which is most of them.
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

/// RFC 8414 authorization-server metadata, as far as this build reads it.
#[derive(Debug, Clone, Deserialize)]
pub struct ServerMetadata {
    /// The issuer identifier, which is the only value RFC 9207's `iss` check
    /// may be made against.
    ///
    /// Not the address the document was fetched from. Those two are the same
    /// string only for an authorization server at the root of an origin, and
    /// `guaca.bot` mounts its own under `/api/auth`: a sign-in checked against
    /// the origin reached the consent screen, was issued a code, and was then
    /// refused at the redirect for naming the issuer the service publishes. RFC
    /// 8414 section 3.3 wants the two to agree, and where they do not, the
    /// published value wins, because it is also the one the server will send.
    ///
    /// `default` because the plugin flow takes its issuer from the *resource's*
    /// metadata, where RFC 9728 has already named it, and never reads this
    /// field. A vendor that omits a required one must not become a plugin
    /// nobody can connect.
    #[serde(default)]
    pub issuer: String,
    pub authorization_endpoint: String,
    pub token_endpoint: String,
    /// Absent is a server Guaca cannot sign in to at all: with no RFC 7591
    /// endpoint there is nothing to register a loopback redirect against.
    #[serde(default)]
    pub registration_endpoint: Option<String>,
    #[serde(default)]
    pub scopes_supported: Vec<String>,
    /// S256 is the only challenge this build sends, so a server that stops
    /// naming it is one every sign-in would be refused by.
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

/// The metadata addresses to try for one URL, in the order RFC 8414 gives.
///
/// An issuer with a path has the well-known segment inserted *before* it, which
/// is the part everybody gets wrong; the suffixed form is tried first and the
/// bare form second because a server with no path publishes only the second and
/// a server with one may publish either.
fn well_known(url: &str, name: &str) -> Vec<String> {
    let Some((origin, path)) = split_origin(url) else {
        return Vec::new();
    };
    let path = path.trim_end_matches('/');
    let mut out = Vec::new();
    if !path.is_empty() {
        out.push(format!("{origin}/.well-known/{name}{path}"));
        // The other reading of the same rule, which some servers implement.
        out.push(format!("{origin}{path}/.well-known/{name}"));
    }
    out.push(format!("{origin}/.well-known/{name}"));
    out
}

/// `https://host:port` and the path after it, without pulling in a URL crate
/// for two `find` calls.
pub(crate) fn split_origin(url: &str) -> Option<(String, String)> {
    let (scheme, rest) = url.split_once("://")?;
    match rest.find('/') {
        Some(at) => Some((format!("{scheme}://{}", &rest[..at]), rest[at..].to_string())),
        None => Some((format!("{scheme}://{rest}"), String::new())),
    }
}

/// The `resource_metadata` address out of a `WWW-Authenticate` challenge.
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
    /// What to ask for, when something publishes a list.
    ///
    /// Three places publish one and they are not the same list, so they are
    /// tried in the order of how much each one knows about the request that was
    /// refused. The challenge (RFC 6750, on the 401) is the server saying what
    /// *this* request needed and beats everything. The resource's (RFC 9728) is
    /// the scopes used to request access to that resource in general. The
    /// authorization server's (RFC 8414) is everything the issuer can grant,
    /// across every resource behind it and for clients it created by hand as
    /// well as ones that registered themselves, which is why it is last: it is
    /// the only one of the three that is not about this server at all.
    ///
    /// AgentMail is why, and it was found by an operator rather than by a test.
    /// Its MCP server names `openid email profile`; the Clerk instance behind
    /// it also lists `public_metadata`, `private_metadata` and `user:org:read`,
    /// and refuses a registered client that asks for them: `invalid_scope`, on
    /// a vendor's error page, where nothing here can explain it.
    ///
    /// `offline_access` is the one addition, and only when the authorization
    /// server names it. It is not access to anything: it is the scope that
    /// decides whether a refresh token comes back, and without one a plugin
    /// works until the access token expires and then asks to be signed in
    /// again, every hour, for as long as it stays connected.
    ///
    /// `*` is filtered out wherever it appears: an operator connecting a
    /// database plugin has not agreed to hand over everything their account can
    /// do, and a server that offers a wildcard also offers the named scopes
    /// that add up to the part Guaca needs.
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

// ---- registration --------------------------------------------------------

#[derive(Debug, Clone, Deserialize)]
struct Registered {
    client_id: String,
    #[serde(default)]
    client_secret: Option<String>,
    /// What this registration says the token endpoint expects. A server may
    /// issue a secret and still register the client as public, and sending the
    /// secret then is what gets the exchange rejected.
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

// ---- the redirect --------------------------------------------------------

/// Waits for the browser to come back, and answers it with a page.
///
/// Anything that is not the callback is answered 404 and the listener keeps
/// waiting: a browser asks for `/favicon.ico` while it is showing the page, and
/// taking the first connection as the answer would abandon the sign-in the
/// moment the operator's browser was being thorough.
pub(crate) async fn wait_for_redirect(
    listener: TcpListener,
    state: &str,
    issuer: &str,
) -> Result<String, OauthError> {
    let deadline = tokio::time::sleep(WAIT_FOR_OPERATOR);
    tokio::pin!(deadline);

    loop {
        let accepted = tokio::select! {
            _ = &mut deadline => return Err(OauthError::TimedOut),
            accepted = listener.accept() => accepted,
        };
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
            Some(read) => {
                let (status, page) = page_for(&read);
                let _ = reply(&mut socket, status, page).await;
                return read;
            }
            None => {
                let _ = reply(&mut socket, 404, NOT_THE_SIGNIN).await;
                continue;
            }
        }
    }
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

// ---- the exchange --------------------------------------------------------

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
        // RFC 8707. Without it a server that issues audience-bound tokens has
        // to guess which resource this grant is for, and some refuse instead.
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

// ---- the small things ----------------------------------------------------

pub(crate) fn http() -> Result<reqwest::Client, OauthError> {
    reqwest::Client::builder().build().map_err(|source| OauthError::Transport {
        what: "to build a client",
        url: String::new(),
        source,
    })
}

/// 256 bits of randomness, base64url.
///
/// Two v4 UUIDs rather than a random-number crate: both are drawn from the
/// operating system's generator, and this is the only place in the app that
/// needs unguessable bytes that are not already an id.
pub(crate) fn secret() -> String {
    let mut bytes = [0u8; 32];
    bytes[..16].copy_from_slice(uuid::Uuid::new_v4().as_bytes());
    bytes[16..].copy_from_slice(uuid::Uuid::new_v4().as_bytes());
    base64url(&bytes)
}

pub(crate) fn pkce_challenge(verifier: &str) -> String {
    base64url(&Sha256::digest(verifier.as_bytes()))
}

/// Base64url without padding, which is what every one of these fields wants.
///
/// Not shared with `e2b::encode`: that one is standard base64 with padding, for
/// a shell, and the two alphabets differ in exactly the characters that a URL
/// treats specially. One encoder with a flag would be one flag away from a
/// PKCE challenge that no server accepts.
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

/// Percent-encoding for a query parameter: everything but the unreserved set.
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

/// The inverse, for what comes back on the redirect. `+` is a space here
/// because that is what a form-encoded query means by it, and an authorization
/// code containing one would otherwise be exchanged with a plus in it.
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
        // A trailing slash on the origin is not two slashes on the redirect.
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
        // And the browser was told what happened, by the flow rather than the route.
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
        // Without a state at all it cannot even be filed.
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
        // RFC 7636 appendix B, which is the one way to know the hash, the
        // alphabet and the missing padding are all right at once.
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
        // RFC 7636 wants 43 to 128 characters, and a short one is refused by
        // the server rather than here.
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
        // Query strings are form-encoded on the way back. An authorization code
        // is opaque, so a wrong reading here fails at the token endpoint with
        // "invalid grant" and nothing says why.
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
        // The part of RFC 8414 that is most often got wrong, and the reason
        // Stripe's authorization server is found at all: its issuer is
        // `https://access.stripe.com/mcp`, and the first form below is the only
        // one of the three that answers.
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

    /// The same, with what the 401 itself named on the front.
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
        // The MCP authorization spec's own priority order, and the reason for
        // it: a challenge is the server saying what *this* request needed,
        // where both documents say what the resource needs in general. Asking
        // for less than the challenge named is a second 401 on the same call.
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
        // The common case, and every vendor on the list today: a challenge that
        // names the metadata and nothing else. Reading a scope out of one of
        // these is how Guaca would come to invent one.
        assert!(challenge_scope(
            r#"Bearer resource_metadata="https://x.test/.well-known/oauth-protected-resource""#
        )
        .is_empty());
        assert!(challenge_scope("Bearer").is_empty());
        // Not a parameter that merely ends in the right six characters.
        assert!(challenge_scope(r#"Bearer myscope="nope""#).is_empty());
    }

    #[test]
    fn a_wildcard_scope_is_never_asked_for() {
        // Neon offers `read`, `write` and `*`, and publishes no list of its
        // own on the resource. Connecting a database plugin is not consent to
        // everything the operator's account can do.
        assert_eq!(scopes(&[], &["read", "write", "*"]).as_deref(), Some("read write"));
    }

    #[test]
    fn a_server_that_publishes_no_scopes_is_not_asked_for_one() {
        // Cloudflare's does not. An invented scope is refused in the browser,
        // where nothing can explain it.
        assert_eq!(scopes(&[], &[]), None);
    }

    #[test]
    fn the_resource_decides_what_is_asked_for_and_not_its_authorization_server() {
        // AgentMail, exactly as the two documents read today. Its MCP server
        // wants three scopes; the Clerk instance behind it can issue seven and
        // refuses a registered client that asks for the other four, with an
        // `invalid_scope` the operator sees and cannot act on.
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
        // `offline_access` is not access to anything: it is the difference
        // between a plugin that renews itself and one that asks the operator to
        // sign in again every hour. Added once, and never invented.
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
        // Neon issues one and registers the client as public in the same
        // answer. Sending it then is what gets the exchange rejected.
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
        // A server that states no expiry is taken at its word. Guessing one
        // would refresh a working token on a timer nobody chose.
        assert!(!grant(None).stale(now));
    }
}
