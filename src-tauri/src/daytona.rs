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

use std::time::Duration;

use serde::{Deserialize, Serialize};

/// Where noVNC serves the desktop inside a sandbox.
const VNC_PORT: u16 = 6080;
/// Where the web terminal serves.
const TERMINAL_PORT: u16 = 22222;

const API_BASE: &str = "https://app.daytona.io/api";
const TOOLBOX_BASE: &str = "https://proxy.app.daytona.io/toolbox";

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

#[derive(Debug, Deserialize)]
struct PreviewRow {
    url: String,
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
            // `view_only` and `autoconnect` are noVNC's own parameters. The pane
            // decides whether the operator can take control by re-rendering with
            // a different value, so the URL carries the read-only default.
            let vnc = self.preview(sandbox, VNC_PORT).await?;
            computer.vnc_url = Some(format!(
                "{}/vnc.html?DAYTONA_SANDBOX_AUTH_KEY={}&autoconnect=1&resize=scale&reconnect=1",
                vnc.url.trim_end_matches('/'),
                vnc.token
            ));

            let terminal = self.preview(sandbox, TERMINAL_PORT).await?;
            computer.terminal_url = Some(format!(
                "{}/?DAYTONA_SANDBOX_AUTH_KEY={}",
                terminal.url.trim_end_matches('/'),
                terminal.token
            ));
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
