use std::fs;
use std::io;
use std::path::PathBuf;
use std::time::Duration;

use parking_lot::Mutex;
use serde::{Deserialize, Serialize};

use crate::oauth::{self, OauthError};
use crate::secrets::Secret;

pub const DEFAULT_ORIGIN: &str = "https://guaca.bot";

pub const CLIENT_ID: &str = "guaca-desktop";

pub const SCOPE: &str = "openid email offline_access connectors";

const REFRESH_SKEW_MS: i64 = 5 * 60 * 1000;

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
    #[error("the sign-in was refused: {error}{}", .description.as_deref().map(|d| format!(": {d}")).unwrap_or_default())]
    Refused { error: String, description: Option<String> },
    #[error("the answer from your browser did not match the sign-in that was started")]
    StateMismatch,
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
    #[error(
        "the sign-in did something this build has no answer for: {detail}. That is a bug in Guaca \
         rather than something to retry."
    )]
    Unexpected { detail: String },
    #[error("not signed in to a Guaca account")]
    NotSignedIn,
    #[error(
        "Remove GUACA_BROKER_BEARER from the host environment and restart the host to sign out."
    )]
    BrokerConfigured,
    #[error("{detail}")]
    Broker { detail: String },
    #[error("the sign-in to {origin} has expired. Sign in again.")]
    Expired { origin: String },
    #[error("could not save the sign-in to {path}: {source}")]
    Io {
        path: PathBuf,
        #[source]
        source: io::Error,
    },
}

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
            err @ (OauthError::NoRegistration { .. }
            | OauthError::NoToken { .. }
            | OauthError::NoRefreshToken) => AccountError::Unexpected { detail: err.to_string() },
        }
    }
}

#[derive(Debug, Clone, Default, Deserialize, PartialEq)]
struct Stored {
    access_token: Secret,
    #[serde(default)]
    refresh_token: Option<Secret>,
    #[serde(default)]
    expires_at: Option<i64>,
    #[serde(default)]
    email: String,
    token_endpoint: String,
    #[serde(default)]
    origin: String,
}

impl Stored {
    fn on_disk(&self) -> serde_json::Value {
        serde_json::json!({
            "access_token": self.access_token.expose(),
            "refresh_token": self.refresh_token.as_ref().map(Secret::expose),
            "expires_at": self.expires_at,
            "email": self.email,
            "token_endpoint": self.token_endpoint,
            "origin": self.origin,
        })
    }

