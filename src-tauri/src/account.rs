//! Signing in to a Guaca account, which nobody has to do.
//!
//! Guaca runs on this machine with the operator's own keys and stays that way.
//! An account is one optional thing: a hosted OAuth client, so an agent can
//! reach a service that will only issue programmatic access to a registered
//! application. Gmail is the example. Guaca cannot be that application, because
//! its client secret would be in the download, and no amount of local
//! cleverness changes that. `guaca.bot` can be, and holds the refresh token so
//! this machine never has to.
//!
//! An install that never signs in never talks to it. Everything else in the app
//! works exactly the same either way, and that is a property to keep rather than
//! a phase to grow out of.
//!
//! ## Why this is the third OAuth flow and not a copy of the first
//!
//! [`crate::subscription`] uses the device flow and argues for it: a fixed
//! loopback port may already belong to something else, and a URL scheme is
//! claimed by whichever build registered last. [`crate::oauth`] answers both
//! objections for MCP plugins by binding `127.0.0.1:0` *before* naming the
//! redirect, so the port is one the operating system has already handed out.
//! This module is that second answer pointed at one known server.
//!
//! The choice matters more here than anywhere else in the app. A device code is
//! a bearer secret carried by a human, and RFC 8628 section 5.4 says what
//! follows: nothing binds the code to the machine that asked for it, so anyone
//! who can talk an operator into approving one walks away with a token that
//! mints Gmail access tokens on that operator's account. RFC 8252 is the
//! standing advice for a native application with a browser, and this is it: an
//! authorization code, bound to this process by a PKCE verifier that never
//! leaves it, delivered to a port on this machine. A code lifted off the wire is
//! worth nothing without the verifier, and a code delivered to `127.0.0.1` on
//! somebody else's machine never reaches this one.
//!
//! ## Where the service is, and where it is not
//!
//! [`DEFAULT_ORIGIN`], a constant, overridable only through the environment and
//! never through settings. The reason is `subscription.rs`'s: a mistyped sign-in
//! service is a credential sent somewhere nobody chose. The override exists so
//! the flow can be run end to end against a Worker on this machine, and it is
//! refused for anything that is neither HTTPS nor loopback.
//!
//! ## What is stored, and where
//!
//! Its own file, for the reason `subscription.rs` gives: the token set rotates
//! on refresh, which is Guaca writing in the background, and `config.json` is
//! rewritten wholesale whenever the operator presses Save. Two writers on one
//! file lose a refreshed token to a stale in-memory copy.
//!
//! Plaintext and 0600, and the same caveat applies with the same force. This
//! credential is not Guaca's to lose: it stands for an account that can mint
//! access tokens for the operator's mail. The honest fix is the OS keychain, and
//! it is the first thing to reach for if this file grows a second reader.

use std::fs;
use std::io;
use std::path::PathBuf;
use std::time::Duration;

use parking_lot::Mutex;
use serde::{Deserialize, Serialize};
use tokio::net::TcpListener;

use crate::oauth::{self, OauthError};

/// The service, and the only one a build ships pointed at.
pub const DEFAULT_ORIGIN: &str = "https://guaca.bot";

/// What the app is registered as there.
///
/// Not a secret and not capable of being one: this is a public client, and PKCE
/// is what proves a code belongs to the copy of Guaca that asked for it. The
/// same string is a row in the service's own migration, which is what keeps the
/// consent screen naming an application the service chose rather than one a
/// stranger registered.
pub const CLIENT_ID: &str = "guaca-desktop";

/// Everything the app asks for, which is everything it has a use for.
///
/// `openid` is what lets the service turn this token back into a person on its
/// own endpoints, `email` is so the operator can see which account they linked,
/// `offline_access` is the refresh token, and `connectors` is the one that does
/// anything. Asking for less would mean asking again later, which is another
/// consent screen for a person who has already said yes.
pub const SCOPE: &str = "openid email offline_access connectors";

/// Refresh this far ahead of expiry rather than on rejection, for the reason
/// `subscription.rs` gives: a call that discovers the token expired has already
/// spent the wait on a round trip that was never going to work.
const REFRESH_SKEW_MS: i64 = 5 * 60 * 1000;

/// Long enough for a slow exchange, short enough that a hung service does not
/// park the dialog forever.
const HTTP_TIMEOUT: Duration = Duration::from_secs(30);

