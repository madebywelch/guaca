//! Signing in to a ChatGPT subscription, so a plan pays for a turn instead of a
//! key.
//!
//! This is a credential, not a protocol: what to do with the token once it is
//! held is `llm/codex.rs`. The two are separate because they fail differently.
//! A sign-in fails once, in front of the operator, and is fixed by signing in
//! again. A model call fails mid-turn, in front of an agent, and has to say
//! something an agent can act on.
//!
//! ## Why the device flow and not a loopback redirect
//!
//! The other half of OAuth's browser dance is a redirect back, and a desktop
//! app has two ways to catch one: bind a localhost port, or register a URL
//! scheme. Both put the app in the path of a credential arriving from a
//! browser. A port that is already taken silently belongs to something else,
//! and a scheme handler is claimed by whichever build of the app registered
//! last, which on a developer's machine is not the one they are running.
//!
//! The device flow has no redirect. Guaca asks for a code, the operator carries
//! it to a browser by hand, and Guaca polls for the answer. Nothing listens,
//! nothing is claimed, and the same code path works on a machine with no
//! browser at all.
//!
//! ## What is stored, and where
//!
//! Its own file, not `config.json`. The token set rotates on refresh, which is
//! Guaca writing to it in the background, while `config.json` is rewritten
//! wholesale every time the operator presses Save in Settings. Those two writers
//! on one file lose a refreshed token to a stale in-memory copy, and the symptom
//! is a sign-in that works until the operator changes an unrelated setting.
//!
//! Plaintext and 0600, for the reason `config.rs` gives about the API key: this
//! is a local app, and a key encrypted beside its own key is theater. Worth
//! being blunter here, though, because this credential is not Guaca's to lose:
//! it belongs to a ChatGPT account that has more than Guaca behind it. The
//! honest fix is the OS keychain, and it is the first thing to reach for if this
//! file ever grows a second reader.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::time::Duration;

use parking_lot::Mutex;
use serde::{Deserialize, Serialize};

/// Where the sign-in happens.
///
/// Overridable through [`Subscription::open_at`] rather than through settings:
/// there is no operator-facing reason to change it, and a mistyped sign-in
/// service is a credential sent somewhere nobody chose.
pub const DEFAULT_ISSUER: &str = "https://auth.openai.com";

/// The public client the Codex CLI signs in as.
///
/// Not a secret, and not Guaca's: it identifies the ChatGPT device-code
/// application that OpenAI operates, which is the one the subscription flow is
/// published under. A PKCE public client has no client secret by construction,
/// which is why this can be a constant in an app anybody can read.
pub const CLIENT_ID: &str = "app_EMoamEEZ73f0CkXaXp7hrann";

/// Refresh this far ahead of expiry rather than on rejection.
///
/// A turn that discovers its token expired has already spent the operator's
/// wait on a round trip that was never going to work, and the retry it triggers
/// looks like an outage. Five minutes is long enough to cover a slow refresh
/// and a clock that is a little wrong in either direction.
///
/// This is the optimization, not the guarantee. What decides whether a token
/// still works is the backend refusing it: see [`Subscription::renew`].
const REFRESH_SKEW: Duration = Duration::from_secs(5 * 60);

/// How long the whole device flow may take before it is abandoned.
///
/// The code the operator is carrying expires at fifteen minutes, so polling past
/// that is polling for something that cannot arrive.
const MAX_WAIT: Duration = Duration::from_secs(15 * 60);

/// Floor on the poll interval, whatever the server suggests.
///
/// A server that reports zero, or omits the field, would otherwise be polled in
/// a tight loop for fifteen minutes.
const MIN_INTERVAL: Duration = Duration::from_secs(2);

#[derive(Debug, thiserror::Error)]
pub enum SigninError {
    #[error("could not reach the sign-in service at {url}: {source}")]
    Transport {
        url: String,
        #[source]
        source: reqwest::Error,
    },
    #[error("the sign-in service returned HTTP {status}: {message}")]
    Upstream { status: u16, message: String },
    #[error(
        "device sign-in is not available at {url}. This build of Guaca is pointed at a sign-in \
         service that does not offer it."
    )]
    NotAvailable { url: String },
    #[error("the sign-in service sent something unexpected: {0}")]
    Malformed(String),
    #[error("nobody entered the code within fifteen minutes. Start the sign-in again.")]
    TimedOut,
    #[error(
        "no ChatGPT subscription is signed in on this machine. Open Settings, choose Provider, \
         and sign in."
    )]
    NotSignedIn,
    #[error("could not save the sign-in to {path}: {source}")]
    Io {
        path: PathBuf,
        #[source]
        source: io::Error,
    },
}

impl SigninError {
    fn upstream(status: u16, body: &str) -> Self {
        // The sign-in service reports a refused refresh as an OAuth error
        // object. Anything else is passed through truncated: an HTML error page
        // from a proxy is still worth seeing the first line of.
        let message = serde_json::from_str::<serde_json::Value>(body)
            .ok()
            .and_then(|v| {
                v.get("error_description")
                    .or_else(|| v.get("detail"))
                    .or_else(|| v.get("error"))
                    .and_then(|m| m.as_str().map(str::to_string))
            })
            .unwrap_or_else(|| body.trim().chars().take(300).collect());
        SigninError::Upstream { status, message }
    }
}

/// What a started sign-in gives the operator to carry to a browser.
///
/// `device_auth_id` comes back so the polling half can be a second call: the
/// dialog wants to draw the code immediately, and completing takes as long as
/// the operator does.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct DeviceCode {
    pub verification_url: String,
    pub user_code: String,
    pub device_auth_id: String,
    /// Seconds between polls, as the service asked for it.
    pub interval_secs: u64,
}

/// The token set, as stored. Never crosses IPC: see [`Status`].
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
struct Stored {
    access_token: String,
    refresh_token: String,
    /// Kept for its claims: the plan and the account this token spends against.
    id_token: String,
    /// The workspace a request is billed to, from the id token's claims.
    account_id: String,
    /// Unix seconds, read from the access token rather than from a duration the
    /// service quoted. A quoted lifetime plus the local clock at the moment of
    /// exchange drifts.
    ///
    /// It is a floor on the token's life and not a ceiling: the backend stops
    /// accepting one well before this, which is what [`Subscription::renew`]
    /// exists for. A token observed three days old, with seven days of `exp`
    /// still on it, was refused with `token_expired`.
    expires_at: i64,
    /// Denormalized out of the id token so a signed-in status can be answered
    /// without decoding a JWT on every read.
    #[serde(default)]
    email: String,
    #[serde(default)]
    plan: String,
}

