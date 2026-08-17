//! E2B sandboxes: one computer per agent, run by somebody else's cloud.
//!
//! An agent that can only emit text is limited to what the model already knows.
//! A sandbox gives it a machine with a shell, a network, and a desktop that can
//! be watched.
//!
//! Two protocols, because E2B uses two:
//!
//! - The control plane at `api.e2b.app` is plain REST with an `X-API-Key`
//!   header: create, list, kill.
//! - Everything *inside* a sandbox goes to `envd`, which speaks Connect RPC on
//!   port 49983 of the sandbox's own hostname. `exec` below implements the JSON
//!   form of that protocol directly rather than pulling in a gRPC stack, since
//!   one streaming method covers everything Guac needs.
//!
//! Running a command is the only primitive that matters here. The agent's tool
//! is a command, the operator's terminal is a command, and the desktop itself
//! is four commands, so `exec` is what everything above this file is built
//! from.
//!
//! Replaces an earlier Daytona integration, which was dropped because its
//! sandboxes have no internet access below the Tier 3 plan. An agent that
//! cannot reach the network cannot look anything up, which is most of the point
//! of giving it a computer.

use std::time::Duration;

use serde::Deserialize;

use crate::computer::provider::{
    ComputerProvider, CreateComputer, ExecRequest, Output, ProviderError, ProviderHandle,
    ProviderState, ViewerTarget,
};
use crate::domain::computer::{Provider, Secret};
use crate::domain::ids::ComputerId;

/// E2B's public template with a desktop, a VNC server and noVNC already in it.
const DESKTOP_TEMPLATE: &str = "desktop";

/// envd, the agent daemon inside every sandbox.
const ENVD_PORT: u16 = 49983;

const API_BASE: &str = "https://api.e2b.app";

/// Long enough for `apt-get install`, short enough that a hung command does not
/// hold an agent's turn open indefinitely. The outer bound: a caller asking for
/// less gets less, and nobody gets more.
const RUN_TIMEOUT: Duration = Duration::from_secs(120);

/// What went wrong at E2B, in E2B's own terms. Private: everything above the
/// boundary reads `ProviderError`, and the conversion below is the only way
/// across.
#[derive(Debug, thiserror::Error)]
enum E2bError {
    #[error("E2B request failed: {0}")]
    Transport(String),
    #[error("E2B rejected the request ({status}): {message}")]
    Api { status: u16, message: String },
    #[error("the sandbox replied in a form this build does not understand: {0}")]
    Protocol(String),
}

impl From<E2bError> for ProviderError {
    /// Transport is the one worth retrying, so it is the one that becomes
    /// `Unavailable`. A rejection and a reply nobody can parse are both
    /// something a person has to look at.
    fn from(err: E2bError) -> Self {
        match err {
            E2bError::Transport(_) => ProviderError::Unavailable(err.to_string()),
            E2bError::Api { .. } | E2bError::Protocol(_) => {
                ProviderError::Operation(err.to_string())
            }
        }
    }
}

/// Only the id is taken. Liveness comes from whether a sandbox appears in the
/// running list at all, which is the one signal E2B reports consistently.
///
/// The field is named explicitly rather than derived: E2B spells it `sandboxID`
/// with a capital D, and `rename_all = "camelCase"` produces `sandboxId`, which
/// matches nothing. That mismatch created a sandbox, failed to read its id, and
/// left it running with nobody holding a reference to it.
#[derive(Debug, Deserialize)]
struct SandboxRow {
    #[serde(rename = "sandboxID", alias = "sandboxId", alias = "sandbox_id")]
    sandbox_id: String,
    /// Present only when the sandbox was created as secure. envd refuses every
    /// request without it.
    #[serde(default, rename = "envdAccessToken")]
    envd_token: Option<String>,
    /// Present only when public traffic is restricted.
    #[serde(default, rename = "trafficAccessToken")]
    traffic_token: Option<String>,
    #[serde(default)]
    metadata: std::collections::HashMap<String, String>,
}

/// E2B, behind the boundary.
///
/// No environment on it. Credentials arrive on each `ExecRequest`, because
/// which agent a command acts for is a property of the command and a provider
/// held per session was one that eight call sites could each forget to rebuild.
pub struct E2bProvider {
    http: reqwest::Client,
    api_key: String,
}