#[derive(Debug, thiserror::Error)]
pub enum AccountError {
    #[error(
        "{origin} is not a sign-in service Guaca will use. It has to be HTTPS, or a loopback \
         address for local development."
    )]
    UnsafeOrigin { origin: String },
    #[error(
        "{origin} does not publish where to sign in, so Guaca cannot start. Check the address, or \
         that the service is running."
    )]
    NoMetadata { origin: String },
    #[error(
        "{origin} published a sign-in address at {endpoint}, which is somewhere else. Guaca will \
         not send a credential there."
    )]
    ForeignEndpoint { origin: String, endpoint: String },
    #[error("could not listen for the answer from your browser: {source}")]
    NoPort {
        #[source]
        source: io::Error,
    },
    #[error("could not open your browser: {detail}")]
    NoBrowser { detail: String },
    #[error("nothing came back from your browser within five minutes; the sign-in was abandoned")]
    TimedOut,
    #[error("the sign-in was refused: {error}{}", .description.as_deref().map(|d| format!(" — {d}")).unwrap_or_default())]
    Refused { error: String, description: Option<String> },
    #[error("the answer from your browser did not match the sign-in that was started")]
    StateMismatch,
    /// RFC 9207: the redirect named an authorization server other than the one
    /// the sign-in was started against.
    ///
    /// A different sentence from [`OauthError::IssuerMismatch`] on purpose. A
    /// plugin's issuer is discovered per sign-in, so a mismatch there is an
    /// answer arriving from somewhere unexpected. Here the expected value is
    /// one the service itself published moments earlier, so a mismatch means
    /// the service is contradicting its own metadata, and saying so is the
    /// difference between an operator retrying forever and an operator
    /// reporting it.
    #[error(
        "{expected} was published as the sign-in service and the answer came back naming \
         {named}. Nothing was connected, and signing in again will not help until those two \
         agree."
    )]
    IssuerMismatch { expected: String, named: String },
    #[error("{origin} answered HTTP {status}: {message}")]
    Upstream { origin: String, status: u16, message: String },
    #[error("could not reach {origin}: {detail}")]
    Transport { origin: String, detail: String },
    /// An [`OauthError`] this flow has no way to produce.
    ///
    /// It exists so the conversion below can be exhaustive, which is the whole
    /// point: the catch-all it replaced put "could not reach" in front of every
    /// answer that arrived and disagreed, with an empty origin where the
    /// service's name should have been. A new variant in `oauth.rs` is now a
    /// compile error here rather than a sentence about the network.
    #[error(
        "the sign-in did something this build has no answer for: {detail}. That is a bug in Guaca \
         rather than something to retry."
    )]
    Unexpected { detail: String },
    #[error("not signed in to a Guaca account")]
    NotSignedIn,
    #[error("the sign-in to {origin} has expired. Sign in again.")]
    Expired { origin: String },
    #[error("could not save the sign-in to {path}: {source}")]
    Io {
        path: PathBuf,
        #[source]
        source: io::Error,
    },
}

/// Exhaustive rather than defensive.
///
/// [`OauthError`] describes a plugin sign-in as well, and that flow registers a
/// client and renews its own grant where this one does neither, so three of its
/// variants cannot arrive here. Naming them anyway is what stopped the ones
/// that *can* arrive from being folded into `Transport`: an `IssuerMismatch` is
/// an answer that came back and disagreed, and reporting it as a service that
/// could not be reached sends the operator to check their network.
impl From<OauthError> for AccountError {
    fn from(err: OauthError) -> Self {
        match err {
            OauthError::NoPort { source } => AccountError::NoPort { source },
            OauthError::NoBrowser { detail } => AccountError::NoBrowser { detail },
            OauthError::TimedOut => AccountError::TimedOut,
            OauthError::Refused { error, description } => {
                AccountError::Refused { error, description }
            }
            OauthError::StateMismatch => AccountError::StateMismatch,
            OauthError::IssuerMismatch { expected, named } => {
                AccountError::IssuerMismatch { expected, named }
            }
            OauthError::NoMetadata { url, .. } => AccountError::NoMetadata { origin: url },
            OauthError::Status { url, status, body, .. } => {
                AccountError::Upstream { origin: url, status, message: body }
            }
            OauthError::Transport { url, source, .. } => {
                AccountError::Transport { origin: url, detail: source.to_string() }
            }
            // The three the plugin flow owns. `NoRegistration` and
            // `NoRefreshToken` come from `oauth::connect` and `oauth::refresh`,
            // which this module does not call; `NoToken` is raised by the one
            // place that reads a grant rather than by `post_token`.
            err @ (OauthError::NoRegistration { .. }
            | OauthError::NoToken { .. }
            | OauthError::NoRefreshToken) => AccountError::Unexpected { detail: err.to_string() },
        }
    }
}