/// What the webview is allowed to know about a sign-in.
///
/// No token, and no field one could arrive in. The plan and the email are here
/// because "signed in" on its own is not enough to tell an operator whether
/// they signed in to the account they meant to.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Status {
    pub signed_in: bool,
    pub email: String,
    /// As the service spells it: `plus`, `pro`, `team`, `enterprise`, `free`.
    pub plan: String,
    /// Whether that plan includes Codex. A free plan signs in successfully and
    /// then cannot make a single call, which is worth saying before a turn does.
    pub includes_codex: bool,
}

/// A signed-in ChatGPT subscription, and the file it lives in.
///
/// Cheap to clone: everything is behind the `Arc` its owner holds. The mutex is
/// held only to read or replace the token set, never across a request, so a
/// refresh cannot block a concurrent turn that already has a valid token.
#[derive(Debug)]
pub struct Subscription {
    path: PathBuf,
    issuer: String,
    /// Where the credential is spent. Travels with the credential rather than
    /// sitting in `codex.rs` as a constant, because an account and the backend
    /// that will accept it are one fact, not two.
    backend: String,
    http: reqwest::Client,
    tokens: Mutex<Option<Stored>>,
    /// Held across a refresh, so a crew that all discover a dead token at once
    /// spends one.
    ///
    /// The refresh token rotates, so concurrent refreshes race to retire each
    /// other's: the losers are left holding one the service has already thrown
    /// away, and the sign-in dies with nothing on screen able to say why.
    /// Whoever gets in first refreshes, and everyone behind them finds the new
    /// token already stored and takes it. Async rather than `parking_lot`
    /// because it is held across an await, which is the one thing the other
    /// lock here is careful never to be.
    renewing: tokio::sync::Mutex<()>,
}

impl Subscription {
    /// Opens the store at `path`, reading whatever is already there.
    ///
    /// A file that cannot be parsed is treated as "not signed in" rather than as
    /// a failure to start: the app is still usable with an API key, and the
    /// operator can sign in again, which overwrites it.
    pub fn open(path: PathBuf) -> Self {
        Self::open_at(path, DEFAULT_ISSUER, crate::llm::codex::DEFAULT_BASE_URL)
    }

    /// The same, against a named sign-in service and backend.
    ///
    /// Public because the end-to-end suite drives the real transport against a
    /// local stub, and a wire protocol that has to be right about two shapes at
    /// once is not worth testing any other way. It is also the seam a deployment
    /// that does not live at `chatgpt.com` would need, which is why it takes both
    /// halves rather than being a test hook for one.
    pub fn open_at(path: PathBuf, issuer: impl Into<String>, backend: impl Into<String>) -> Self {
        let tokens = match fs::read_to_string(&path) {
            Ok(raw) => match serde_json::from_str::<Stored>(&raw) {
                Ok(stored) if !stored.refresh_token.is_empty() => Some(stored),
                Ok(_) => None,
                Err(err) => {
                    tracing::warn!(%err, path = %path.display(), "ignoring an unreadable subscription file");
                    None
                }
            },
            Err(err) if err.kind() == io::ErrorKind::NotFound => None,
            Err(err) => {
                tracing::warn!(%err, path = %path.display(), "could not read the subscription file");
                None
            }
        };

        Self {
            path,
            issuer: issuer.into(),
            backend: backend.into(),
            http: reqwest::Client::builder()
                // Long enough for a slow token exchange, short enough that a
                // hung sign-in service does not park the dialog forever.
                .timeout(Duration::from_secs(30))
                .build()
                .unwrap_or_default(),
            tokens: Mutex::new(tokens),
            renewing: tokio::sync::Mutex::new(()),
        }
    }

    /// Where a call made with this credential goes.
    pub fn backend(&self) -> &str {
        &self.backend
    }

    pub fn status(&self) -> Status {
        match &*self.tokens.lock() {
            Some(stored) => Status {
                signed_in: true,
                email: stored.email.clone(),
                plan: stored.plan.clone(),
                includes_codex: plan_includes_codex(&stored.plan),
            },
            None => Status::default(),
        }
    }

    pub fn is_signed_in(&self) -> bool {
        self.tokens.lock().is_some()
    }

    /// Asks for a code the operator can carry to a browser.
    pub async fn begin(&self) -> Result<DeviceCode, SigninError> {
        let base = self.issuer.trim_end_matches('/');
        let url = format!("{base}/api/accounts/deviceauth/usercode");

        let response = self
            .http
            .post(&url)
            .json(&serde_json::json!({ "client_id": CLIENT_ID }))
            .send()
            .await
            .map_err(|source| SigninError::Transport { url: url.clone(), source })?;

        let status = response.status();
        if status == reqwest::StatusCode::NOT_FOUND {
            return Err(SigninError::NotAvailable { url: base.to_string() });
        }
        if !status.is_success() {
            let body = response.text().await.unwrap_or_default();
            return Err(SigninError::upstream(status.as_u16(), &body));
        }

        let body = response.text().await.unwrap_or_default();
        let issued: UserCode = serde_json::from_str(&body).map_err(|err| {
            SigninError::Malformed(format!("could not read the device code: {err}"))
        })?;

        Ok(DeviceCode {
            // The page the operator opens, which is not the API they were just
            // issued a code by.
            verification_url: format!("{base}/codex/device"),
            user_code: issued.user_code,
            device_auth_id: issued.device_auth_id,
            interval_secs: Duration::from_secs(issued.interval).max(MIN_INTERVAL).as_secs(),
        })
    }

