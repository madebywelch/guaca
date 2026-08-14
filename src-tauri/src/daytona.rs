//! Daytona sandboxes: one computer per agent.
//!
//! An agent that can only emit text is limited to what the model already knows.
//! A sandbox gives it a machine: a desktop it can be watched using, a terminal,
//! and a filesystem that survives between turns.
//!
//! This talks to Daytona's REST API rather than its TypeScript SDK, because the
//! sandbox is part of an agent's identity and agents live in the Rust runtime.
//! Driving it from the webview would tie a sandbox's lifetime to a window that
//! can be reloaded, and put a credential in the renderer.
//!
//! Two hosts are involved and they are not interchangeable:
//!
//! - `app.daytona.io/api` owns sandboxes: create, start, stop, delete, and the
//!   preview URLs.
//! - `proxy.app.daytona.io/toolbox/{id}` reaches inside a running sandbox. The
//!   desktop processes are started through it.
//!
//! A preview URL is public-looking but gated by a token. The token goes in a
//! query parameter because the UI embeds these in an iframe, which cannot set a
//! header. That is Daytona's own scheme, not a workaround: the cookie form is
//! rejected outright.

use std::collections::HashMap;
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};

/// Where noVNC serves the desktop inside a sandbox.
const VNC_PORT: u16 = 6080;
/// Where the web terminal serves.
const TERMINAL_PORT: u16 = 22222;

const API_BASE: &str = "https://app.daytona.io/api";

/// Daytona serves an "are you sure" interstitial in front of every preview URL,
/// and it is served for *every* request, not just the first document. Without
/// this header noVNC's own stylesheet and scripts come back as copies of that
/// warning page, and the desktop renders as unstyled text. An iframe cannot set
/// a header, which is the whole reason `proxy_get` below exists.
const SKIP_WARNING: (&str, &str) = ("X-Daytona-Skip-Preview-Warning", "true");
const TOOLBOX_BASE: &str = "https://proxy.app.daytona.io/toolbox";

/// Preview tokens, kept per sandbox and port.
///
/// noVNC pulls about thirty files to draw one desktop, and asking Daytona for a
/// fresh token before each of them turned one screen into thirty round trips.
/// The lifetime is deliberately short: Daytona resets a sandbox's tokens when it
/// restarts, so a stale one has to expire rather than be trusted forever, and a
/// rejected request clears the entry immediately.
type TokenCache = Mutex<HashMap<(String, u16), (String, Instant)>>;

fn token_cache() -> &'static TokenCache {
    static CACHE: OnceLock<TokenCache> = OnceLock::new();
    CACHE.get_or_init(|| Mutex::new(HashMap::new()))
}

const TOKEN_TTL: Duration = Duration::from_secs(120);

/// The URI scheme the webview loads an agent's computer through. Every request
/// on it is forwarded by `app.rs` with the interstitial suppressed.
pub const COMPUTER_SCHEME: &str = "guaccomputer";

/// Percent-encodes the few characters a preview token could contain that would
/// otherwise terminate the query it is embedded in.
fn urlencode(raw: &str) -> String {
    raw.chars()
        .map(|c| match c {
            'a'..='z' | 'A'..='Z' | '0'..='9' | '-' | '_' | '.' | '~' => c.to_string(),
            other => format!("%{:02X}", other as u32),
        })
        .collect()
}

/// Stops a sandbox nobody is watching. Minutes, not hours: an idle desktop is
/// billed the same as a busy one.
const AUTO_STOP_MINUTES: u32 = 30;

#[derive(Debug, thiserror::Error)]
pub enum DaytonaError {
    #[error("no Daytona API key is set; add one in app settings to give agents a computer")]
    NoKey,
    #[error("Daytona request failed: {0}")]
    Transport(String),
    #[error("Daytona rejected the request ({status}): {message}")]
    Api { status: u16, message: String },
}

/// What the UI needs to show an agent's computer.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Computer {
    pub sandbox_id: String,
    /// Daytona's own word: `started`, `stopped`, `creating`, `error`, …
    pub state: String,
    /// Absent unless the sandbox is running, because a preview URL for a
    /// stopped sandbox resolves to nothing and renders as a broken frame.
    pub vnc_url: Option<String>,
    pub terminal_url: Option<String>,
}

impl Computer {
    pub fn is_running(&self) -> bool {
        self.state == "started"
    }
}

#[derive(Debug, Deserialize)]
struct SandboxRow {
    id: String,
    #[serde(default)]
    state: String,
}

/// One file fetched from inside a sandbox, on its way to the webview.
#[derive(Debug, Clone)]
pub struct ProxiedFile {
    pub status: u16,
    pub content_type: String,
    pub body: Vec<u8>,
}

/// The proxy host for one sandbox port. Stable for the life of the sandbox,
/// unlike a signed URL, whose host changes on every call.
pub fn preview_host(sandbox: &str, port: u16) -> String {
    format!("https://{port}-{sandbox}.daytonaproxy01.net")
}