/// The token set, as stored. Never crosses IPC: see [`Status`].
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
struct Stored {
    access_token: String,
    /// Absent means a service that issued no refresh token, which is a sign-in
    /// that ends when the access token does rather than one that is broken.
    #[serde(default)]
    refresh_token: Option<String>,
    /// Milliseconds since the epoch. Absent means the service did not say, and
    /// a token with no stated expiry is used until it is refused.
    #[serde(default)]
    expires_at: Option<i64>,
    /// Denormalized so a signed-in status can be answered without a round trip.
    #[serde(default)]
    email: String,
    /// Kept with the tokens because a refresh needs it, and rediscovering the
    /// endpoint on every refresh is a second round trip in front of every call.
    token_endpoint: String,
    /// Which service issued this, so a build pointed somewhere else does not
    /// read it as its own.
    ///
    /// One profile per bundle identifier, so the development build and the real
    /// one share this file. Without this field, signing in to a Worker on this
    /// machine and then opening the real app presents a localhost token to
    /// `guaca.bot`, which refuses it: an operator sees a sign-in that used to
    /// work and now reports itself expired, with nothing on screen to say why.
    /// Absent means a file written before this field existed, which can only
    /// have been the default service.
    #[serde(default)]
    origin: String,
}

impl Stored {
    /// Close enough to expiry that the next call should not be given it.
    ///
    /// A method rather than a line at the top of `access`, because `access`
    /// asks it twice: once before queueing for a refresh and once after, and
    /// two copies of this that drifted apart would be a refresh nobody waits
    /// for.
    fn expiring(&self) -> bool {
        self.expires_at.is_some_and(|at| at - REFRESH_SKEW_MS <= now_ms())
    }
}

/// What the webview is allowed to know about an account.
///
/// No token, and no field one could arrive in. The origin is here because in
/// development it is not `guaca.bot`, and an operator who cannot see which
/// service they linked to cannot tell the two apart.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Status {
    pub signed_in: bool,
    pub email: String,
    pub origin: String,
}

/// One thing an authorized provider can do, as the service describes it.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Capability {
    pub id: String,
    pub label: String,
    pub granted: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Provider {
    pub id: String,
    pub label: String,
    pub capabilities: Vec<Capability>,
}

/// What the account holds, as the service reports it.
///
/// Read rather than kept. The authoritative answer is the service's, it changes
/// when the operator authorizes something in a browser rather than when this
/// app does anything, and a stale local copy would be a list of capabilities an
/// agent is told it has and does not.
/// One identity the operator has authorized at a provider.
///
/// A person can authorize the same provider twice — a work Google and a
/// personal one — and each is a separate grant with its own id. A group binds
/// to one of these, which is what lets two crews use two mailboxes.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct AccountConnection {
    pub id: String,
    pub provider: String,
    /// The provider's own name for it, which is how an operator tells two
    /// apart. An email where the provider says, its subject where it does not.
    pub label: String,
    pub capabilities: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Connectors {
    pub email: String,
    pub providers: Vec<Provider>,
    /// Every authorized identity, newest last. Empty on a service that has none
    /// and on one too old to report them, which reads the same to a caller:
    /// there is nothing to choose between.
    #[serde(default)]
    pub connections: Vec<AccountConnection>,
}