impl std::fmt::Debug for E2bProvider {
    /// The operator's account key is the one thing on here, and a derived
    /// `Debug` prints it in the first log line that names the provider. Same
    /// reason `Secret` says nothing about itself.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("E2bProvider")
    }
}

impl E2bProvider {
    /// `None` when no key is configured, so callers can tell "not set up" apart
    /// from "set up and failing".
    pub fn new(api_key: &str) -> Option<Self> {
        let api_key = api_key.trim();
        if api_key.is_empty() {
            return None;
        }
        let http = reqwest::Client::builder().timeout(RUN_TIMEOUT).build().ok()?;
        Some(Self { http, api_key: api_key.to_string() })
    }

    async fn control<T: for<'de> Deserialize<'de>>(
        &self,
        request: reqwest::RequestBuilder,
    ) -> Result<T, E2bError> {
        let response = request
            .header("X-API-Key", &self.api_key)
            .send()
            .await
            .map_err(|e| E2bError::Transport(e.to_string()))?;

        let status = response.status();
        let body = response.text().await.unwrap_or_default();
        if !status.is_success() {
            let message = serde_json::from_str::<serde_json::Value>(&body)
                .ok()
                .and_then(|v| v["message"].as_str().or(v["error"].as_str()).map(str::to_string))
                .unwrap_or_else(|| body.chars().take(200).collect());
            return Err(E2bError::Api { status: status.as_u16(), message });
        }
        // A 204 has no body, and several of these calls return one.
        if body.trim().is_empty() {
            return serde_json::from_str("null")
                .map_err(|e| E2bError::Protocol(format!("empty reply: {e}")));
        }
        serde_json::from_str(&body)
            .map_err(|e| E2bError::Protocol(format!("could not read E2B's reply: {e}")))
    }
}

#[async_trait::async_trait]
impl ComputerProvider for E2bProvider {
    fn kind(&self) -> Provider {
        Provider::E2b
    }

    /// Creates a sandbox for one agent.
    ///
    /// Internet access is on deliberately: without it an agent cannot look
    /// anything up, which is the failure that ended the previous provider.
    ///
    /// Both locks are on too. `secure` makes envd refuse commands without a
    /// token, and `allow_public_traffic: false` does the same for the sandbox's
    /// public URLs. Left open, an agent's desktop is reachable by anyone who
    /// learns its id, and these desktops are meant to hold logged-in sessions.
    async fn create(&self, request: &CreateComputer) -> Result<ProviderHandle, ProviderError> {
        let row: SandboxRow = self
            .control(self.http.post(format!("{API_BASE}/sandboxes")).json(&create_body(request)))
            .await?;
        Ok(handle(request.computer, row))
    }

    /// What this sandbox is doing, without waking it.
    ///
    /// Asked of the sandbox itself rather than of the running list, because a
    /// sleeping machine is absent from that list and treating it as gone would
    /// throw away the disk this whole feature exists to keep.
    async fn inspect(&self, handle: &ProviderHandle) -> Result<ProviderState, ProviderError> {
        let id = &handle.provider_id;
        let response = self
            .http
            .get(format!("{API_BASE}/sandboxes/{id}"))
            .header("X-API-Key", &self.api_key)
            .send()
            .await
            .map_err(|e| E2bError::Transport(e.to_string()))?;

        let status = response.status();
        if status.as_u16() == 404 {
            return Ok(ProviderState::Gone);
        }
        let body = response.text().await.unwrap_or_default();
        // An expired key or a bad gateway is a rejected request, not a state.
        // Read as one it became "state \"\", which this build does not
        // understand", which tells the operator E2B said something it never
        // said and advises destroying a machine that is fine.
        if !status.is_success() {
            return Err(E2bError::Api {
                status: status.as_u16(),
                message: body.chars().take(200).collect(),
            }
            .into());
        }

        classify(id, &body)
    }