    fn expiring(&self) -> bool {
        self.expires_at.is_some_and(|at| at - REFRESH_SKEW_MS <= now_ms())
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Status {
    pub signed_in: bool,
    pub email: String,
    pub origin: String,
    #[serde(default)]
    pub broker: bool,
}

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

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct AccountConnection {
    pub id: String,
    pub provider: String,
    pub label: String,
    pub capabilities: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Connectors {
    pub email: String,
    pub providers: Vec<Provider>,
    #[serde(default)]
    pub connections: Vec<AccountConnection>,
}

enum Credential {
    Absent,
    Present(Stored),
}

struct AccountState {
    generation: u64,
    attempt: u64,
    credential: Credential,
}

enum Transition {
    Begin,
    SignIn(u64, Stored),
    Refresh(u64, Stored),
    SignOut,
}

impl AccountState {
    fn transition(&mut self, event: Transition) -> bool {
        match event {
            Transition::Begin => self.attempt = self.attempt.wrapping_add(1),
            Transition::SignIn(attempt, stored) => {
                if attempt != self.attempt {
                    return false;
                }
                self.generation = self.generation.wrapping_add(1);
                self.credential = Credential::Present(stored);
            }
            Transition::Refresh(generation, stored) => {
                if generation != self.generation {
                    return false;
                }
                self.credential = Credential::Present(stored);
            }
            Transition::SignOut => {
                self.generation = self.generation.wrapping_add(1);
                self.attempt = self.attempt.wrapping_add(1);
                self.credential = Credential::Absent;
            }
        }
        true
    }

    fn stored(&self) -> Option<&Stored> {
        match &self.credential {
            Credential::Absent => None,
            Credential::Present(stored) => Some(stored),
        }
    }
}

pub struct Account {
    path: PathBuf,
    origin: String,
    http: reqwest::Client,
    broker_http: reqwest::Client,
    tokens: Mutex<AccountState>,
    broker: Option<String>,
    origin_override: bool,
    renewing: tokio::sync::Mutex<()>,
}

impl Account {
    pub fn open(path: PathBuf) -> Self {
        let mut account = Self::open_at(path, DEFAULT_ORIGIN);
        account.origin_override = false;
        account
    }

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
            broker_http: reqwest::Client::builder()
                .timeout(HTTP_TIMEOUT)
                .redirect(reqwest::redirect::Policy::none())
                .build()
                .expect("could not build account HTTP client"),
            tokens: Mutex::new(AccountState {
                generation: 0,
                attempt: 0,
                credential: tokens.map_or(Credential::Absent, Credential::Present),
            }),
            broker: None,
            origin_override: true,
            renewing: tokio::sync::Mutex::new(()),
        }
    }

    pub fn with_broker(mut self, bearer: String) -> Self {
        self.broker = Some(bearer);
        self
    }

    pub fn has_broker(&self) -> bool {
        self.broker.is_some()
    }

    fn broker_token(&self) -> Result<&str, AccountError> {
        if !self.origin_override || !self.is_safe_origin() {
            return Err(AccountError::UnsafeOrigin { origin: "configured account origin".into() });
        }
        self.broker.as_deref().filter(|s| !s.is_empty()).ok_or(AccountError::NotSignedIn)
    }

    pub async fn checked_status(&self) -> Status {
        if self.broker.is_none() {
            return self.status();
        }
        let held = match self.broker_token() {
            Ok(token) => self.fetch_connectors(token).await.ok(),
            Err(_) => None,
        };
        Status {
            signed_in: held.is_some(),
            email: held.map_or_else(String::new, |h| h.email),
            origin: if self.is_safe_origin() { self.origin.clone() } else { String::new() },
            broker: true,
        }
    }

    pub async fn open_broker(
        &self,
        open: impl FnOnce(&str) -> Result<(), String>,
    ) -> Result<Status, AccountError> {
        #[derive(Deserialize)]
        struct Handoff {
            code: String,
        }
        let token = self.broker_token()?;
        let response = self
            .broker_http
            .post(format!("{}/api/selfhost/handoff", self.origin))
            .bearer_auth(token)
            .send()
            .await
            .map_err(|_| AccountError::Broker { detail: "could not reach the broker".into() })?;
        if !response.status().is_success() {
            return Err(AccountError::Broker {
                detail: format!(
                    "the broker refused the handoff (HTTP {})",
                    response.status().as_u16()
                ),
            });
        }
        let handoff: Handoff = response.json().await.map_err(|_| AccountError::Broker {
            detail: "the broker returned an invalid handoff".into(),
        })?;
        if handoff.code.is_empty() {
            return Err(AccountError::Broker {
                detail: "the broker returned an empty handoff".into(),
            });
        }
        let url = format!("{}/enter?code={}", self.origin, oauth::encode(&handoff.code));
        open(&url).map_err(|_| AccountError::NoBrowser {
            detail: "could not open the broker page".into(),
        })?;
        Ok(self.checked_status().await)
    }

    pub fn origin(&self) -> &str {
        &self.origin
    }

    pub fn status(&self) -> Status {
        match self.tokens.lock().stored() {
            Some(stored) => Status {
                signed_in: true,
                email: stored.email.clone(),
                origin: self.origin.clone(),
                broker: false,
            },
            None => Status {
                signed_in: false,
                email: String::new(),
                origin: self.origin.clone(),
                broker: false,
            },
        }
    }

    pub fn is_signed_in(&self) -> bool {
        self.tokens.lock().stored().is_some()
    }

    pub async fn sign_in(
        &self,
        open: impl FnOnce(&str) -> Result<(), String>,
        landing: &oauth::Landing,
    ) -> Result<Status, AccountError> {
        let attempt = {
            let mut state = self.tokens.lock();
            state.transition(Transition::Begin);
            state.attempt
        };
        let server = self.discover().await?;

        let verifier = oauth::secret();
        let challenge = oauth::pkce_challenge(&verifier);
        let state = oauth::secret();

        let opened = landing.open(&state).await?;
        let redirect_uri = opened.redirect_uri.clone();

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

        let code = opened.wait(&state, &server.issuer).await?;

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
            email: String::new(),
            token_endpoint: server.token_endpoint,
            origin: self.origin.clone(),
        };

        let held = self.fetch_connectors(stored.access_token.expose()).await?;
        self.store(true, attempt, Stored { email: held.email, ..stored })
    }