/// A signed-in Guaca account, and the file it lives in.
///
/// Cheap to clone behind the `Arc` its owner holds. The mutex is held only to
/// read or replace the token set, never across a request.
#[derive(Debug)]
pub struct Account {
    path: PathBuf,
    origin: String,
    http: reqwest::Client,
    tokens: Mutex<Option<Stored>>,
    /// Held across a refresh, so a crew that all reach for the account at once
    /// spends one.
    ///
    /// The same lock `Subscription` holds, for the same reason and against the
    /// same service behavior: the refresh token rotates and the presented one
    /// is revoked, so two callers renewing together race to retire each other's.
    /// The loser presents a token the service has just thrown away, and gets an
    /// `invalid_grant` that reads nothing like what happened. Whoever gets in
    /// first refreshes; everyone behind them finds the new token already stored
    /// and takes it. Async rather than `parking_lot` because it is held across
    /// an await, which is the one thing the other lock here is careful never to
    /// be.
    renewing: tokio::sync::Mutex<()>,
}

impl Account {
    /// Opens the store at `path`, against the service this build ships with.
    pub fn open(path: PathBuf) -> Self {
        Self::open_at(path, DEFAULT_ORIGIN)
    }

    /// The same, against a named service.
    ///
    /// The seam for the test suite and for running the real flow against a
    /// Worker on this machine. Deliberately not reachable from settings: see the
    /// module comment.
    pub fn open_at(path: PathBuf, origin: impl Into<String>) -> Self {
        let origin = origin.into().trim_end_matches('/').to_string();
        let tokens = match fs::read_to_string(&path) {
            Ok(raw) => match serde_json::from_str::<Stored>(&raw) {
                Ok(stored) if stored.access_token.is_empty() => None,
                Ok(stored) if !belongs_to(&stored, &origin) => {
                    tracing::info!(
                        stored = %stored.origin,
                        configured = %origin,
                        "ignoring an account signed in to a different service"
                    );
                    None
                }
                Ok(stored) => Some(stored),
                Err(err) => {
                    tracing::warn!(%err, path = %path.display(), "ignoring an unreadable account file");
                    None
                }
            },
            Err(err) if err.kind() == io::ErrorKind::NotFound => None,
            Err(err) => {
                tracing::warn!(%err, path = %path.display(), "could not read the account file");
                None
            }
        };

        Self {
            path,
            origin,
            http: reqwest::Client::builder().timeout(HTTP_TIMEOUT).build().unwrap_or_default(),
            tokens: Mutex::new(tokens),
            renewing: tokio::sync::Mutex::new(()),
        }
    }

    pub fn origin(&self) -> &str {
        &self.origin
    }

    pub fn status(&self) -> Status {
        match &*self.tokens.lock() {
            Some(stored) => {
                Status { signed_in: true, email: stored.email.clone(), origin: self.origin.clone() }
            }
            None => Status { signed_in: false, email: String::new(), origin: self.origin.clone() },
        }
    }

    pub fn is_signed_in(&self) -> bool {
        self.tokens.lock().is_some()
    }

    /// The whole sign-in, from discovery to a stored token.
    ///
    /// Long-running by design: it returns when the operator has finished in the
    /// browser, refused, or taken longer than five minutes. The caller is an IPC
    /// command the dialog awaits, so abandoning it costs one parked call and
    /// leaves nothing behind but a closed socket.
    pub async fn sign_in(
        &self,
        open: impl FnOnce(&str) -> Result<(), String>,
    ) -> Result<Status, AccountError> {
        let server = self.discover().await?;

        // Bound before the redirect URI is built, which is the whole reason a
        // loopback redirect is safe: the port is one the operating system has
        // already given out, so nothing can take it between choosing it and
        // listening on it. The service registered this redirect with no port at
        // all, because RFC 8252 section 7.3 has it compare everything but.
        let listener = TcpListener::bind("127.0.0.1:0")
            .await
            .map_err(|source| AccountError::NoPort { source })?;
        let port = listener.local_addr().map_err(|source| AccountError::NoPort { source })?.port();
        let redirect_uri = format!("http://127.0.0.1:{port}/callback");

        let verifier = oauth::secret();
        let challenge = oauth::pkce_challenge(&verifier);
        let state = oauth::secret();

        let url = format!(
            "{}?response_type=code&client_id={}&redirect_uri={}&scope={}&state={}\
             &code_challenge={}&code_challenge_method=S256",
            server.authorization_endpoint,
            oauth::encode(CLIENT_ID),
            oauth::encode(&redirect_uri),
            oauth::encode(SCOPE),
            oauth::encode(&state),
            oauth::encode(&challenge),
        );

        open(&url).map_err(|detail| AccountError::NoBrowser { detail })?;

        // RFC 9207. The value compared against is the issuer the service
        // published and nothing else: see `ServerMetadata::issuer`, which is
        // where the origin used to be substituted for it. Read out of the
        // document before the browser opens rather than out of the answer,
        // because the check is only worth anything against a value recorded in
        // advance.
        let code = oauth::wait_for_redirect(listener, &state, &server.issuer).await?;

        let issued = oauth::post_token(
            &oauth::http()?,
            &server.token_endpoint,
            &[
                ("grant_type".to_string(), "authorization_code".to_string()),
                ("code".to_string(), code),
                ("redirect_uri".to_string(), redirect_uri),
                ("client_id".to_string(), CLIENT_ID.to_string()),
                ("code_verifier".to_string(), verifier),
            ],
            &oauth::Gate::none(),
        )
        .await?;

        let stored = Stored {
            access_token: issued.access_token,
            refresh_token: issued.refresh_token,
            expires_at: issued.expires_in.map(|secs| now_ms() + secs * 1000),
            // Filled in below. The service is the only thing that knows which
            // account this turned out to be.
            email: String::new(),
            token_endpoint: server.token_endpoint,
            origin: self.origin.clone(),
        };

        // Spending the token once, before anything is written, is what makes a
        // sign-in that reports success one that actually works. It is also where
        // the email comes from, so there is no second call to make.
        let held = self.fetch_connectors(&stored.access_token).await?;
        self.store(Stored { email: held.email, ..stored })
    }