    /// Wakes it, and hands back the handle it now answers to.
    ///
    /// The whole handle is rebuilt from the resume reply, id included: a woken
    /// machine answers to a new name as well as to new tokens, and the manager
    /// persists all of it. Keeping any of the old handle is a machine that is
    /// running and unreachable, which looks exactly like one that is broken.
    async fn start(
        &self,
        handle: &ProviderHandle,
        idle_seconds: u32,
    ) -> Result<ProviderHandle, ProviderError> {
        let id = &handle.provider_id;
        let row: SandboxRow = self
            .control(
                self.http
                    .post(format!("{API_BASE}/sandboxes/{id}/resume"))
                    .json(&serde_json::json!({ "timeout": idle_seconds })),
            )
            .await?;
        Ok(self::handle(handle.computer, row))
    }

    /// Pushes the sleep deadline back to the full idle period.
    ///
    /// Called on every use, which is what turns a fixed lifetime into an idle
    /// timeout. Failure is not worth interrupting an agent for: the worst case
    /// is that the machine sleeps sooner and is woken again. It is still worth
    /// a line, because a machine that has stopped answering this has usually
    /// stopped answering everything.
    async fn keep_awake(&self, handle: &ProviderHandle, idle_seconds: u32) {
        let id = &handle.provider_id;
        let sent = self
            .http
            .post(format!("{API_BASE}/sandboxes/{id}/timeout"))
            .header("X-API-Key", &self.api_key)
            .json(&serde_json::json!({ "timeout": idle_seconds }))
            .send()
            .await;

        match sent {
            Ok(response) if !response.status().is_success() => tracing::debug!(
                sandbox = %id,
                status = response.status().as_u16(),
                "E2B refused to push back the sleep deadline"
            ),
            Err(e) => tracing::debug!(
                sandbox = %id,
                error = %e,
                "could not reach E2B to push back the sleep deadline"
            ),
            Ok(_) => {}
        }
    }

    /// Puts the machine to sleep. The disk is kept; the bill is not.
    ///
    /// Deliberately without its memory. E2B keeps memory by default, which
    /// preserves running processes and open tabs, but a desktop has 8 GiB of it
    /// and that snapshot is stored for as long as the machine sleeps. The disk
    /// is what carries a signed-in browser, and the browser is restarted on the
    /// next use anyway, so this costs a few seconds on waking and saves storing
    /// eight gigabytes per sleeping agent.
    async fn stop(&self, handle: &ProviderHandle) -> Result<(), ProviderError> {
        let id = &handle.provider_id;
        let response = self
            .http
            .post(format!("{API_BASE}/sandboxes/{id}/pause"))
            .header("X-API-Key", &self.api_key)
            .json(&serde_json::json!({ "memory": false }))
            .send()
            .await
            .map_err(|e| E2bError::Transport(e.to_string()))?;
        if response.status().is_success() || response.status().as_u16() == 404 {
            return Ok(());
        }
        Err(E2bError::Api {
            status: response.status().as_u16(),
            message: response.text().await.unwrap_or_default().chars().take(200).collect(),
        }
        .into())
    }

    async fn delete(&self, handle: &ProviderHandle) -> Result<(), ProviderError> {
        let id = &handle.provider_id;
        let response = self
            .http
            .delete(format!("{API_BASE}/sandboxes/{id}"))
            .header("X-API-Key", &self.api_key)
            .send()
            .await
            .map_err(|e| E2bError::Transport(e.to_string()))?;
        // A sandbox that is already gone is the outcome the caller wanted.
        if response.status().is_success() || response.status().as_u16() == 404 {
            return Ok(());
        }
        Err(E2bError::Api {
            status: response.status().as_u16(),
            message: response.text().await.unwrap_or_default().chars().take(200).collect(),
        }
        .into())
    }