    /// Waits for the operator to enter the code, then stores the result.
    ///
    /// Long-running by design: it returns when the sign-in finishes, is
    /// refused, or the code expires. The caller is an IPC command the dialog
    /// awaits, so a sign-in that is abandoned costs one parked call and no
    /// state.
    pub async fn complete(&self, code: &DeviceCode) -> Result<Status, SigninError> {
        let base = self.issuer.trim_end_matches('/');
        let url = format!("{base}/api/accounts/deviceauth/token");
        let interval = Duration::from_secs(code.interval_secs).max(MIN_INTERVAL);
        let deadline = tokio::time::Instant::now() + MAX_WAIT;

        let authorized = loop {
            let response = self
                .http
                .post(&url)
                .json(&serde_json::json!({
                    "device_auth_id": code.device_auth_id,
                    "user_code": code.user_code,
                }))
                .send()
                .await
                .map_err(|source| SigninError::Transport { url: url.clone(), source })?;

            let status = response.status();
            if status.is_success() {
                let body = response.text().await.unwrap_or_default();
                break serde_json::from_str::<Authorized>(&body).map_err(|err| {
                    SigninError::Malformed(format!("could not read the authorization: {err}"))
                })?;
            }

            // Not an error: the operator has not finished yet. Both spellings
            // are treated as "still waiting" because the service has used each,
            // and reading one of them as a refusal ends a sign-in that was
            // about to succeed.
            if status == reqwest::StatusCode::FORBIDDEN || status == reqwest::StatusCode::NOT_FOUND
            {
                if tokio::time::Instant::now() + interval > deadline {
                    return Err(SigninError::TimedOut);
                }
                tokio::time::sleep(interval).await;
                continue;
            }

            let body = response.text().await.unwrap_or_default();
            return Err(SigninError::upstream(status.as_u16(), &body));
        };

        // The verifier is the service's, not ours: this flow has it generate the
        // PKCE pair and hand back both halves with the code, because the browser
        // that authorized it and the app redeeming it are not the same process
        // and could not otherwise share a secret.
        let tokens = self
            .exchange(
                "authorization_code",
                &[
                    ("code", authorized.authorization_code.as_str()),
                    ("redirect_uri", &format!("{base}/deviceauth/callback")),
                    ("code_verifier", authorized.code_verifier.as_str()),
                ],
            )
            .await?;

        self.store(tokens)
    }

    /// Forgets the sign-in on this machine.
    ///
    /// Local only: it does not revoke the token at the service, because the
    /// operator's other Codex clients are signed in with their own and revoking
    /// centrally would sign them out of something Guaca does not own.
    pub fn sign_out(&self) -> Result<(), SigninError> {
        *self.tokens.lock() = None;
        match fs::remove_file(&self.path) {
            Ok(()) => Ok(()),
            Err(err) if err.kind() == io::ErrorKind::NotFound => Ok(()),
            Err(source) => Err(SigninError::Io { path: self.path.clone(), source }),
        }
    }

    /// A token that is good right now, and the workspace to bill it to.
    ///
    /// Refreshes when the stored one is close enough to expiry to be a risk.
    /// Every caller is a turn about to make a model call, so this is the only
    /// place that decides a token is too old, and it decides it before spending
    /// anything.
    pub async fn access(&self) -> Result<Access, SigninError> {
        let stored = self.tokens.lock().clone();
        let Some(stored) = stored else {
            return Err(SigninError::NotSignedIn);
        };

        if !expiring_soon(stored.expires_at) {
            return Ok(Access { token: stored.access_token, account_id: stored.account_id });
        }

        // A refresh that fails on the network leaves the stored token in place:
        // it may still have minutes left, and a turn that could have run is
        // worth more than a tidy cache. Every other failure surfaces, because
        // the token it was called to replace is already inside its own last
        // five minutes.
        match self.renew(&stored.access_token).await {
            Ok(access) => Ok(access),
            Err(err @ SigninError::Transport { .. }) => {
                tracing::warn!(%err, "using the stored token: its refresh could not be reached");
                Ok(Access { token: stored.access_token, account_id: stored.account_id })
            }
            Err(err) => Err(err),
        }
    }

    /// Trades the refresh token in for a new one, whatever the stored expiry
    /// says, and hands back what to spend next.
    ///
    /// This is the recovery path, and it exists because `exp` is not the truth.
    /// OpenAI mints a ChatGPT access token with ten days on its `exp` claim and
    /// the backend stops accepting it long before that, with `token_expired`.
    /// A build that believed the claim sat on a three-day-old token for a week,
    /// refused every turn, and went on reporting a healthy sign-in in Settings
    /// the whole time. So the backend is the authority: [`crate::llm::codex`]
    /// calls this on a 401 and tries the call again.
    ///
    /// `refused` is the token that was just turned down. If somebody else
    /// refreshed while this call was queued behind them, theirs is by
    /// definition not the one that failed, and it is handed back untouched
    /// rather than burned on a second refresh.
    pub async fn renew(&self, refused: &str) -> Result<Access, SigninError> {
        let _gate = self.renewing.lock().await;

        let Some(held) = self.tokens.lock().clone() else {
            return Err(SigninError::NotSignedIn);
        };
        if held.access_token != refused {
            return Ok(Access { token: held.access_token, account_id: held.account_id });
        }

        let fresh = match self.refresh(&held.refresh_token).await {
            Ok(fresh) => fresh,
            Err(err) => {
                // A refused refresh is the end of this sign-in, and keeping the
                // file is what let Settings claim a credential every turn had
                // already been told was dead. Forgetting it is what makes "sign
                // in again" a thing the operator can actually do.
                if is_terminal(&err) {
                    if let Err(gone) = self.sign_out() {
                        tracing::warn!(%gone, "could not remove a sign-in the service refused");
                    }
                }
                return Err(err);
            }
        };

        let status = self.store(fresh)?;
        debug_assert!(status.signed_in);
        match self.tokens.lock().clone() {
            Some(held) => Ok(Access { token: held.access_token, account_id: held.account_id }),
            // Signed out from Settings while this refresh was in flight. The
            // operator's click wins: a token stored back over it would sign
            // them silently back in.
            None => Err(SigninError::NotSignedIn),
        }
    }