    /// Forgets the sign-in on this machine.
    ///
    /// Local only. Revoking centrally would be the service's business and would
    /// end sign-ins on the operator's other machines, which this one does not
    /// own. What it does end here is immediate: the file goes, and the next call
    /// has nothing to present.
    pub fn sign_out(&self) -> Result<(), AccountError> {
        *self.tokens.lock() = None;
        match fs::remove_file(&self.path) {
            Ok(()) => Ok(()),
            Err(err) if err.kind() == io::ErrorKind::NotFound => Ok(()),
            Err(source) => Err(AccountError::Io { path: self.path.clone(), source }),
        }
    }

    /// What the account holds right now, asked of the service.
    pub async fn connectors(&self) -> Result<Connectors, AccountError> {
        let token = self.access().await?;
        self.fetch_connectors(&token).await
    }

    /// A token that is good right now.
    ///
    /// Refreshes when the stored one is close enough to expiry to be a risk, so
    /// every caller is spared deciding. A refresh the network refused leaves the
    /// stored token in place: it may still have minutes left, and a call that
    /// could have worked is worth more than a tidy cache. A refusal is different
    /// and surfaces, and it surfaces as itself: this is signed in and could not
    /// renew, which is not the same fact as having no account and must not
    /// reach an operator or an agent wearing that one's words.
    pub async fn access(&self) -> Result<String, AccountError> {
        let stored = self.tokens.lock().clone().ok_or(AccountError::NotSignedIn)?;
        if !stored.expiring() {
            return Ok(stored.access_token);
        }

        // Everything past here is one at a time, and the fast path above is
        // deliberately not: a token with an hour left on it must never queue
        // behind somebody else's refresh.
        let _gate = self.renewing.lock().await;

        // Read again, because the wait is the whole point. Whoever held the
        // gate has already stored a fresh token, and taking theirs is what
        // makes this a shared refresh rather than a second one presenting a
        // refresh token they have just retired.
        let stored = self.tokens.lock().clone().ok_or(AccountError::NotSignedIn)?;
        if !stored.expiring() {
            return Ok(stored.access_token);
        }

        let Some(refresh_token) = stored.refresh_token.clone() else {
            // No way to renew, so the token stands until it is refused. Saying
            // so here would sign the operator out of a sign-in that still works.
            return Ok(stored.access_token);
        };

        match self.refresh(&stored, &refresh_token).await {
            Ok(fresh) => Ok(fresh),
            Err(err @ AccountError::Transport { .. }) => {
                tracing::warn!(
                    %err,
                    "using the stored account token: its refresh could not be reached"
                );
                Ok(stored.access_token)
            }
            // Said out loud here because this is the only place that has it.
            // What the service answered is the difference between a sign-in to
            // redo and a bad ten seconds at the token endpoint, and every
            // caller used to drop it: the operator got "not signed in" about an
            // account that was, and nothing anywhere recorded the status.
            Err(err) => {
                tracing::warn!(%err, origin = %self.origin, "the account refused to renew");
                Err(err)
            }
        }
    }

