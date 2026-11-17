use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::time::Duration;

use parking_lot::Mutex;
use serde::{Deserialize, Serialize};

pub const DEFAULT_ISSUER: &str = "https://auth.openai.com";

pub const CLIENT_ID: &str = "app_EMoamEEZ73f0CkXaXp7hrann";

const REFRESH_SKEW: Duration = Duration::from_secs(5 * 60);

const MAX_WAIT: Duration = Duration::from_secs(15 * 60);

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

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct DeviceCode {
    pub verification_url: String,
    pub user_code: String,
    pub device_auth_id: String,
    pub interval_secs: u64,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
struct Stored {
    access_token: String,
    refresh_token: String,
    id_token: String,
    account_id: String,
    expires_at: i64,
    #[serde(default)]
    email: String,
    #[serde(default)]
    plan: String,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Status {
    pub signed_in: bool,
    pub email: String,
    pub plan: String,
    pub includes_codex: bool,
}

#[derive(Debug)]
pub struct Subscription {
    path: PathBuf,
    issuer: String,
    backend: String,
    http: reqwest::Client,
    tokens: Mutex<Option<Stored>>,
    lifecycle: Mutex<Lifecycle>,
    renewing: tokio::sync::Mutex<()>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Credential {
    Absent(u64),
    Present(u64),
}

impl Credential {
    fn next(self, present: bool) -> Self {
        let generation = match self {
            Self::Absent(n) | Self::Present(n) => n + 1,
        };
        if present {
            Self::Present(generation)
        } else {
            Self::Absent(generation)
        }
    }
}

#[derive(Debug, PartialEq, Eq)]
enum Attempt {
    Idle,
    Starting(u64),
    Waiting(u64, String),
}

enum AttemptEvent {
    Start(u64),
    Issued(u64, String),
    Clear,
}

impl Attempt {
    fn transition(&mut self, event: AttemptEvent) -> bool {
        match event {
            AttemptEvent::Start(id) => *self = Self::Starting(id),
            AttemptEvent::Issued(id, device) => {
                if !matches!(self, Self::Starting(current) if *current == id) {
                    return false;
                }
                *self = Self::Waiting(id, device);
            }
            AttemptEvent::Clear => *self = Self::Idle,
        }
        true
    }
}

#[derive(Debug)]
struct Lifecycle {
    credential: Credential,
    attempt: Attempt,
    next_attempt: u64,
}

impl Subscription {
    pub fn open(path: PathBuf) -> Self {
        Self::open_at(path, DEFAULT_ISSUER, crate::llm::codex::DEFAULT_BASE_URL)
    }

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

        let credential =
            if tokens.is_some() { Credential::Present(0) } else { Credential::Absent(0) };
        Self {
            path,
            issuer: issuer.into(),
            backend: backend.into(),
            http: reqwest::Client::builder()
                .timeout(Duration::from_secs(30))
                .build()
                .unwrap_or_default(),
            tokens: Mutex::new(tokens),
            lifecycle: Mutex::new(Lifecycle {
                credential,
                attempt: Attempt::Idle,
                next_attempt: 0,
            }),
            renewing: tokio::sync::Mutex::new(()),
        }
    }

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

    pub async fn begin(&self) -> Result<DeviceCode, SigninError> {
        let attempt = {
            let mut state = self.lifecycle.lock();
            state.next_attempt += 1;
            let attempt = state.next_attempt;
            state.attempt.transition(AttemptEvent::Start(attempt));
            attempt
        };
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

        let mut state = self.lifecycle.lock();
        if !state.attempt.transition(AttemptEvent::Issued(attempt, issued.device_auth_id.clone())) {
            return Err(SigninError::NotSignedIn);
        }
        Ok(DeviceCode {
            verification_url: format!("{base}/codex/device"),
            user_code: issued.user_code,
            device_auth_id: issued.device_auth_id,
            interval_secs: Duration::from_secs(issued.interval).max(MIN_INTERVAL).as_secs(),
        })
    }

    pub async fn complete(&self, code: &DeviceCode) -> Result<Status, SigninError> {
        let attempt = match &self.lifecycle.lock().attempt {
            Attempt::Waiting(id, device) if *device == code.device_auth_id => *id,
            _ => return Err(SigninError::NotSignedIn),
        };
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

        let mut state = self.lifecycle.lock();
        if !matches!(&state.attempt, Attempt::Waiting(id, device) if *id == attempt && *device == code.device_auth_id)
        {
            return Err(SigninError::NotSignedIn);
        }
        let status = self.store(tokens)?;
        state.credential = state.credential.next(true);
        state.attempt.transition(AttemptEvent::Clear);
        Ok(status)
    }

    pub fn sign_out(&self) -> Result<(), SigninError> {
        let mut state = self.lifecycle.lock();
        state.credential = state.credential.next(false);
        state.attempt.transition(AttemptEvent::Clear);
        self.remove_credential()
    }

    fn remove_credential(&self) -> Result<(), SigninError> {
        *self.tokens.lock() = None;
        match fs::remove_file(&self.path) {
            Ok(()) => Ok(()),
            Err(err) if err.kind() == io::ErrorKind::NotFound => Ok(()),
            Err(source) => Err(SigninError::Io { path: self.path.clone(), source }),
        }
    }

    pub async fn access(&self) -> Result<Access, SigninError> {
        let (generation, stored) = {
            let state = self.lifecycle.lock();
            (state.credential, self.tokens.lock().clone())
        };
        let Some(stored) = stored else {
            return Err(SigninError::NotSignedIn);
        };

        if !expiring_soon(stored.expires_at) {
            return Ok(Access { token: stored.access_token, account_id: stored.account_id });
        }

        match self.renew(&stored.access_token).await {
            Ok(access) => Ok(access),
            Err(err @ SigninError::Transport { .. }) => {
                if self.lifecycle.lock().credential != generation {
                    return Err(SigninError::NotSignedIn);
                }
                tracing::warn!(%err, "using the stored token: its refresh could not be reached");
                Ok(Access { token: stored.access_token, account_id: stored.account_id })
            }
            Err(err) => Err(err),
        }
    }

    pub async fn renew(&self, refused: &str) -> Result<Access, SigninError> {
        let _gate = self.renewing.lock().await;

        let (generation, held) = {
            let state = self.lifecycle.lock();
            let Some(held) = self.tokens.lock().clone() else {
                return Err(SigninError::NotSignedIn);
            };
            (state.credential, held)
        };
        if held.access_token != refused {
            return Ok(Access { token: held.access_token, account_id: held.account_id });
        }

        let fresh = match self.refresh(&held.refresh_token).await {
            Ok(fresh) => fresh,
            Err(err) => {
                if is_terminal(&err) {
                    let mut state = self.lifecycle.lock();
                    if state.credential == generation {
                        state.credential = state.credential.next(false);
                        if let Err(gone) = self.remove_credential() {
                            tracing::warn!(%gone, "could not remove a sign-in the service refused");
                        }
                    }
                }
                return Err(err);
            }
        };

        let mut state = self.lifecycle.lock();
        if state.credential != generation {
            return Err(SigninError::NotSignedIn);
        }
        let status = self.store(fresh)?;
        state.credential = state.credential.next(true);
        debug_assert!(status.signed_in);
        match self.tokens.lock().clone() {
            Some(held) => Ok(Access { token: held.access_token, account_id: held.account_id }),
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
        if tokens.refresh_token.is_empty() {
            tokens.refresh_token = refresh_token.to_string();
        }
        Ok(tokens)
    }

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

    fn store(&self, tokens: Tokens) -> Result<Status, SigninError> {
        let previous = self.tokens.lock().clone();
        let id_token = if tokens.id_token.is_empty() {
            previous.map(|held| held.id_token).unwrap_or_default()
        } else {
            tokens.id_token
        };

        let claims = Claims::of(&id_token);
        let stored = Stored {
            expires_at: expiry_of(&tokens.access_token).unwrap_or(0),
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

        let tmp = self.path.with_extension("json.tmp");
        fs::write(&tmp, json).map_err(|source| SigninError::Io { path: tmp.clone(), source })?;
        restrict(&tmp)?;
        fs::rename(&tmp, &self.path)
            .map_err(|source| SigninError::Io { path: self.path.clone(), source })?;
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Access {
    pub token: String,
    pub account_id: String,
}

#[derive(Debug, Deserialize)]
struct UserCode {
    device_auth_id: String,
    #[serde(alias = "usercode")]
    user_code: String,
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

#[derive(Debug, Default, PartialEq)]
struct Claims {
    email: String,
    plan: String,
    account_id: String,
}

impl Claims {
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

fn expiry_of(jwt: &str) -> Option<i64> {
    let payload = jwt.split('.').nth(1)?;
    let decoded = base64url_decode(payload)?;
    let json: serde_json::Value = serde_json::from_slice(&decoded).ok()?;
    json.get("exp")?.as_i64()
}

fn expiring_soon(expires_at: i64) -> bool {
    let now = chrono::Utc::now().timestamp();
    expires_at <= now.saturating_add(REFRESH_SKEW.as_secs() as i64)
}

fn is_terminal(err: &SigninError) -> bool {
    matches!(err, SigninError::Upstream { status: 400 | 401 | 403, .. })
}

fn plan_includes_codex(plan: &str) -> bool {
    !matches!(plan.trim().to_ascii_lowercase().as_str(), "" | "free" | "unknown")
}

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

    fn access_token(expires_at: i64, nth: usize) -> String {
        jwt(serde_json::json!({ "exp": expires_at, "nth": nth }))
    }

    fn hour_ahead() -> i64 {
        chrono::Utc::now().timestamp() + 3600
    }

    #[derive(Clone, Default)]
    struct Stub {
        pending: Arc<AtomicUsize>,
        polls: Arc<AtomicUsize>,
        refreshes: Arc<AtomicUsize>,
        refresh_status: Arc<AtomicUsize>,
        refresh_omits_id_token: Arc<AtomicUsize>,
        refresh_gate: Option<Arc<tokio::sync::Barrier>>,
        exchange_gate: Option<Arc<tokio::sync::Barrier>>,
        exchanges: Arc<AtomicUsize>,
        begins: Arc<AtomicUsize>,
    }

    async fn serve(stub: Stub) -> String {
        let app = Router::new()
            .route(
                "/api/accounts/deviceauth/usercode",
                post(|State(stub): State<Stub>| async move {
                    let nth = stub.begins.fetch_add(1, Ordering::SeqCst);
                    axum::Json(serde_json::json!({
                        "device_auth_id": format!("dev-{}", nth + 1),
                        "user_code": "ABCD-EFGH",
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
                    if body.contains("grant_type=authorization_code") {
                        let nth = stub.exchanges.fetch_add(1, Ordering::SeqCst);
                        if let Some(gate) = stub.exchange_gate.as_ref().filter(|_| nth == 0) {
                            gate.wait().await;
                            gate.wait().await;
                        }
                        assert!(body.contains("code_verifier=verifier-1"), "verifier not sent");
                        assert!(body.contains("deviceauth%2Fcallback"), "redirect not sent");
                        return axum::Json(serde_json::json!({
                            "access_token": access_token(hour_ahead(), 0),
                            "refresh_token": "refresh-1",
                            "id_token": id_token(if nth == 0 { "pro" } else { "plus" }, "acct-1", "a@example.com"),
                        }))
                        .into_response();
                    }

                    let nth = stub.refreshes.fetch_add(1, Ordering::SeqCst) + 1;
                    if let Some(gate) = &stub.refresh_gate {
                        gate.wait().await;
                        gate.wait().await;
                    }
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

    #[test]
    fn credential_transitions_advance_the_generation() {
        assert_eq!(Credential::Absent(0).next(true), Credential::Present(1));
        assert_eq!(Credential::Absent(0).next(false), Credential::Absent(1));
        assert_eq!(Credential::Present(0).next(true), Credential::Present(1));
        assert_eq!(Credential::Present(0).next(false), Credential::Absent(1));
    }

    #[test]
    fn attempt_transitions_reject_a_stale_issuance() {
        let mut attempt = Attempt::Idle;
        assert!(attempt.transition(AttemptEvent::Start(1)));
        assert!(!attempt.transition(AttemptEvent::Issued(2, "old".into())));
        assert_eq!(attempt, Attempt::Starting(1));
        assert!(attempt.transition(AttemptEvent::Issued(1, "device".into())));
        assert_eq!(attempt, Attempt::Waiting(1, "device".into()));
        assert!(!attempt.transition(AttemptEvent::Issued(1, "again".into())));
        assert!(attempt.transition(AttemptEvent::Start(2)));
        assert!(attempt.transition(AttemptEvent::Clear));
        assert_eq!(attempt, Attempt::Idle);
        assert!(!attempt.transition(AttemptEvent::Issued(2, "late".into())));
    }

    #[tokio::test]
    async fn sign_out_invalidates_a_device_code_before_polling() {
        let base = serve(Stub::default()).await;
        let dir = tempfile::tempdir().unwrap();
        let subscription = store(&dir, &base);
        let code = subscription.begin().await.unwrap();
        subscription.sign_out().unwrap();
        assert!(subscription.complete(&code).await.is_err());
        assert!(!store(&dir, &base).is_signed_in());
    }

    #[tokio::test]
    async fn stale_refresh_cannot_restore_a_signed_out_credential() {
        let gate = Arc::new(tokio::sync::Barrier::new(2));
        let base = serve(Stub { refresh_gate: Some(gate.clone()), ..Default::default() }).await;
        let dir = tempfile::tempdir().unwrap();
        let subscription = Arc::new(store(&dir, &base));
        let code = subscription.begin().await.unwrap();
        subscription.complete(&code).await.unwrap();
        let token = subscription.access().await.unwrap().token;
        let task = {
            let subscription = subscription.clone();
            tokio::spawn(async move { subscription.renew(&token).await })
        };
        gate.wait().await;
        subscription.sign_out().unwrap();
        gate.wait().await;
        let _ = task.await.unwrap();
        assert!(!subscription.is_signed_in());
        assert!(!store(&dir, &base).is_signed_in());
    }

    #[tokio::test]
    async fn stale_device_exchange_cannot_restore_a_signed_out_credential() {
        let gate = Arc::new(tokio::sync::Barrier::new(2));
        let base = serve(Stub { exchange_gate: Some(gate.clone()), ..Default::default() }).await;
        let dir = tempfile::tempdir().unwrap();
        let subscription = Arc::new(store(&dir, &base));
        let code = subscription.begin().await.unwrap();
        let task = {
            let subscription = subscription.clone();
            tokio::spawn(async move { subscription.complete(&code).await })
        };
        gate.wait().await;
        subscription.sign_out().unwrap();
        gate.wait().await;
        let _ = task.await.unwrap();
        assert!(!subscription.is_signed_in());
        assert!(!store(&dir, &base).is_signed_in());
    }

    #[tokio::test]
    async fn stale_device_exchange_cannot_replace_a_new_login() {
        let gate = Arc::new(tokio::sync::Barrier::new(2));
        let base = serve(Stub { exchange_gate: Some(gate.clone()), ..Default::default() }).await;
        let dir = tempfile::tempdir().unwrap();
        let subscription = Arc::new(store(&dir, &base));
        let old = subscription.begin().await.unwrap();
        let task = {
            let subscription = subscription.clone();
            tokio::spawn(async move { subscription.complete(&old).await })
        };
        gate.wait().await;
        let new = subscription.begin().await.unwrap();
        subscription.complete(&new).await.unwrap();
        gate.wait().await;
        let _ = task.await.unwrap();
        assert_eq!(subscription.status().plan, "plus");
        assert_eq!(store(&dir, &base).status().plan, "plus");
    }

    async fn stale_refresh_cannot_change_a_new_login(status: usize) {
        let gate = Arc::new(tokio::sync::Barrier::new(2));
        let stub = Stub {
            refresh_gate: Some(gate.clone()),
            refresh_status: Arc::new(AtomicUsize::new(status)),
            ..Default::default()
        };
        let base = serve(stub).await;
        let dir = tempfile::tempdir().unwrap();
        let subscription = Arc::new(store(&dir, &base));
        let code = subscription.begin().await.unwrap();
        subscription.complete(&code).await.unwrap();
        let token = subscription.access().await.unwrap().token;
        let task = {
            let subscription = subscription.clone();
            tokio::spawn(async move { subscription.renew(&token).await })
        };
        gate.wait().await;
        let code = subscription.begin().await.unwrap();
        subscription.complete(&code).await.unwrap();
        gate.wait().await;
        let _ = task.await.unwrap();
        assert_eq!(subscription.status().plan, "plus", "refresh status {status}");
        assert_eq!(store(&dir, &base).status().plan, "plus", "refresh status {status}");
    }

    #[tokio::test]
    async fn stale_refresh_cannot_replace_a_new_login() {
        stale_refresh_cannot_change_a_new_login(0).await;
    }

    #[tokio::test]
    async fn stale_terminal_failure_cannot_retire_a_new_login() {
        stale_refresh_cannot_change_a_new_login(400).await;
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

        subscription.tokens.lock().as_mut().unwrap().expires_at =
            chrono::Utc::now().timestamp() + 60;

        subscription.access().await.unwrap();
        assert_eq!(stub.refreshes.load(Ordering::SeqCst), 1);

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

        assert_eq!(subscription.tokens.lock().as_ref().unwrap().refresh_token, "refresh-1");

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
        assert!(err.to_string().contains("Settings"), "{err}");
    }

    #[tokio::test]
    async fn an_unreachable_refresh_falls_back_to_the_token_it_has() {
        let base = serve(Stub::default()).await;
        let dir = tempfile::tempdir().unwrap();
        let subscription = store(&dir, &base);
        let code = subscription.begin().await.unwrap();
        subscription.complete(&code).await.unwrap();

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
        subscription.sign_out().unwrap();
    }

    #[tokio::test]
    async fn a_service_without_the_device_endpoint_says_so() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move { axum::serve(listener, Router::new()).await.unwrap() });

        let dir = tempfile::tempdir().unwrap();
        let subscription = store(&dir, &format!("http://{addr}"));
        assert!(matches!(subscription.begin().await, Err(SigninError::NotAvailable { .. }),));
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
        let padded = crate::e2b::encode(b"any carnal pleasure?");
        assert_eq!(base64url_decode(&padded).unwrap(), b"any carnal pleasure?");
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

        fs::write(&path, r#"{"access_token":"a","refresh_token":""}"#).unwrap();
        assert!(!Subscription::open(path).is_signed_in());
    }
}