    pub fn sign_out(&self) -> Result<(), AccountError> {
        if self.has_broker() {
            return Err(AccountError::BrokerConfigured);
        }
        let mut state = self.tokens.lock();
        state.transition(Transition::SignOut);
        match fs::remove_file(&self.path) {
            Ok(()) => Ok(()),
            Err(err) if err.kind() == io::ErrorKind::NotFound => Ok(()),
            Err(source) => Err(AccountError::Io { path: self.path.clone(), source }),
        }
    }

    pub async fn connectors(&self) -> Result<Connectors, AccountError> {
        let token = self.access().await?;
        self.fetch_connectors(&token).await
    }

    pub async fn access(&self) -> Result<String, AccountError> {
        if self.has_broker() {
            return Ok(self.broker_token()?.to_owned());
        }
        let stored = self.tokens.lock().stored().cloned().ok_or(AccountError::NotSignedIn)?;
        if !stored.expiring() {
            return Ok(stored.access_token.expose().to_string());
        }

        let _gate = self.renewing.lock().await;

        let (stored, generation) = {
            let state = self.tokens.lock();
            (state.stored().cloned().ok_or(AccountError::NotSignedIn)?, state.generation)
        };
        if !stored.expiring() {
            return Ok(stored.access_token.expose().to_string());
        }

        let Some(refresh_token) = stored.refresh_token.clone() else {
            return Ok(stored.access_token.expose().to_string());
        };

        match self.refresh(&stored, refresh_token.expose(), generation).await {
            Ok(fresh) => Ok(fresh),
            Err(err @ AccountError::Transport { .. }) => {
                tracing::warn!(
                    %err,
                    "using the stored account token: its refresh could not be reached"
                );
                Ok(stored.access_token.expose().to_string())
            }
            Err(err) => {
                tracing::warn!(%err, origin = %self.origin, "the account refused to renew");
                Err(err)
            }
        }
    }

    async fn refresh(
        &self,
        stored: &Stored,
        refresh_token: &str,
        generation: u64,
    ) -> Result<String, AccountError> {
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

        let access_token = issued.access_token.expose().to_string();
        self.store(
            false,
            generation,
            Stored {
                access_token: issued.access_token,
                refresh_token: issued.refresh_token.or_else(|| Some(refresh_token.into())),
                expires_at: issued.expires_in.map(|secs| now_ms() + secs * 1000),
                email: stored.email.clone(),
                token_endpoint: stored.token_endpoint.clone(),
                origin: self.origin.clone(),
            },
        )?;
        Ok(access_token)
    }

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