/// Only the token is taken from this. The host is derived from the sandbox and
/// port instead, because `preview_host` has to produce the same address for the
/// proxy and for the WebSocket, and one source for it is fewer than two.
#[derive(Debug, Deserialize)]
struct PreviewRow {
    token: String,
}

#[derive(Debug, Clone)]
pub struct DaytonaClient {
    http: reqwest::Client,
    api_key: String,
}

impl DaytonaClient {
    /// `None` when no key is configured, so callers can tell "not set up" apart
    /// from "set up and failing".
    pub fn new(api_key: &str) -> Option<Self> {
        let api_key = api_key.trim();
        if api_key.is_empty() {
            return None;
        }
        let http = reqwest::Client::builder().timeout(Duration::from_secs(30)).build().ok()?;
        Some(Self { http, api_key: api_key.to_string() })
    }

    async fn send<T: for<'de> Deserialize<'de>>(
        &self,
        request: reqwest::RequestBuilder,
    ) -> Result<T, DaytonaError> {
        let response = request
            .bearer_auth(&self.api_key)
            .send()
            .await
            .map_err(|e| DaytonaError::Transport(e.to_string()))?;

        let status = response.status();
        let body = response.text().await.unwrap_or_default();

        if !status.is_success() {
            // Daytona wraps its errors in {message}. Falling back to the raw
            // body keeps whatever it actually said rather than swallowing it.
            let message = serde_json::from_str::<serde_json::Value>(&body)
                .ok()
                .and_then(|v| v["message"].as_str().map(str::to_string))
                .unwrap_or_else(|| body.chars().take(200).collect());
            return Err(DaytonaError::Api { status: status.as_u16(), message });
        }

        serde_json::from_str(&body)
            .map_err(|e| DaytonaError::Transport(format!("could not read Daytona's reply: {e}")))
    }

    /// Creates a sandbox labelled with the agent that owns it, so an orphan is
    /// identifiable in Daytona's own dashboard rather than being an anonymous
    /// container someone is paying for.
    pub async fn create(&self, agent: &str) -> Result<String, DaytonaError> {
        let body = serde_json::json!({
            "labels": { "guac": "true", "guac-agent": agent },
            "autoStopInterval": AUTO_STOP_MINUTES,
        });
        let row: SandboxRow =
            self.send(self.http.post(format!("{API_BASE}/sandbox")).json(&body)).await?;
        Ok(row.id)
    }

    pub async fn state(&self, sandbox: &str) -> Result<String, DaytonaError> {
        let row: SandboxRow =
            self.send(self.http.get(format!("{API_BASE}/sandbox/{sandbox}"))).await?;
        Ok(row.state)
    }

    pub async fn start(&self, sandbox: &str) -> Result<(), DaytonaError> {
        let _: serde_json::Value =
            self.send(self.http.post(format!("{API_BASE}/sandbox/{sandbox}/start"))).await?;
        Ok(())
    }

    pub async fn stop(&self, sandbox: &str) -> Result<(), DaytonaError> {
        let _: serde_json::Value =
            self.send(self.http.post(format!("{API_BASE}/sandbox/{sandbox}/stop"))).await?;
        Ok(())
    }

    pub async fn delete(&self, sandbox: &str) -> Result<(), DaytonaError> {
        let response = self
            .http
            .delete(format!("{API_BASE}/sandbox/{sandbox}"))
            .bearer_auth(&self.api_key)
            .send()
            .await
            .map_err(|e| DaytonaError::Transport(e.to_string()))?;
        // A sandbox that is already gone is the outcome the caller wanted.
        if response.status().is_success() || response.status().as_u16() == 404 {
            return Ok(());
        }
        Err(DaytonaError::Api {
            status: response.status().as_u16(),
            message: response.text().await.unwrap_or_default().chars().take(200).collect(),
        })
    }

    /// Brings up Xvfb, xfce4, x11vnc and noVNC inside the sandbox.
    ///
    /// Safe to call again: Daytona reports the processes as already running
    /// rather than failing, which is what lets the UI ask for a desktop without
    /// tracking whether it has asked before.
    pub async fn start_desktop(&self, sandbox: &str) -> Result<(), DaytonaError> {
        let _: serde_json::Value = self
            .send(self.http.post(format!("{TOOLBOX_BASE}/{sandbox}/computeruse/start")))
            .await?;
        Ok(())
    }

    /// A URL the webview can put in an iframe, token included.
    async fn preview(&self, sandbox: &str, port: u16) -> Result<PreviewRow, DaytonaError> {
        self.send(self.http.get(format!("{API_BASE}/sandbox/{sandbox}/ports/{port}/preview-url")))
            .await
    }