    /// Runs one command inside a sandbox and waits for it to finish.
    ///
    /// Speaks Connect RPC's JSON framing by hand. `process.Process/Start` is a
    /// server-streaming method, so the reply is a sequence of length-prefixed
    /// envelopes rather than one document, and the useful parts arrive as
    /// separate events: `data` carries base64 stdout and stderr, `end` carries
    /// the exit code.
    ///
    /// The caller's deadline is enforced here rather than left to the HTTP
    /// client's, which is the outer bound for every call this provider makes.
    async fn exec(
        &self,
        handle: &ProviderHandle,
        request: ExecRequest,
    ) -> Result<Output, ProviderError> {
        let id = &handle.provider_id;
        let call = async {
            let response = self
                .http
                .post(format!("{}/process.Process/Start", envd_base(id)))
                .header("content-type", "application/connect+json")
                .header("connect-protocol-version", "1")
                .header("X-Access-Token", handle.control_secret.expose())
                .body(envelope(&serde_json::to_vec(&process_body(&request)).unwrap_or_default()))
                .send()
                .await
                .map_err(|e| E2bError::Transport(e.to_string()))?;

            let status = response.status();
            let body = response.bytes().await.map_err(|e| E2bError::Transport(e.to_string()))?;
            Ok::<_, E2bError>((status, body))
        };

        // Both halves are inside the deadline: envd answers with headers as
        // soon as the process starts, so a timeout around the send alone would
        // never fire for the command that actually hangs.
        let (status, body) = match tokio::time::timeout(request.timeout, call).await {
            Ok(result) => result?,
            Err(_) => {
                return Err(ProviderError::Unavailable(format!(
                    "the command did not finish within {}s",
                    request.timeout.as_secs()
                )))
            }
        };

        if !status.is_success() {
            return Err(E2bError::Api {
                status: status.as_u16(),
                message: String::from_utf8_lossy(&body).chars().take(200).collect(),
            }
            .into());
        }

        Ok(collect(&body)?)
    }

    async fn viewer_target(
        &self,
        handle: &ProviderHandle,
        port: u16,
    ) -> Result<ViewerTarget, ProviderError> {
        viewer_target(handle, port)
    }

    /// Every sandbox this app made, whether or not anything still refers to it.
    ///
    /// Used to sweep up: a sandbox nobody holds a reference to bills exactly as
    /// much as one in use, and is invisible from inside the app.
    ///
    /// The v2 list is used because it includes sleeping ones. A sleeping orphan
    /// still holds its disk, so listing only the running ones would leave it
    /// billing quietly forever.
    async fn list_owned(&self) -> Result<Vec<String>, ProviderError> {
        let rows: Vec<SandboxRow> =
            self.control(self.http.get(format!("{API_BASE}/v2/sandboxes"))).await?;
        Ok(rows
            .into_iter()
            .filter(|r| r.metadata.get("guac").map(String::as_str) == Some("true"))
            .map(|r| r.sandbox_id)
            .collect())
    }
}

/// The id and the two tokens that reach it, kept together because they are
/// useless apart: an id without its tokens names a machine nothing is allowed
/// to talk to.
fn handle(computer: ComputerId, row: SandboxRow) -> ProviderHandle {
    ProviderHandle {
        computer,
        provider_id: row.sandbox_id,
        control_secret: Secret::new(row.envd_token.unwrap_or_default()),
        viewer_secret: Secret::new(row.traffic_token.unwrap_or_default()),
    }
}

/// The body that creates a locked-down desktop.
///
/// Built here rather than inline so the shape can be asserted. E2B accepts
/// three different casings across this one object and silently ignores a field
/// it does not recognise: `allow_public_traffic` at the top level is accepted
/// and does nothing, and the sandbox comes back with no traffic token and its
/// ports open to anyone who learns the id. The nesting below is the form that
/// actually locks it.
fn create_body(request: &CreateComputer) -> serde_json::Value {
    serde_json::json!({
        "templateID": DESKTOP_TEMPLATE,
        // Counted from the last time the machine was used, because the runtime
        // pushes this forward on every action. What expires is idle time.
        "timeout": request.idle_seconds,
        // Makes that expiry a sleep rather than a death: the disk is kept, so
        // the browser is still signed in when it wakes.
        "autoPause": true,
        // Without this an agent cannot look anything up, which is the failure
        // that ended the previous provider.
        "allow_internet_access": true,
        // envd refuses commands without the token it returns.
        "secure": true,
        // The public ports refuse traffic without the other token it returns.
        "network": { "allowPublicTraffic": false },
        "metadata": {
            "guac": "true",
            // The computer id, not the agent's: a crash between making the
            // sandbox and writing the row leaves a resource whose only link
            // back to this app is what is written here.
            "guac-computer": request.computer.to_string(),
            // For a person reading E2B's own dashboard. Never an identity:
            // agents can be renamed.
            "guac-agent": request.agent_name,
        },
    })
}