    fn is_safe_origin(&self) -> bool {
        let Ok(url) = reqwest::Url::parse(&self.origin) else {
            return false;
        };
        if !url.username().is_empty()
            || url.password().is_some()
            || url.path() != "/"
            || url.query().is_some()
            || url.fragment().is_some()
        {
            return false;
        }
        url.scheme() == "https"
            || (url.scheme() == "http"
                && matches!(url.host_str(), Some("localhost" | "127.0.0.1" | "[::1]")))
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
        let client = if self.has_broker() { &self.broker_http } else { &self.http };
        let response = client.get(&url).bearer_auth(token).send().await.map_err(|err| {
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
                message: if self.has_broker() {
                    "broker request failed".into()
                } else {
                    body.chars().take(300).collect()
                },
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

    fn store(
        &self,
        sign_in: bool,
        generation: u64,
        stored: Stored,
    ) -> Result<Status, AccountError> {
        let mut state = self.tokens.lock();
        if generation != if sign_in { state.attempt } else { state.generation } {
            return Ok(Status {
                signed_in: state.stored().is_some(),
                email: state.stored().map_or_else(String::new, |s| s.email.clone()),
                origin: self.origin.clone(),
                broker: false,
            });
        }
        let status = Status {
            signed_in: true,
            email: stored.email.clone(),
            origin: self.origin.clone(),
            broker: false,
        };
        self.write(&stored)?;
        state.transition(if sign_in {
            Transition::SignIn(generation, stored)
        } else {
            Transition::Refresh(generation, stored)
        });
        Ok(status)
    }

    fn write(&self, stored: &Stored) -> Result<(), AccountError> {
        if let Some(parent) = self.path.parent() {
            fs::create_dir_all(parent)
                .map_err(|source| AccountError::Io { path: parent.to_path_buf(), source })?;
        }

        let json = serde_json::to_string_pretty(&stored.on_disk()).map_err(|err| {
            AccountError::Transport { origin: self.origin.clone(), detail: err.to_string() }
        })?;

        let tmp = self.path.with_extension("json.tmp");
        fs::write(&tmp, json).map_err(|source| AccountError::Io { path: tmp.clone(), source })?;
        restrict(&tmp)?;
        fs::rename(&tmp, &self.path)
            .map_err(|source| AccountError::Io { path: self.path.clone(), source })?;
        Ok(())
    }
}

fn belongs_to(stored: &Stored, origin: &str) -> bool {
    if stored.origin.is_empty() {
        return origin == DEFAULT_ORIGIN;
    }
    stored.origin == origin
}

fn now_ms() -> i64 {
    crate::domain::now_ms()
}

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
    #[test]
    fn credential_transitions_reject_obsolete_commits() {
        use super::{AccountState, Credential, Stored, Transition};
        let mut state = AccountState { generation: 0, attempt: 0, credential: Credential::Absent };
        assert!(state.stored().is_none());
        assert!(state.transition(Transition::Begin));
        assert!(state.transition(Transition::SignIn(1, Stored::default())));
        assert!(state.stored().is_some());
        assert!(state.transition(Transition::Begin));
        assert!(!state.transition(Transition::SignIn(1, Stored::default())));
        assert!(state.stored().is_some());
        assert!(state.transition(Transition::Refresh(1, Stored::default())));
        assert!(state.transition(Transition::SignOut));
        assert!(state.stored().is_none());
        assert!(!state.transition(Transition::SignIn(2, Stored::default())));
        assert!(!state.transition(Transition::Refresh(1, Stored::default())));
        assert!(state.transition(Transition::SignOut));
        assert!(state.stored().is_none());
    }
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

    fn write_stored(origin: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("guaca-account-{}", uuid::Uuid::new_v4()));
        fs::create_dir_all(&dir).unwrap();
        let path = dir.join("account.json");
        let stored = Stored {
            access_token: "token".into(),
            refresh_token: None,
            expires_at: None,
            email: "a@b.com".to_string(),
            token_endpoint: format!("{origin}/api/auth/oauth2/token"),
            origin: origin.to_string(),
        };
        fs::write(&path, stored.on_disk().to_string()).unwrap();
        path
    }

    #[test]
    fn a_sign_in_to_another_service_is_not_read_as_this_ones() {
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
        for origin in ["http://guaca.bot", "http://evil.example", "ftp://guaca.bot", "guaca.bot"] {
            assert!(!account(origin).is_safe_origin(), "{origin} should be refused");
        }
    }

    #[test]
    fn an_endpoint_somewhere_else_is_not_followed() {
        let held = account("https://guaca.bot");
        assert!(held.is_same_origin("https://guaca.bot/api/auth/oauth2/token"));
        assert!(held.is_same_origin("https://guaca.bot"));
        assert!(!held.is_same_origin("https://evil.example/token"));
        assert!(!held.is_same_origin("https://guaca.bot.evil.example/token"));
        assert!(!held.is_same_origin("http://guaca.bot/token"));
    }

    #[test]
    fn a_trailing_slash_on_the_service_is_not_a_second_service() {
        assert_eq!(account("https://guaca.bot/").origin(), "https://guaca.bot");
    }

    #[test]
    fn the_status_that_crosses_ipc_carries_no_token() {
        let json = serde_json::to_string(&Status {
            signed_in: true,
            email: "a@b.com".to_string(),
            origin: DEFAULT_ORIGIN.to_string(),
            broker: false,
        })
        .unwrap();
        assert_eq!(
            json,
            r#"{"signedIn":true,"email":"a@b.com","origin":"https://guaca.bot","broker":false}"#
        );
    }
}