    async fn refresh(&self, refresh_token: &str) -> Result<Tokens, SigninError> {
        let base = self.issuer.trim_end_matches('/');
        let url = format!("{base}/oauth/token");

        let response = self
            .http
            .post(&url)
            .json(&serde_json::json!({
                "client_id": CLIENT_ID,
                "grant_type": "refresh_token",
                "refresh_token": refresh_token,
            }))
            .send()
            .await
            .map_err(|source| SigninError::Transport { url: url.clone(), source })?;

        let status = response.status();
        let body = response.text().await.unwrap_or_default();
        if !status.is_success() {
            return Err(SigninError::upstream(status.as_u16(), &body));
        }

        let mut tokens: Tokens = serde_json::from_str(&body).map_err(|err| {
            SigninError::Malformed(format!("could not read the new token: {err}"))
        })?;
        // A refresh that rotates the refresh token says so; one that does not
        // expects the old one to keep working. Storing an empty string here
        // signs the operator out at the next refresh.
        if tokens.refresh_token.is_empty() {
            tokens.refresh_token = refresh_token.to_string();
        }
        Ok(tokens)
    }

    /// POSTs a grant to the token endpoint as form-encoded fields.
    ///
    /// Form-encoded rather than JSON because that is what the authorization-code
    /// grant is defined as and what the service accepts for it; the refresh
    /// grant is JSON. Two shapes on one endpoint is not a nicety anyone would
    /// design, and sending the wrong one is a 400 that reads like a bad code.
    async fn exchange(
        &self,
        grant_type: &str,
        fields: &[(&str, &str)],
    ) -> Result<Tokens, SigninError> {
        let base = self.issuer.trim_end_matches('/');
        let url = format!("{base}/oauth/token");

        let mut form = vec![("grant_type", grant_type), ("client_id", CLIENT_ID)];
        form.extend_from_slice(fields);

        let response = self
            .http
            .post(&url)
            .form(&form)
            .send()
            .await
            .map_err(|source| SigninError::Transport { url: url.clone(), source })?;

        let status = response.status();
        let body = response.text().await.unwrap_or_default();
        if !status.is_success() {
            return Err(SigninError::upstream(status.as_u16(), &body));
        }

        serde_json::from_str(&body)
            .map_err(|err| SigninError::Malformed(format!("could not read the token: {err}")))
    }

    /// Records a token set, and reports what it turned out to be.
    fn store(&self, tokens: Tokens) -> Result<Status, SigninError> {
        // A refresh does not have to re-issue the id token, and one that does
        // not expects the old one to stand. Reading claims out of the empty
        // string would blank the account every call is billed to, and the
        // symptom is every agent being refused an hour after a working sign-in,
        // with nothing in the settings looking wrong.
        let previous = self.tokens.lock().clone();
        let id_token = if tokens.id_token.is_empty() {
            previous.map(|held| held.id_token).unwrap_or_default()
        } else {
            tokens.id_token
        };

        let claims = Claims::of(&id_token);
        let stored = Stored {
            expires_at: expiry_of(&tokens.access_token).unwrap_or(0),
            // The account a request is billed to. Absent on a personal account
            // that has never had a workspace, and the backend accepts the call
            // without the header in that case, so an empty string is a real
            // state rather than a failure.
            account_id: claims.account_id,
            email: claims.email,
            plan: claims.plan,
            access_token: tokens.access_token,
            refresh_token: tokens.refresh_token,
            id_token,
        };

        let status = Status {
            signed_in: true,
            email: stored.email.clone(),
            plan: stored.plan.clone(),
            includes_codex: plan_includes_codex(&stored.plan),
        };

        self.write(&stored)?;
        *self.tokens.lock() = Some(stored);
        Ok(status)
    }

    fn write(&self, stored: &Stored) -> Result<(), SigninError> {
        if let Some(parent) = self.path.parent() {
            fs::create_dir_all(parent)
                .map_err(|source| SigninError::Io { path: parent.to_path_buf(), source })?;
        }

        let json = serde_json::to_string_pretty(stored)
            .map_err(|err| SigninError::Malformed(err.to_string()))?;

        // Same temp-then-rename as the config, for the same reason: a crash
        // mid-write must not leave a file that parses as a partial sign-in.
        // Permissions go on before the rename, so the file is never briefly
        // world-readable under its real name.
        let tmp = self.path.with_extension("json.tmp");
        fs::write(&tmp, json).map_err(|source| SigninError::Io { path: tmp.clone(), source })?;
        restrict(&tmp)?;
        fs::rename(&tmp, &self.path)
            .map_err(|source| SigninError::Io { path: self.path.clone(), source })?;
        Ok(())
    }
}

/// A token and the workspace it spends against.
#[derive(Debug, Clone, PartialEq)]
pub struct Access {
    pub token: String,
    /// Empty when the account has no workspace, which is a valid call.
    pub account_id: String,
}

#[derive(Debug, Deserialize)]
struct UserCode {
    device_auth_id: String,
    #[serde(alias = "usercode")]
    user_code: String,
    /// A string on the wire, not a number. Tolerating both is cheaper than
    /// finding out which one a deployment sends by having sign-in fail.
    #[serde(default, deserialize_with = "seconds")]
    interval: u64,
}

#[derive(Debug, Deserialize)]
struct Authorized {
    authorization_code: String,
    code_verifier: String,
}

#[derive(Debug, Deserialize)]
struct Tokens {
    access_token: String,
    #[serde(default)]
    refresh_token: String,
    #[serde(default)]
    id_token: String,
}

/// Reads a number that may have been sent as a JSON string.
fn seconds<'de, D: serde::Deserializer<'de>>(de: D) -> Result<u64, D::Error> {
    #[derive(Deserialize)]
    #[serde(untagged)]
    enum Either {
        Num(u64),
        Text(String),
    }
    Ok(match Either::deserialize(de)? {
        Either::Num(n) => n,
        Either::Text(s) => s.trim().parse().unwrap_or(0),
    })
}

/// The claims Guaca reads out of an id token.
#[derive(Debug, Default, PartialEq)]
struct Claims {
    email: String,
    plan: String,
    account_id: String,
}