/// The body that starts one command, with whatever credentials it should see.
///
/// Built here rather than inline so the environment can be asserted on. A
/// silently empty `envs` is a connector that appears configured everywhere in
/// the app and does nothing on the machine.
fn process_body(request: &ExecRequest) -> serde_json::Value {
    serde_json::json!({
        "process": {
            // Exactly the vector the caller gave. Whatever shell wrapping a
            // command needs happened above the boundary, once, so two providers
            // cannot disagree about it.
            "cmd": request.argv.first().cloned().unwrap_or_default(),
            "args": request.argv.get(1..).unwrap_or_default(),
            "cwd": request.cwd,
            // Passed per command rather than written into a dotfile: a file on
            // the sandbox's disk survives the sleep this app relies on, and
            // would leave tokens on a machine long after the connector holding
            // them was deleted.
            "envs": request.env,
        }
    })
}

/// What E2B's description of a sandbox says it is doing.
///
/// Extracted so the one state this build refuses can be asserted on. Anything
/// outside the two known words is an error rather than `Gone`, because `Gone`
/// is permission to throw a disk away: a state this build has not heard of is
/// more likely a machine that is fine and an app that is old, and E2B has
/// shipped intermediate states before. A body that does not parse arrives here
/// as the same kind of unknown.
fn classify(id: &str, body: &str) -> Result<ProviderState, ProviderError> {
    let state = serde_json::from_str::<serde_json::Value>(body)
        .ok()
        .and_then(|v| v["state"].as_str().map(str::to_string))
        .unwrap_or_default();

    match state.as_str() {
        "running" => Ok(ProviderState::Running),
        "paused" => Ok(ProviderState::Asleep),
        _ => Err(ProviderError::Operation(format!(
            "E2B reports sandbox {id} in state {state:?}, which this build does not \
             understand; try again, and if it persists destroy the computer from its pane"
        ))),
    }
}

/// Where the viewer proxy should connect for one of a sandbox's ports.
///
/// Always TLS on 443: E2B publishes every port of every sandbox as a subdomain,
/// and the port is in the hostname rather than on the socket. The traffic token
/// rides in the head, which is why this is answered to the proxy and never to
/// the webview.
///
/// A sandbox with no traffic token is refused rather than pointed at. Rows
/// carried over from before the tokens existed have an empty one, and sending
/// an empty header is a 403 from E2B: an error the operator can act on, drawn
/// as a broken frame with no explanation.
fn viewer_target(handle: &ProviderHandle, port: u16) -> Result<ViewerTarget, ProviderError> {
    if handle.viewer_secret.is_empty() {
        return Err(ProviderError::Operation(format!(
            "computer {} has no viewer token, so its desktop cannot be shown; it predates the \
             tokens and has to be destroyed and made again from its pane",
            handle.computer
        )));
    }
    Ok(ViewerTarget {
        tls: true,
        host: format!("{port}-{}.e2b.app", handle.provider_id),
        port: 443,
        headers: vec![(
            "e2b-traffic-access-token".to_string(),
            handle.viewer_secret.expose().to_string(),
        )],
    })
}

/// envd's address for one sandbox.
fn envd_base(sandbox: &str) -> String {
    format!("https://{ENVD_PORT}-{sandbox}.e2b.app")
}

/// Wraps a payload in Connect's envelope: a flags byte, then a big-endian
/// length, then the message.
fn envelope(payload: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(payload.len() + 5);
    out.push(0);
    out.extend_from_slice(&(payload.len() as u32).to_be_bytes());
    out.extend_from_slice(payload);
    out
}