    async fn refresh(&self, stored: &Stored, refresh_token: &str) -> Result<String, AccountError> {
        let issued = oauth::post_token(
            &oauth::http()?,
            &stored.token_endpoint,
            &[
                ("grant_type".to_string(), "refresh_token".to_string()),
                ("refresh_token".to_string(), refresh_token.to_string()),
                ("client_id".to_string(), CLIENT_ID.to_string()),
            ],
            &oauth::Gate::none(),
        )
        .await?;

        let access_token = issued.access_token.clone();
        self.store(Stored {
            access_token: issued.access_token,
            // A rotation that issues a new refresh token says so; one that does
            // not expects the old one to keep working. Storing nothing here
            // would end the sign-in at the next renewal.
            refresh_token: issued.refresh_token.or_else(|| Some(refresh_token.to_string())),
            expires_at: issued.expires_in.map(|secs| now_ms() + secs * 1000),
            email: stored.email.clone(),
            token_endpoint: stored.token_endpoint.clone(),
            origin: self.origin.clone(),
        })?;
        Ok(access_token)
    }

    /// Where the service says to sign in, read from the service.
    ///
    /// RFC 8414, at the root of the origin. Guaca could hard-code the paths and
    /// save a round trip, and then a service that moved one would be a sign-in
    /// nobody could complete until every copy of the app was updated.
    ///
    /// Everything in the document is checked to be on the origin that published
    /// it, the issuer included. That is not defensiveness about a document Guaca
    /// fetched over TLS from the one place it trusts: it is the difference
    /// between a service that can change its own paths and a service that can
    /// redirect an operator's credential to a third party, and the check costs
    /// three string comparisons.
    ///
    /// The issuer is filled in here when the service published none, so that
    /// [`Self::sign_in`] has exactly one value to read and no reason to reach
    /// for the origin itself. The origin is what an absent issuer means rather
    /// than a guess: this document was fetched from the root well-known address,
    /// which under RFC 8414 section 3.3 is the address of an issuer with no
    /// path. It is also the reading the field's absence used to be given
    /// unconditionally, and `guaca.bot` is the service that showed why that is
    /// not the same thing.
    async fn discover(&self) -> Result<oauth::ServerMetadata, AccountError> {
        if !self.is_safe_origin() {
            return Err(AccountError::UnsafeOrigin { origin: self.origin.clone() });
        }

        let mut server =
            oauth::server_metadata(&self.http, &self.origin, &oauth::Gate::none()).await?;
        if server.issuer.is_empty() {
            server.issuer = self.origin.clone();
        }

        for endpoint in [&server.authorization_endpoint, &server.token_endpoint, &server.issuer] {
            if !self.is_same_origin(endpoint) {
                return Err(AccountError::ForeignEndpoint {
                    origin: self.origin.clone(),
                    endpoint: endpoint.clone(),
                });
            }
        }
        Ok(server)
    }

    /// HTTPS, or a loopback address for local development.
    ///
    /// The one thing an environment variable must not be able to do is send a
    /// credential over plaintext to somewhere on the network. Loopback is
    /// exempt because it does not cross one.
    fn is_safe_origin(&self) -> bool {
        if self.origin.starts_with("https://") {
            return true;
        }
        let Some(rest) = self.origin.strip_prefix("http://") else {
            return false;
        };
        let host = rest.split('/').next().unwrap_or("").rsplit_once(':').map_or(rest, |(h, _)| h);
        matches!(host, "localhost" | "127.0.0.1" | "[::1]" | "::1")
    }

    fn is_same_origin(&self, url: &str) -> bool {
        oauth::split_origin(url).is_some_and(|(origin, _)| origin == self.origin)
    }