impl Claims {
    /// Reads what it can, and reports nothing for what it cannot.
    ///
    /// The signature is deliberately not verified. This token was just received
    /// over TLS from the service that minted it, and nothing here is a security
    /// decision: the claims are used to label the sign-in for the operator and
    /// to fill in one request header. The thing that decides whether the
    /// credential is real is the backend rejecting it.
    fn of(id_token: &str) -> Self {
        let Some(payload) = id_token.split('.').nth(1) else {
            return Self::default();
        };
        let Some(decoded) = base64url_decode(payload) else {
            return Self::default();
        };
        let Ok(json) = serde_json::from_slice::<serde_json::Value>(&decoded) else {
            return Self::default();
        };

        let auth = json.get("https://api.openai.com/auth");
        let text = |value: Option<&serde_json::Value>| {
            value.and_then(|v| v.as_str()).unwrap_or_default().to_string()
        };

        Self {
            email: {
                let top = text(json.get("email"));
                if top.is_empty() {
                    text(json.get("https://api.openai.com/profile").and_then(|p| p.get("email")))
                } else {
                    top
                }
            },
            plan: text(auth.and_then(|a| a.get("chatgpt_plan_type"))),
            account_id: text(auth.and_then(|a| a.get("chatgpt_account_id"))),
        }
    }
}

/// `exp` out of a JWT, as unix seconds.
fn expiry_of(jwt: &str) -> Option<i64> {
    let payload = jwt.split('.').nth(1)?;
    let decoded = base64url_decode(payload)?;
    let json: serde_json::Value = serde_json::from_slice(&decoded).ok()?;
    json.get("exp")?.as_i64()
}

fn expiring_soon(expires_at: i64) -> bool {
    // An unreadable expiry stored as zero refreshes on first use, which is the
    // safe direction: one wasted round trip beats a turn spent on a dead token.
    let now = chrono::Utc::now().timestamp();
    expires_at <= now.saturating_add(REFRESH_SKEW.as_secs() as i64)
}

/// Whether a refused refresh means this sign-in is finished.
///
/// The token endpoint answers a refresh token it has retired with a 4xx and an
/// `invalid_grant`, and that is the one case where the operator has to sign in
/// again. A 5xx is the service having a bad minute and must not cost them a
/// working sign-in; neither must a body this build could not read, which is a
/// deployment that changed shape rather than a credential that expired.
fn is_terminal(err: &SigninError) -> bool {
    matches!(err, SigninError::Upstream { status: 400 | 401 | 403, .. })
}

/// Whether a plan can call Codex at all.
///
/// A free account signs in perfectly well and then cannot make one model call.
/// Naming the plans that work, rather than the ones that do not, means a plan
/// nobody here has heard of is treated as working: the backend is the authority
/// and a wrong "your plan cannot do this" is worse than a real error from it.
fn plan_includes_codex(plan: &str) -> bool {
    !matches!(plan.trim().to_ascii_lowercase().as_str(), "" | "free" | "unknown")
}

/// Base64url without padding, which is how a JWT spells its segments.
///
/// Hand-rolled for the same reason the encoder in `e2b.rs` is: it is twenty
/// lines and the alternative is a dependency. Tolerates padding and the
/// standard alphabet too, because a service that pads is not wrong and a decode
/// that fails here silently costs the operator the email beside their sign-in.
fn base64url_decode(input: &str) -> Option<Vec<u8>> {
    fn sextet(byte: u8) -> Option<u32> {
        Some(match byte {
            b'A'..=b'Z' => (byte - b'A') as u32,
            b'a'..=b'z' => (byte - b'a') as u32 + 26,
            b'0'..=b'9' => (byte - b'0') as u32 + 52,
            b'+' | b'-' => 62,
            b'/' | b'_' => 63,
            _ => return None,
        })
    }

    let raw: Vec<u8> = input.bytes().filter(|b| *b != b'=' && !b.is_ascii_whitespace()).collect();
    let mut out = Vec::with_capacity(raw.len() * 3 / 4);
    for chunk in raw.chunks(4) {
        // A single sextet cannot encode any whole byte, so a trailing one is a
        // truncated segment rather than something to salvage.
        if chunk.len() < 2 {
            return None;
        }
        let mut acc = 0u32;
        for byte in chunk {
            acc = (acc << 6) | sextet(*byte)?;
        }
        acc <<= 6 * (4 - chunk.len());
        out.push((acc >> 16) as u8);
        if chunk.len() > 2 {
            out.push((acc >> 8) as u8);
        }
        if chunk.len() > 3 {
            out.push(acc as u8);
        }
    }
    Some(out)
}

#[cfg(unix)]
fn restrict(path: &Path) -> Result<(), SigninError> {
    use std::os::unix::fs::PermissionsExt;
    fs::set_permissions(path, fs::Permissions::from_mode(0o600))
        .map_err(|source| SigninError::Io { path: path.to_path_buf(), source })
}