/// Reads a stream of Connect envelopes into one result.
///
/// The end-of-stream frame carries an error when something went wrong inside
/// the sandbox, and reporting that is the difference between "the command
/// failed" and silence.
fn collect(body: &[u8]) -> Result<Output, E2bError> {
    let mut stdout = String::new();
    let mut stderr = String::new();
    let mut exit_code = 0;
    let mut cursor = 0usize;

    while cursor + 5 <= body.len() {
        let flags = body[cursor];
        let len = u32::from_be_bytes([
            body[cursor + 1],
            body[cursor + 2],
            body[cursor + 3],
            body[cursor + 4],
        ]) as usize;
        cursor += 5;
        if cursor + len > body.len() {
            return Err(E2bError::Protocol("a frame ran past the end of the reply".into()));
        }
        let payload = &body[cursor..cursor + len];
        cursor += len;

        let value: serde_json::Value = match serde_json::from_slice(payload) {
            Ok(v) => v,
            // A frame that is not JSON is not fatal on its own; the end frame
            // still decides the outcome.
            Err(_) => continue,
        };

        // The high bit marks the end-of-stream frame, which carries trailers
        // rather than an event.
        if flags & 0x02 != 0 {
            if let Some(message) = value["error"]["message"].as_str() {
                return Err(E2bError::Api { status: 500, message: message.to_string() });
            }
            continue;
        }

        let event = &value["event"];
        if let Some(data) = event.get("data") {
            if let Some(chunk) = data["stdout"].as_str() {
                stdout.push_str(&decode(chunk));
            }
            if let Some(chunk) = data["stderr"].as_str() {
                stderr.push_str(&decode(chunk));
            }
        }
        if let Some(end) = event.get("end") {
            // Proto3 JSON omits zero-valued fields, so a missing exitCode is a
            // successful command rather than a missing answer.
            exit_code = end["exitCode"].as_i64().unwrap_or(0) as i32;
        }
    }

    Ok(Output { stdout, stderr, exit_code })
}