    async fn fetch_connectors(&self, token: &str) -> Result<Connectors, AccountError> {
        #[derive(Deserialize)]
        struct Body {
            user: User,
            providers: Vec<Provider>,
            #[serde(default)]
            connections: Vec<AccountConnection>,
        }
        #[derive(Deserialize)]
        struct User {
            email: String,
        }

        let url = format!("{}/api/connectors", self.origin);
        let response = self.http.get(&url).bearer_auth(token).send().await.map_err(|err| {
            AccountError::Transport { origin: self.origin.clone(), detail: err.to_string() }
        })?;

        let status = response.status();
        if status == reqwest::StatusCode::UNAUTHORIZED {
            return Err(AccountError::Expired { origin: self.origin.clone() });
        }
        let body = response.text().await.unwrap_or_default();
        if !status.is_success() {
            return Err(AccountError::Upstream {
                origin: self.origin.clone(),
                status: status.as_u16(),
                message: body.chars().take(300).collect(),
            });
        }

        let parsed: Body = serde_json::from_str(&body).map_err(|err| AccountError::Transport {
            origin: self.origin.clone(),
            detail: format!("its answer did not parse: {err}"),
        })?;
        Ok(Connectors {
            email: parsed.user.email,
            providers: parsed.providers,
            connections: parsed.connections,
        })
    }

    fn store(&self, stored: Stored) -> Result<Status, AccountError> {
        let status =
            Status { signed_in: true, email: stored.email.clone(), origin: self.origin.clone() };
        self.write(&stored)?;
        *self.tokens.lock() = Some(stored);
        Ok(status)
    }

    fn write(&self, stored: &Stored) -> Result<(), AccountError> {
        if let Some(parent) = self.path.parent() {
            fs::create_dir_all(parent)
                .map_err(|source| AccountError::Io { path: parent.to_path_buf(), source })?;
        }

        let json = serde_json::to_string_pretty(stored).map_err(|err| AccountError::Transport {
            origin: self.origin.clone(),
            detail: err.to_string(),
        })?;

        // Same temp-then-rename as the config and the subscription, for the
        // same reason: a crash mid-write must not leave a file that parses as a
        // partial sign-in. Permissions go on before the rename, so the file is
        // never briefly world-readable under its real name.
        let tmp = self.path.with_extension("json.tmp");
        fs::write(&tmp, json).map_err(|source| AccountError::Io { path: tmp.clone(), source })?;
        restrict(&tmp)?;
        fs::rename(&tmp, &self.path)
            .map_err(|source| AccountError::Io { path: self.path.clone(), source })?;
        Ok(())
    }
}

/// Whether a stored sign-in is one this build may present.
///
/// An empty stored origin is a file written before the field existed, and the
/// only service there has ever been is the default one.
fn belongs_to(stored: &Stored, origin: &str) -> bool {
    if stored.origin.is_empty() {
        return origin == DEFAULT_ORIGIN;
    }
    stored.origin == origin
}

fn now_ms() -> i64 {
    crate::domain::now_ms()
}

/// 0600, where the platform has such a thing.
#[cfg(unix)]
fn restrict(path: &std::path::Path) -> Result<(), AccountError> {
    use std::os::unix::fs::PermissionsExt;
    fs::set_permissions(path, fs::Permissions::from_mode(0o600))
        .map_err(|source| AccountError::Io { path: path.to_path_buf(), source })
}