    /// Fetches one file from inside a sandbox's preview, with the interstitial
    /// suppressed and the preview token attached.
    ///
    /// Returns the status alongside the body so the caller can pass a 404
    /// through as a 404 rather than turning every miss into an error page.
    pub async fn proxy_get(
        &self,
        sandbox: &str,
        port: u16,
        path: &str,
        query: Option<&str>,
    ) -> Result<ProxiedFile, DaytonaError> {
        let token = self.cached_token(sandbox, port).await?;
        let host = preview_host(sandbox, port);
        let url = match query {
            Some(q) if !q.is_empty() => format!("{host}/{path}?{q}"),
            _ => format!("{host}/{path}"),
        };

        let response = self
            .http
            .get(&url)
            .header(SKIP_WARNING.0, SKIP_WARNING.1)
            .header("x-daytona-preview-token", &token)
            .send()
            .await
            .map_err(|e| DaytonaError::Transport(e.to_string()))?;

        let status = response.status().as_u16();
        let content_type = response
            .headers()
            .get(reqwest::header::CONTENT_TYPE)
            .and_then(|v| v.to_str().ok())
            .unwrap_or("application/octet-stream")
            .to_string();
        let body =
            response.bytes().await.map_err(|e| DaytonaError::Transport(e.to_string()))?.to_vec();

        // A rejected token is almost always one Daytona rotated under us when
        // the sandbox restarted. Dropping it means the next request fetches a
        // fresh one instead of the whole desktop failing until a relaunch.
        if status == 401 || status == 403 {
            self.forget_token(sandbox, port);
        }

        Ok(ProxiedFile { status, content_type, body })
    }

    async fn cached_token(&self, sandbox: &str, port: u16) -> Result<String, DaytonaError> {
        let key = (sandbox.to_string(), port);
        if let Ok(cache) = token_cache().lock() {
            if let Some((token, taken)) = cache.get(&key) {
                if taken.elapsed() < TOKEN_TTL {
                    return Ok(token.clone());
                }
            }
        }

        let token = self.preview(sandbox, port).await?.token;
        if let Ok(mut cache) = token_cache().lock() {
            cache.insert(key, (token.clone(), Instant::now()));
        }
        Ok(token)
    }

    fn forget_token(&self, sandbox: &str, port: u16) {
        if let Ok(mut cache) = token_cache().lock() {
            cache.remove(&(sandbox.to_string(), port));
        }
    }

    /// The whole picture for one sandbox: state, and the two URLs if it is up.
    ///
    /// Preview URLs are fetched only for a running sandbox. Asking for one
    /// while it is stopped returns a link that renders as a broken frame, which
    /// reads as a bug rather than as a stopped computer.
    pub async fn describe(&self, sandbox: &str) -> Result<Computer, DaytonaError> {
        let state = self.state(sandbox).await?;
        let mut computer =
            Computer { sandbox_id: sandbox.to_string(), state, vnc_url: None, terminal_url: None };

        if computer.is_running() {
            // Both documents load through our own scheme so the interstitial can
            // be skipped. noVNC's RFB socket is pointed straight at Daytona
            // instead: a custom scheme cannot carry a WebSocket, and the socket
            // does not get the interstitial anyway, only documents do.
            let vnc = self.preview(sandbox, VNC_PORT).await?;
            let socket =
                format!("websockify%3FDAYTONA_SANDBOX_AUTH_KEY%3D{}", urlencode(&vnc.token));
            computer.vnc_url = Some(format!(
                "{scheme}://localhost/{sandbox}/{VNC_PORT}/vnc.html\
                 ?autoconnect=1&resize=scale&reconnect=1\
                 &host={port}-{sandbox}.daytonaproxy01.net&port=443&encrypt=1&path={socket}",
                scheme = COMPUTER_SCHEME,
                port = VNC_PORT,
            ));

            computer.terminal_url =
                Some(format!("{COMPUTER_SCHEME}://localhost/{sandbox}/{TERMINAL_PORT}/"));
        }

        Ok(computer)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_blank_key_means_not_configured_rather_than_a_client_that_always_fails() {
        assert!(DaytonaClient::new("   ").is_none());
        assert!(DaytonaClient::new("dtn_x").is_some());
    }

    #[test]
    fn a_preview_host_is_derived_from_the_sandbox_and_port() {
        // The proxy and the WebSocket must agree on the address, so this is the
        // one place it is built.
        assert_eq!(preview_host("abc-123", 6080), "https://6080-abc-123.daytonaproxy01.net");
    }

    #[test]
    fn a_token_is_encoded_before_it_is_put_in_a_query() {
        // The token is embedded inside noVNC's own `path` parameter, so a stray
        // & or = would silently truncate the socket address.
        assert_eq!(urlencode("abc_123-x.y~z"), "abc_123-x.y~z");
        assert_eq!(urlencode("a&b=c d"), "a%26b%3Dc%20d");
    }

    #[test]
    fn only_a_started_sandbox_counts_as_running() {
        let mut computer = Computer {
            sandbox_id: "s".into(),
            state: "stopped".into(),
            vnc_url: None,
            terminal_url: None,
        };
        assert!(!computer.is_running());
        // Daytona reports several transitional states, and none of them can
        // serve a desktop yet.
        for state in ["creating", "starting", "error", "archived"] {
            computer.state = state.into();
            assert!(!computer.is_running(), "{state} must not be treated as running");
        }
        computer.state = "started".into();
        assert!(computer.is_running());
    }
}