#[cfg(not(unix))]
fn restrict(_path: &Path) -> Result<(), SigninError> {
    // As `config.rs` says: a per-user config directory is already scoped by
    // Windows ACL defaults, and tightening further needs a platform crate.
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Arc;

    use axum::extract::State;
    use axum::response::IntoResponse;
    use axum::routing::post;
    use axum::Router;

    /// A JWT with the given claims. Unsigned: nothing here verifies one, and a
    /// test that signed them would be testing its own signer.
    fn jwt(claims: serde_json::Value) -> String {
        let payload = crate::e2b::encode(claims.to_string().as_bytes())
            .replace('+', "-")
            .replace('/', "_")
            .replace('=', "");
        format!("header.{payload}.signature")
    }

    fn id_token(plan: &str, account: &str, email: &str) -> String {
        jwt(serde_json::json!({
            "email": email,
            "https://api.openai.com/auth": {
                "chatgpt_plan_type": plan,
                "chatgpt_account_id": account,
            },
        }))
    }

    /// `nth` distinguishes one issue from the next, exactly as a real service's
    /// tokens do. Two refreshes a millisecond apart otherwise mint the same
    /// string, and a test about not refreshing twice cannot see the difference.
    fn access_token(expires_at: i64, nth: usize) -> String {
        jwt(serde_json::json!({ "exp": expires_at, "nth": nth }))
    }

    fn hour_ahead() -> i64 {
        chrono::Utc::now().timestamp() + 3600
    }

    /// The sign-in service, as much of it as the flow touches.
    #[derive(Clone, Default)]
    struct Stub {
        /// Polls to refuse before reporting the code entered.
        pending: Arc<AtomicUsize>,
        polls: Arc<AtomicUsize>,
        refreshes: Arc<AtomicUsize>,
        /// What the next refresh answers with, if it should not be a token.
        refresh_status: Arc<AtomicUsize>,
        /// Whether a refresh re-issues the id token. Real services do not
        /// promise to, and the claims have to survive one that does not.
        refresh_omits_id_token: Arc<AtomicUsize>,
    }

    async fn serve(stub: Stub) -> String {
        let app = Router::new()
            .route(
                "/api/accounts/deviceauth/usercode",
                post(|| async {
                    axum::Json(serde_json::json!({
                        "device_auth_id": "dev-1",
                        "user_code": "ABCD-EFGH",
                        // A string, as the service sends it.
                        "interval": "1",
                    }))
                    .into_response()
                }),
            )
            .route(
                "/api/accounts/deviceauth/token",
                post(|State(stub): State<Stub>| async move {
                    stub.polls.fetch_add(1, Ordering::SeqCst);
                    if stub.pending.load(Ordering::SeqCst) > 0 {
                        stub.pending.fetch_sub(1, Ordering::SeqCst);
                        return (axum::http::StatusCode::FORBIDDEN, "not yet").into_response();
                    }
                    axum::Json(serde_json::json!({
                        "authorization_code": "auth-code-1",
                        "code_challenge": "challenge",
                        "code_verifier": "verifier-1",
                    }))
                    .into_response()
                }),
            )
            .route(
                "/oauth/token",
                post(|State(stub): State<Stub>, body: String| async move {
                    // The two grants arrive in different shapes, which is the
                    // thing most worth pinning: form for the code exchange,
                    // JSON for the refresh.
                    if body.contains("grant_type=authorization_code") {
                        assert!(body.contains("code_verifier=verifier-1"), "verifier not sent");
                        assert!(body.contains("deviceauth%2Fcallback"), "redirect not sent");
                        return axum::Json(serde_json::json!({
                            "access_token": access_token(hour_ahead(), 0),
                            "refresh_token": "refresh-1",
                            "id_token": id_token("pro", "acct-1", "a@example.com"),
                        }))
                        .into_response();
                    }

                    let nth = stub.refreshes.fetch_add(1, Ordering::SeqCst) + 1;
                    let status = stub.refresh_status.load(Ordering::SeqCst);
                    if status != 0 {
                        return (
                            axum::http::StatusCode::from_u16(status as u16).unwrap(),
                            r#"{"error":"invalid_grant","error_description":"expired"}"#,
                        )
                            .into_response();
                    }

                    let parsed: serde_json::Value = serde_json::from_str(&body).unwrap();
                    assert_eq!(parsed["grant_type"], "refresh_token");
                    let mut fresh = serde_json::json!({
                        "access_token": access_token(hour_ahead(), nth),
                        // The refresh token is deliberately absent, to pin that
                        // the old one is kept.
                    });
                    if stub.refresh_omits_id_token.load(Ordering::SeqCst) == 0 {
                        fresh["id_token"] = id_token("team", "acct-2", "b@example.com").into();
                    }
                    axum::Json(fresh).into_response()
                }),
            )
            .with_state(stub);

        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        format!("http://{addr}")
    }

    fn store(dir: &tempfile::TempDir, issuer: &str) -> Subscription {
        Subscription::open_at(
            dir.path().join("subscription.json"),
            issuer,
            crate::llm::codex::DEFAULT_BASE_URL,
        )
    }

    #[tokio::test]
    async fn a_fresh_store_is_not_signed_in() {
        let dir = tempfile::tempdir().unwrap();
        let subscription = store(&dir, "http://unused");
        assert!(!subscription.is_signed_in());
        assert_eq!(subscription.status(), Status::default());
    }

    #[tokio::test]
    async fn signing_in_polls_until_the_code_is_entered() {
        let stub = Stub { pending: Arc::new(AtomicUsize::new(2)), ..Default::default() };
        let base = serve(stub.clone()).await;
        let dir = tempfile::tempdir().unwrap();
        let subscription = store(&dir, &base);

        let code = subscription.begin().await.unwrap();
        assert_eq!(code.user_code, "ABCD-EFGH");
        assert!(code.verification_url.ends_with("/codex/device"), "got {}", code.verification_url);
        // Floored, so a service that says one second is not polled faster than
        // the floor allows.
        assert_eq!(code.interval_secs, MIN_INTERVAL.as_secs());

        let status = subscription.complete(&code).await.unwrap();
        assert_eq!(status.plan, "pro");
        assert_eq!(status.email, "a@example.com");
        assert!(status.includes_codex);
        assert_eq!(stub.polls.load(Ordering::SeqCst), 3, "two refusals then the answer");
    }

    #[tokio::test]
    async fn a_completed_sign_in_survives_a_restart() {
        let base = serve(Stub::default()).await;
        let dir = tempfile::tempdir().unwrap();

        {
            let subscription = store(&dir, &base);
            let code = subscription.begin().await.unwrap();
            subscription.complete(&code).await.unwrap();
        }

        let reopened = store(&dir, &base);
        assert!(reopened.is_signed_in(), "a sign-in must outlive the process that made it");
        assert_eq!(reopened.status().plan, "pro");

        let access = reopened.access().await.unwrap();
        assert_eq!(access.account_id, "acct-1");
        assert!(!access.token.is_empty());
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn the_stored_file_is_readable_only_by_its_owner() {
        use std::os::unix::fs::PermissionsExt;

        let base = serve(Stub::default()).await;
        let dir = tempfile::tempdir().unwrap();
        let subscription = store(&dir, &base);
        let code = subscription.begin().await.unwrap();
        subscription.complete(&code).await.unwrap();

        let mode = fs::metadata(dir.path().join("subscription.json")).unwrap().permissions().mode();
        assert_eq!(mode & 0o777, 0o600, "a token file must not be group or world readable");
    }

    #[tokio::test]
    async fn a_token_near_expiry_is_refreshed_before_it_is_handed_out() {
        let stub = Stub::default();
        let base = serve(stub.clone()).await;
        let dir = tempfile::tempdir().unwrap();
        let subscription = store(&dir, &base);
        let code = subscription.begin().await.unwrap();
        subscription.complete(&code).await.unwrap();

        // Inside the skew, so still technically valid and still refreshed.
        subscription.tokens.lock().as_mut().unwrap().expires_at =
            chrono::Utc::now().timestamp() + 60;

        subscription.access().await.unwrap();
        assert_eq!(stub.refreshes.load(Ordering::SeqCst), 1);

        // And the refreshed claims replace the old ones.
        assert_eq!(subscription.status().plan, "team");
        assert_eq!(subscription.access().await.unwrap().account_id, "acct-2");
        assert_eq!(
            stub.refreshes.load(Ordering::SeqCst),
            1,
            "a fresh token is not refreshed again"
        );
    }

    #[tokio::test]
    async fn a_refresh_that_omits_a_new_refresh_token_keeps_the_old_one() {
        let stub = Stub::default();
        let base = serve(stub.clone()).await;
        let dir = tempfile::tempdir().unwrap();
        let subscription = store(&dir, &base);
        let code = subscription.begin().await.unwrap();
        subscription.complete(&code).await.unwrap();

        subscription.tokens.lock().as_mut().unwrap().expires_at = 0;
        subscription.access().await.unwrap();

        // The whole point: an empty refresh token stored here signs the
        // operator out at the next refresh, silently, hours later.
        assert_eq!(subscription.tokens.lock().as_ref().unwrap().refresh_token, "refresh-1");

        // Which is only provable by refreshing again.
        subscription.tokens.lock().as_mut().unwrap().expires_at = 0;
        subscription.access().await.unwrap();
        assert_eq!(stub.refreshes.load(Ordering::SeqCst), 2);
    }

    #[tokio::test]
    async fn a_refresh_that_omits_the_id_token_keeps_the_account_it_bills_to() {
        let stub =
            Stub { refresh_omits_id_token: Arc::new(AtomicUsize::new(1)), ..Default::default() };
        let base = serve(stub.clone()).await;
        let dir = tempfile::tempdir().unwrap();
        let subscription = store(&dir, &base);
        let code = subscription.begin().await.unwrap();
        subscription.complete(&code).await.unwrap();

        subscription.tokens.lock().as_mut().unwrap().expires_at = 0;
        let access = subscription.access().await.unwrap();

        // Reading claims out of an empty id token blanks all three of these,
        // and the first symptom is every agent refused an hour after a sign-in
        // that plainly worked.
        assert_eq!(access.account_id, "acct-1", "the billing account must survive a refresh");
        assert_eq!(subscription.status().plan, "pro");
        assert_eq!(subscription.status().email, "a@example.com");
    }

    #[tokio::test]
    async fn a_refused_refresh_surfaces_rather_than_being_retried_forever() {
        let stub = Stub { refresh_status: Arc::new(AtomicUsize::new(400)), ..Default::default() };
        let base = serve(stub.clone()).await;
        let dir = tempfile::tempdir().unwrap();
        let subscription = store(&dir, &base);
        let code = subscription.begin().await.unwrap();
        subscription.complete(&code).await.unwrap();

        subscription.tokens.lock().as_mut().unwrap().expires_at = 0;
        let err = subscription.access().await.unwrap_err();
        let message = err.to_string();
        assert!(message.contains("expired"), "the reason has to reach the operator: {message}");

        // And it is forgotten, because it is finished. A stored sign-in the
        // service has retired is what makes Settings say "signed in" while
        // every turn says the opposite, with no way out of the disagreement.
        assert!(!subscription.is_signed_in());
        assert!(!dir.path().join("subscription.json").exists());
    }

    #[tokio::test]
    async fn a_sign_in_service_having_a_bad_minute_does_not_cost_the_sign_in() {
        let stub = Stub { refresh_status: Arc::new(AtomicUsize::new(503)), ..Default::default() };
        let base = serve(stub.clone()).await;
        let dir = tempfile::tempdir().unwrap();
        let subscription = store(&dir, &base);
        let code = subscription.begin().await.unwrap();
        subscription.complete(&code).await.unwrap();

        subscription.tokens.lock().as_mut().unwrap().expires_at = 0;
        subscription.access().await.unwrap_err();

        // A 5xx is the service, not the credential. Signing the operator out
        // over one turns a wait into a sign-in they have to do by hand.
        assert!(subscription.is_signed_in());
        assert!(dir.path().join("subscription.json").exists());
    }

    #[tokio::test]
    async fn a_token_the_backend_refused_is_replaced_however_current_it_looks() {
        let stub = Stub::default();
        let base = serve(stub.clone()).await;
        let dir = tempfile::tempdir().unwrap();
        let subscription = store(&dir, &base);
        let code = subscription.begin().await.unwrap();
        subscription.complete(&code).await.unwrap();

        // A whole week of `exp` left, which is exactly the state the live
        // backend refuses: OpenAI mints ten days and stops accepting one after
        // about three. Nothing local can tell, so the refusal is the only
        // signal there is, and `renew` has to act on it.
        subscription.tokens.lock().as_mut().unwrap().expires_at =
            chrono::Utc::now().timestamp() + 7 * 86_400;
        let refused = subscription.access().await.unwrap();
        assert_eq!(stub.refreshes.load(Ordering::SeqCst), 0, "nothing was due yet");

        let fresh = subscription.renew(&refused.token).await.unwrap();
        assert_eq!(stub.refreshes.load(Ordering::SeqCst), 1);
        assert_ne!(fresh.token, refused.token, "the refused token cannot be handed back");
    }

    #[tokio::test]
    async fn one_refusal_seen_by_a_whole_crew_spends_one_refresh() {
        let stub = Stub::default();
        let base = serve(stub.clone()).await;
        let dir = tempfile::tempdir().unwrap();
        let subscription = Arc::new(store(&dir, &base));
        let code = subscription.begin().await.unwrap();
        subscription.complete(&code).await.unwrap();

        let refused = subscription.access().await.unwrap().token;

        // Eight agents, one dead token, all of them told so at once. The
        // refresh token rotates, so eight refreshes would race to retire each
        // other's and the last seven would be holding one the service has
        // already thrown away.
        let mut crew = Vec::new();
        for _ in 0..8 {
            let subscription = subscription.clone();
            let refused = refused.clone();
            crew.push(tokio::spawn(async move { subscription.renew(&refused).await }));
        }
        let mut tokens = Vec::new();
        for agent in crew {
            tokens.push(agent.await.unwrap().unwrap().token);
        }

        assert_eq!(stub.refreshes.load(Ordering::SeqCst), 1, "one dead token, one refresh");
        assert!(tokens.iter().all(|t| *t == tokens[0]), "everyone leaves with the same token");
        assert_ne!(tokens[0], refused);
    }

    #[tokio::test]
    async fn renewing_without_a_sign_in_says_so_in_words() {
        let dir = tempfile::tempdir().unwrap();
        let subscription = store(&dir, "http://unused");
        let err = subscription.renew("anything").await.unwrap_err();
        assert!(matches!(err, SigninError::NotSignedIn), "got {err}");
        // The operator reads this one after the app has signed them out for
        // them, so it has to say where to go rather than quote a status code.
        assert!(err.to_string().contains("Settings"), "{err}");
    }

    #[tokio::test]
    async fn an_unreachable_refresh_falls_back_to_the_token_it_has() {
        let base = serve(Stub::default()).await;
        let dir = tempfile::tempdir().unwrap();
        let subscription = store(&dir, &base);
        let code = subscription.begin().await.unwrap();
        subscription.complete(&code).await.unwrap();

        // A token with time left, and a sign-in service that has gone away.
        let subscription = Subscription::open_at(
            dir.path().join("subscription.json"),
            "http://127.0.0.1:1",
            crate::llm::codex::DEFAULT_BASE_URL,
        );
        subscription.tokens.lock().as_mut().unwrap().expires_at =
            chrono::Utc::now().timestamp() + 120;

        let access = subscription.access().await.expect("a usable token must still be usable");
        assert!(!access.token.is_empty());
    }

    #[tokio::test]
    async fn signing_out_removes_the_file() {
        let base = serve(Stub::default()).await;
        let dir = tempfile::tempdir().unwrap();
        let subscription = store(&dir, &base);
        let code = subscription.begin().await.unwrap();
        subscription.complete(&code).await.unwrap();

        subscription.sign_out().unwrap();
        assert!(!subscription.is_signed_in());
        assert!(!dir.path().join("subscription.json").exists());
        // Idempotent: signing out twice is not an error worth showing anyone.
        subscription.sign_out().unwrap();
    }

    #[tokio::test]
    async fn a_service_without_the_device_endpoint_says_so() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move { axum::serve(listener, Router::new()).await.unwrap() });

        let dir = tempfile::tempdir().unwrap();
        let subscription = store(&dir, &format!("http://{addr}"));
        assert!(matches!(
            subscription.begin().await,
            Err(SigninError::NotAvailable { .. }),
            // A bare "HTTP 404" here sends the operator looking at their network.
        ));
    }

    #[test]
    fn a_status_can_never_carry_a_token() {
        let status = Status {
            signed_in: true,
            email: "a@example.com".into(),
            plan: "pro".into(),
            includes_codex: true,
        };
        let json = serde_json::to_string(&status).unwrap();
        assert!(!json.contains("token"), "the status crosses IPC: {json}");
    }

    #[test]
    fn a_free_plan_is_reported_as_unable_to_call_codex() {
        assert!(!plan_includes_codex("free"));
        assert!(!plan_includes_codex(""));
        assert!(plan_includes_codex("plus"));
        assert!(plan_includes_codex("pro"));
        assert!(plan_includes_codex("team"));
        // A plan nobody here has heard of is the backend's call, not ours.
        assert!(plan_includes_codex("some-future-plan"));
    }

    #[test]
    fn claims_are_read_out_of_an_id_token() {
        let claims = Claims::of(&id_token("plus", "acct-9", "c@example.com"));
        assert_eq!(claims.plan, "plus");
        assert_eq!(claims.account_id, "acct-9");
        assert_eq!(claims.email, "c@example.com");
    }

    #[test]
    fn claims_fall_back_to_the_profile_email() {
        let token = jwt(serde_json::json!({
            "https://api.openai.com/profile": { "email": "d@example.com" },
            "https://api.openai.com/auth": { "chatgpt_plan_type": "pro" },
        }));
        assert_eq!(Claims::of(&token).email, "d@example.com");
    }

    #[test]
    fn an_unreadable_id_token_reports_nothing_rather_than_failing() {
        assert_eq!(Claims::of("not-a-jwt"), Claims::default());
        assert_eq!(Claims::of(""), Claims::default());
        assert_eq!(Claims::of("a.!!!!.c"), Claims::default());
    }

    #[test]
    fn base64url_round_trips_both_alphabets() {
        // The standard alphabet, as `e2b::encode` produces it.
        let padded = crate::e2b::encode(b"any carnal pleasure?");
        assert_eq!(base64url_decode(&padded).unwrap(), b"any carnal pleasure?");
        // And the url-safe one with the padding stripped, as a JWT carries it.
        let urlsafe = padded.replace('+', "-").replace('/', "_").replace('=', "");
        assert_eq!(base64url_decode(&urlsafe).unwrap(), b"any carnal pleasure?");
    }

    #[test]
    fn base64url_decodes_every_length_of_tail() {
        for len in 1..=8 {
            let raw: Vec<u8> = (0..len).map(|i| (i * 37 + 11) as u8).collect();
            let encoded = crate::e2b::encode(&raw).replace('=', "");
            assert_eq!(base64url_decode(&encoded).as_deref(), Some(&raw[..]), "length {len}");
        }
    }

    #[test]
    fn an_expiry_is_read_from_the_access_token_itself() {
        assert_eq!(expiry_of(&access_token(1_700_000_000, 0)), Some(1_700_000_000));
        assert_eq!(expiry_of("nonsense"), None);
    }

    #[test]
    fn an_unreadable_expiry_is_treated_as_due() {
        // Zero is what an unparseable token stores, and it has to refresh rather
        // than be handed to a turn.
        assert!(expiring_soon(0));
        assert!(expiring_soon(chrono::Utc::now().timestamp() + 60));
        assert!(!expiring_soon(chrono::Utc::now().timestamp() + 3600));
    }

    #[test]
    fn a_corrupt_store_reads_as_signed_out_rather_than_failing_to_start() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("subscription.json");
        fs::write(&path, "{ not json").unwrap();
        assert!(!Subscription::open(path.clone()).is_signed_in());

        // And so does a well-formed file with nothing to refresh with, which is
        // what a half-written sign-in would leave.
        fs::write(&path, r#"{"access_token":"a","refresh_token":""}"#).unwrap();
        assert!(!Subscription::open(path).is_signed_in());
    }
}