#[cfg(not(unix))]
fn restrict(_path: &std::path::Path) -> Result<(), AccountError> {
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn account(origin: &str) -> Account {
        let dir = std::env::temp_dir().join(format!("guaca-account-{}", uuid::Uuid::new_v4()));
        Account::open_at(dir.join("account.json"), origin)
    }

    #[test]
    fn an_absent_file_is_signed_out_rather_than_a_failure() {
        let held = account(DEFAULT_ORIGIN);
        assert!(!held.is_signed_in());
        assert!(!held.status().signed_in);
        assert_eq!(held.status().origin, DEFAULT_ORIGIN);
    }

    #[test]
    fn a_file_that_does_not_parse_is_signed_out_rather_than_a_failure() {
        // The app is fully usable without an account, so an unreadable file is
        // a sign-in to redo, not a reason to refuse to start.
        let dir = std::env::temp_dir().join(format!("guaca-account-{}", uuid::Uuid::new_v4()));
        fs::create_dir_all(&dir).unwrap();
        let path = dir.join("account.json");
        fs::write(&path, "{ not json").unwrap();
        assert!(!Account::open(path).is_signed_in());
    }

    #[test]
    fn a_stored_file_with_no_token_is_not_a_sign_in() {
        let dir = std::env::temp_dir().join(format!("guaca-account-{}", uuid::Uuid::new_v4()));
        fs::create_dir_all(&dir).unwrap();
        let path = dir.join("account.json");
        fs::write(&path, r#"{"access_token":"","token_endpoint":"x"}"#).unwrap();
        assert!(!Account::open(path).is_signed_in());
    }

    /// A file written by a development build, and the real build reading it.
    fn write_stored(origin: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("guaca-account-{}", uuid::Uuid::new_v4()));
        fs::create_dir_all(&dir).unwrap();
        let path = dir.join("account.json");
        let stored = Stored {
            access_token: "token".to_string(),
            refresh_token: None,
            expires_at: None,
            email: "a@b.com".to_string(),
            token_endpoint: format!("{origin}/api/auth/oauth2/token"),
            origin: origin.to_string(),
        };
        fs::write(&path, serde_json::to_string(&stored).unwrap()).unwrap();
        path
    }

    #[test]
    fn a_sign_in_to_another_service_is_not_read_as_this_ones() {
        // One profile per bundle identifier, so a development build and the real
        // one share this file. Without the check, the real app presents a
        // localhost token to guaca.bot and reports a sign-in that used to work
        // as expired, with nothing on screen to say why.
        let path = write_stored("http://localhost:8787");
        assert!(!Account::open(path.clone()).is_signed_in());
        assert!(Account::open_at(path, "http://localhost:8787").is_signed_in());
    }

    #[test]
    fn a_file_written_before_the_service_was_recorded_is_the_default_one() {
        let dir = std::env::temp_dir().join(format!("guaca-account-{}", uuid::Uuid::new_v4()));
        fs::create_dir_all(&dir).unwrap();
        let path = dir.join("account.json");
        fs::write(&path, r#"{"access_token":"t","token_endpoint":"x"}"#).unwrap();

        assert!(Account::open(path.clone()).is_signed_in(), "the only service there has been");
        assert!(!Account::open_at(path, "http://localhost:8787").is_signed_in());
    }

    #[test]
    fn signing_out_when_signed_out_is_not_an_error() {
        assert!(account(DEFAULT_ORIGIN).sign_out().is_ok());
    }

    #[test]
    fn https_and_loopback_are_the_only_services_a_credential_goes_to() {
        for origin in [
            "https://guaca.bot",
            "https://example.test",
            "http://localhost:8787",
            "http://127.0.0.1:8787",
            "http://[::1]:8787",
        ] {
            assert!(account(origin).is_safe_origin(), "{origin} should be allowed");
        }
        // The whole point of the check: an environment variable must not be
        // able to put a credential on a plaintext connection across a network.
        for origin in ["http://guaca.bot", "http://evil.example", "ftp://guaca.bot", "guaca.bot"] {
            assert!(!account(origin).is_safe_origin(), "{origin} should be refused");
        }
    }

    #[test]
    fn an_endpoint_somewhere_else_is_not_followed() {
        let held = account("https://guaca.bot");
        assert!(held.is_same_origin("https://guaca.bot/api/auth/oauth2/token"));
        assert!(held.is_same_origin("https://guaca.bot"));
        // A published document that names another host is the one way discovery
        // could move a credential, so it is refused rather than followed.
        assert!(!held.is_same_origin("https://evil.example/token"));
        assert!(!held.is_same_origin("https://guaca.bot.evil.example/token"));
        assert!(!held.is_same_origin("http://guaca.bot/token"));
    }

    #[test]
    fn a_trailing_slash_on_the_service_is_not_a_second_service() {
        // Otherwise every discovered endpoint fails the same-origin check
        // against an origin nobody would look at twice.
        assert_eq!(account("https://guaca.bot/").origin(), "https://guaca.bot");
    }

    #[test]
    fn the_status_that_crosses_ipc_carries_no_token() {
        // The one invariant this type exists for. A field added here is a token
        // one serialization away from the webview.
        let json = serde_json::to_string(&Status {
            signed_in: true,
            email: "a@b.com".to_string(),
            origin: DEFAULT_ORIGIN.to_string(),
        })
        .unwrap();
        assert_eq!(json, r#"{"signedIn":true,"email":"a@b.com","origin":"https://guaca.bot"}"#);
    }
}