/// Connect's JSON mapping sends `bytes` as base64.
fn decode(raw: &str) -> String {
    String::from_utf8_lossy(&super::desktop::decode_bytes(raw)).into_owned()
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use super::*;
    use crate::domain::ids::AgentId;

    fn exec(argv: &[&str], env: BTreeMap<String, String>) -> ExecRequest {
        ExecRequest {
            argv: argv.iter().map(|s| s.to_string()).collect(),
            env,
            cwd: "/home/user".into(),
            timeout: Duration::from_secs(5),
        }
    }

    fn creation(agent_name: &str, idle_seconds: u32) -> CreateComputer {
        CreateComputer {
            computer: ComputerId::new(),
            agent: AgentId::new(),
            agent_name: agent_name.to_string(),
            idle_seconds,
        }
    }

    #[test]
    fn a_machine_sleeps_when_idle_rather_than_dying() {
        // The disk is what carries a signed-in browser between sessions, so an
        // idle machine must pause, not be destroyed.
        let body = create_body(&creation("Manager", 900));
        assert_eq!(body["autoPause"], true, "without this the timeout kills it");
        assert_eq!(body["timeout"], 900, "the idle period, pushed back on every use");
    }

    #[test]
    fn a_new_sandbox_is_created_with_both_locks_and_a_network() {
        // Every one of these has been wrong at least once, and each failure is
        // silent: E2B accepts an unrecognised field and returns a sandbox with
        // no token and its ports wide open.
        let request = creation("Manager", 900);
        let body = create_body(&request);
        assert_eq!(body["secure"], true, "envd must refuse anonymous commands");
        assert_eq!(
            body["network"]["allowPublicTraffic"], false,
            "the top-level spelling is accepted and ignored; only this one locks the ports"
        );
        assert_eq!(
            body["allow_internet_access"], true,
            "an agent that cannot look things up is the bug"
        );
        assert_eq!(body["metadata"]["guac"], "true", "the sweeper finds orphans by this label");
        assert_eq!(body["metadata"]["guac-agent"], "Manager");
        assert_eq!(
            body["metadata"]["guac-computer"],
            request.computer.to_string(),
            "a resource whose row was never completed is recognised by this"
        );
    }

    #[test]
    fn a_command_carries_the_credentials_its_agent_was_given() {
        // Silently empty, every connector in the app looks configured and does
        // nothing on the machine, which reads as the API rejecting the token.
        let env = BTreeMap::from([("GITHUB_TOKEN".to_string(), "ghp_hunter2".to_string())]);

        let body = process_body(&exec(&["/bin/bash", "-l", "-c", "curl -s api.github.com"], env));
        assert_eq!(body["process"]["envs"]["GITHUB_TOKEN"], "ghp_hunter2");
        assert_eq!(body["process"]["args"][2], "curl -s api.github.com");

        // And an agent whose group has none gets exactly what it got before.
        let bare = process_body(&exec(&["/bin/bash", "-l", "-c", "echo hi"], BTreeMap::new()));
        assert_eq!(bare["process"]["envs"], serde_json::json!({}));
    }

    #[test]
    fn a_credential_is_never_written_to_the_machines_disk() {
        // A dotfile would survive the sleep this app relies on, so a token
        // would sit on a sandbox long after the connector holding it was
        // deleted. It goes in the process environment and nowhere else.
        let env = BTreeMap::from([("TOKEN".to_string(), "secret".to_string())]);
        let body = process_body(&exec(&["/bin/bash", "-l", "-c", "echo hi"], env));
        let command = body["process"]["args"][2].as_str().unwrap_or_default();
        assert!(!command.contains("secret"), "the value reached the command line: {command}");
        assert!(!command.contains("TOKEN="), "nothing writes it into a file: {command}");
    }

    #[test]
    fn a_command_is_the_argument_vector_it_was_given() {
        // The provider is handed argv, not shell text. Wrapping happens above it,
        // once, so a second provider cannot wrap differently.
        let request = ExecRequest {
            argv: vec!["/bin/bash".into(), "-l".into(), "-c".into(), "echo $X".into()],
            env: BTreeMap::new(),
            cwd: "/home/user".into(),
            timeout: Duration::from_secs(5),
        };
        let body = process_body(&request);
        assert_eq!(body["process"]["cmd"], "/bin/bash");
        assert_eq!(body["process"]["args"], serde_json::json!(["-l", "-c", "echo $X"]));
        assert_eq!(body["process"]["cwd"], "/home/user");
    }

    #[test]
    fn the_viewer_target_carries_the_traffic_token_over_tls() {
        let handle = ProviderHandle {
            computer: ComputerId::new(),
            provider_id: "sbx1".into(),
            control_secret: Secret::new("envd"),
            viewer_secret: Secret::new("traffic-tok"),
        };
        let target = viewer_target(&handle, 6080).expect("a token makes a target");
        assert!(target.tls);
        assert_eq!(target.host, "6080-sbx1.e2b.app");
        assert_eq!(target.port, 443);
        assert_eq!(
            target.headers,
            vec![("e2b-traffic-access-token".to_string(), "traffic-tok".to_string())]
        );
    }

    #[test]
    fn a_sandbox_with_no_traffic_token_is_refused_rather_than_pointed_at() {
        // Rows carried over from before the tokens existed have an empty one.
        // Forwarding it is an empty header and a 403, which the viewer draws as
        // a black rectangle with nothing said; the operator has to be told the
        // machine predates the tokens and cannot be shown.
        let handle = ProviderHandle {
            computer: ComputerId::new(),
            provider_id: "sbx1".into(),
            control_secret: Secret::new("envd"),
            viewer_secret: Secret::new(""),
        };
        let Err(ProviderError::Operation(message)) = viewer_target(&handle, 6080) else {
            panic!("an empty token must not become a header");
        };
        assert!(message.contains(&handle.computer.to_string()), "which machine: {message}");
        assert!(message.contains("destroyed and made again"), "and what to do: {message}");
    }

    #[test]
    fn a_blank_key_means_not_configured_rather_than_a_client_that_always_fails() {
        assert!(E2bProvider::new("   ").is_none());
        assert!(E2bProvider::new("e2b_x").is_some());
    }

    #[test]
    fn an_e2b_state_this_build_does_not_know_is_an_error_rather_than_a_dead_machine() {
        assert_eq!(classify("sbx1", r#"{"state":"running"}"#).unwrap(), ProviderState::Running);
        assert_eq!(classify("sbx1", r#"{"state":"paused"}"#).unwrap(), ProviderState::Asleep);

        // `Gone` is permission to throw a disk away, and E2B has shipped
        // intermediate states before. Reading one of those as `Gone` would
        // replace a machine that was only busy waking up.
        let Err(ProviderError::Operation(message)) = classify("sbx1", r#"{"state":"restoring"}"#)
        else {
            panic!("an unknown state must not classify as anything");
        };
        assert!(message.contains("sbx1"), "the operator has to know which machine: {message}");
        assert!(message.contains("restoring"), "and what it was told: {message}");

        // A 200 whose body is not the object this expects is the same kind of
        // unknown, and used to read as an empty state string.
        assert!(matches!(classify("sbx1", "<html>hello</html>"), Err(ProviderError::Operation(_))));
    }

    #[test]
    fn every_e2b_failure_crosses_the_boundary_as_the_next_step_it_implies() {
        // The only route across, and each variant is a different thing for
        // whoever reads it to do: wait, or look at the message.
        assert!(matches!(
            ProviderError::from(E2bError::Transport("connection closed".into())),
            ProviderError::Unavailable(m) if m == "E2B request failed: connection closed"
        ));
        assert!(matches!(
            ProviderError::from(E2bError::Api { status: 429, message: "slow down".into() }),
            ProviderError::Operation(m) if m == "E2B rejected the request (429): slow down"
        ));
        assert!(matches!(
            ProviderError::from(E2bError::Protocol("a frame ran past the end".into())),
            ProviderError::Operation(m) if m.contains("a frame ran past the end")
        ));
    }

    #[test]
    fn a_provider_does_not_print_the_account_key_it_holds() {
        // The only credential on this struct is the operator's E2B key, and a
        // derived Debug puts it in the first log line that names the provider.
        // That is one of the two ways a secret has left this process before.
        let provider = E2bProvider::new("e2b_sentinel").expect("a key makes a provider");
        let printed = format!("{provider:?}");
        assert!(!printed.contains("e2b_sentinel"), "{printed}");
    }

    #[test]
    fn an_envelope_carries_its_length_ahead_of_the_payload() {
        assert_eq!(envelope(b"hi"), vec![0, 0, 0, 0, 2, b'h', b'i']);
    }

    fn stream(frames: &[(u8, serde_json::Value)]) -> Vec<u8> {
        let mut out = Vec::new();
        for (flags, value) in frames {
            let payload = serde_json::to_vec(value).unwrap();
            out.push(*flags);
            out.extend_from_slice(&(payload.len() as u32).to_be_bytes());
            out.extend_from_slice(&payload);
        }
        out
    }

    #[test]
    fn output_is_stitched_from_the_events_it_arrives_in() {
        let body = stream(&[
            (0, serde_json::json!({"event": {"start": {"pid": 7}}})),
            (0, serde_json::json!({"event": {"data": {"stdout": "aGVsbG8g"}}})),
            (0, serde_json::json!({"event": {"data": {"stdout": "d29ybGQ="}}})),
            (0, serde_json::json!({"event": {"data": {"stderr": "b29wcw=="}}})),
            (0, serde_json::json!({"event": {"end": {"exitCode": 3, "exited": true}}})),
        ]);
        let out = collect(&body).unwrap();
        assert_eq!(out.stdout, "hello world", "chunks arrive split and must be joined");
        assert_eq!(out.stderr, "oops");
        assert_eq!(out.exit_code, 3);
    }

    #[test]
    fn a_successful_command_reports_exit_zero_even_though_the_field_is_omitted() {
        // Proto3 JSON drops zero values, so a missing exitCode must not read as
        // a missing answer.
        let body = stream(&[(0, serde_json::json!({"event": {"end": {"exited": true}}}))]);
        assert_eq!(collect(&body).unwrap().exit_code, 0);
    }

    #[test]
    fn an_error_in_the_end_frame_is_surfaced_rather_than_swallowed() {
        let body = stream(&[(
            2,
            serde_json::json!({"error": {"code": "internal", "message": "no such file"}}),
        )]);
        assert!(
            matches!(collect(&body), Err(E2bError::Api { message, .. }) if message == "no such file")
        );
    }

    #[test]
    fn a_truncated_frame_is_an_error_rather_than_a_silent_half_answer() {
        let mut body = stream(&[(0, serde_json::json!({"event": {"data": {"stdout": "aGk="}}}))]);
        body.truncate(body.len() - 2);
        assert!(matches!(collect(&body), Err(E2bError::Protocol(_))));
    }
}
