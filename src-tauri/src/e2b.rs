//! E2B sandboxes: one computer per agent.
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
//!   port 49983 of the sandbox's own hostname. `run` below implements the JSON
//!   form of that protocol directly rather than pulling in a gRPC stack, since
//!   one streaming method covers everything Guac needs.
//!
//! Running a command is the only primitive that matters here. The agent's tool
//! is a command, the operator's terminal is a command, and the desktop itself
//! is four commands, so `run` is what the rest of the file is built from.
//!
//! Replaces an earlier Daytona integration, which was dropped because its
//! sandboxes have no internet access below the Tier 3 plan. An agent that
//! cannot reach the network cannot look anything up, which is most of the point
//! of giving it a computer.

use std::time::Duration;

use serde::{Deserialize, Serialize};

/// E2B's public template with a desktop, a VNC server and noVNC already in it.
const DESKTOP_TEMPLATE: &str = "desktop";

/// Where noVNC serves the desktop, once it has been started.
const VNC_PORT: u16 = 6080;
/// The VNC server noVNC bridges to. Never exposed publicly.
const RAW_VNC_PORT: u16 = 5900;
/// The host the webview loads an agent's desktop from. Named here because the
/// window's CSP has to allow exactly this, and the two silently disagreeing is
/// a blocked iframe that looks identical to a desktop that failed to start.
pub const VIEWER_HOST: &str = "127.0.0.1";

/// envd, the agent daemon inside every sandbox.
const ENVD_PORT: u16 = 49983;

const API_BASE: &str = "https://api.e2b.app";

/// How long a sandbox lives without being touched. E2B bills running sandboxes,
/// and an abandoned desktop is indistinguishable from a busy one.
const SANDBOX_TTL_SECS: u32 = 1800;

/// Long enough for `apt-get install`, short enough that a hung command does not
/// hold an agent's turn open indefinitely.
const RUN_TIMEOUT: Duration = Duration::from_secs(120);

#[derive(Debug, thiserror::Error)]
pub enum E2bError {
    #[error("no E2B API key is set; add one in app settings to give agents a computer")]
    NoKey,
    #[error("E2B request failed: {0}")]
    Transport(String),
    #[error("E2B rejected the request ({status}): {message}")]
    Api { status: u16, message: String },
    #[error("the sandbox replied in a form this build does not understand: {0}")]
    Protocol(String),
}

/// What the UI needs to show an agent's computer.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Computer {
    pub sandbox_id: String,
    /// `running` or `stopped`. E2B has no long-lived stopped state: a sandbox
    /// that is not running has usually been reclaimed.
    pub state: String,
    /// Absent until the desktop has been started inside the sandbox.
    pub vnc_url: Option<String>,
}

impl Computer {
    pub fn is_running(&self) -> bool {
        self.state == "running"
    }
}

/// A freshly created sandbox, with the two tokens that reach it.
///
/// Kept together because they are useless apart: an id without its tokens names
/// a machine nothing is allowed to talk to.
#[derive(Debug, Clone, PartialEq)]
pub struct Sandbox {
    pub id: String,
    pub envd_token: String,
    pub traffic_token: String,
}

/// The result of one command.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Output {
    pub stdout: String,
    pub stderr: String,
    pub exit_code: i32,
}

impl Output {
    /// What the model is shown. Both streams, labelled, with the exit code only
    /// when it is not zero: a successful command should read as its output and
    /// nothing else.
    pub fn rendered(&self) -> String {
        let mut out = String::new();
        if !self.stdout.trim().is_empty() {
            out.push_str(self.stdout.trim_end());
        }
        if !self.stderr.trim().is_empty() {
            if !out.is_empty() {
                out.push('\n');
            }
            out.push_str("stderr: ");
            out.push_str(self.stderr.trim_end());
        }
        if self.exit_code != 0 {
            if !out.is_empty() {
                out.push('\n');
            }
            out.push_str(&format!("(exit code {})", self.exit_code));
        }
        if out.is_empty() {
            out.push_str("(no output)");
        }
        out
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

#[derive(Debug, Clone)]
pub struct E2bClient {
    http: reqwest::Client,
    api_key: String,
}

impl E2bClient {
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

    /// Creates a sandbox for one agent.
    ///
    /// Internet access is on deliberately: without it an agent cannot look
    /// anything up, which is the failure that ended the previous provider.
    ///
    /// Both locks are on too. `secure` makes envd refuse commands without a
    /// token, and `allow_public_traffic: false` does the same for the sandbox's
    /// public URLs. Left open, an agent's desktop is reachable by anyone who
    /// learns its id, and these desktops are meant to hold logged-in sessions.
    pub async fn create(&self, agent: &str) -> Result<Sandbox, E2bError> {
        let row: SandboxRow = self
            .control(self.http.post(format!("{API_BASE}/sandboxes")).json(&create_body(agent)))
            .await?;
        Ok(Sandbox {
            id: row.sandbox_id,
            envd_token: row.envd_token.unwrap_or_default(),
            traffic_token: row.traffic_token.unwrap_or_default(),
        })
    }

    /// Whether this sandbox is still alive.
    ///
    /// E2B has no "get one sandbox" that reports a reclaimed one usefully, so
    /// this asks the running list. A sandbox that is not in it is gone.
    pub async fn is_alive(&self, sandbox: &str) -> Result<bool, E2bError> {
        let rows: Vec<SandboxRow> =
            self.control(self.http.get(format!("{API_BASE}/sandboxes"))).await?;
        Ok(rows.iter().any(|r| r.sandbox_id == sandbox))
    }

    /// Every sandbox this app made, whether or not anything still refers to it.
    ///
    /// Used to sweep up: a sandbox nobody holds a reference to bills exactly as
    /// much as one in use, and is invisible from inside the app.
    pub async fn list_ours(&self) -> Result<Vec<String>, E2bError> {
        let rows: Vec<SandboxRow> =
            self.control(self.http.get(format!("{API_BASE}/sandboxes"))).await?;
        Ok(rows
            .into_iter()
            .filter(|r| r.metadata.get("guac").map(String::as_str) == Some("true"))
            .map(|r| r.sandbox_id)
            .collect())
    }

    pub async fn kill(&self, sandbox: &str) -> Result<(), E2bError> {
        let response = self
            .http
            .delete(format!("{API_BASE}/sandboxes/{sandbox}"))
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
        })
    }

    /// Runs one shell command inside a sandbox and waits for it to finish.
    ///
    /// Speaks Connect RPC's JSON framing by hand. `process.Process/Start` is a
    /// server-streaming method, so the reply is a sequence of length-prefixed
    /// envelopes rather than one document, and the useful parts arrive as
    /// separate events: `data` carries base64 stdout and stderr, `end` carries
    /// the exit code.
    pub async fn run(
        &self,
        sandbox: &str,
        envd_token: &str,
        command: &str,
    ) -> Result<Output, E2bError> {
        let request = serde_json::json!({
            "process": {
                // Through a login shell so PATH and the usual environment are
                // what a person would get, not a bare exec.
                "cmd": "/bin/bash",
                "args": ["-l", "-c", command],
                "cwd": "/home/user",
                "envs": {},
            }
        });

        let response = self
            .http
            .post(format!("{}/process.Process/Start", envd_base(sandbox)))
            .header("content-type", "application/connect+json")
            .header("connect-protocol-version", "1")
            .header("X-Access-Token", envd_token)
            .body(envelope(&serde_json::to_vec(&request).unwrap_or_default()))
            .send()
            .await
            .map_err(|e| E2bError::Transport(e.to_string()))?;

        let status = response.status();
        let body = response.bytes().await.map_err(|e| E2bError::Transport(e.to_string()))?;
        if !status.is_success() {
            return Err(E2bError::Api {
                status: status.as_u16(),
                message: String::from_utf8_lossy(&body).chars().take(200).collect(),
            });
        }

        collect(&body)
    }

    /// Brings up the desktop: framebuffer, session, VNC server, noVNC bridge.
    ///
    /// Every step is idempotent by construction, because the pane asks for a
    /// desktop without tracking whether it has asked before. `pgrep` guards the
    /// ones that would otherwise stack up a second copy.
    pub async fn start_desktop(&self, sandbox: &str, envd_token: &str) -> Result<(), E2bError> {
        for command in [
            daemon(
                "pgrep -x Xvfb",
                "Xvfb",
                "Xvfb :0 -ac -screen 0 1280x800x24 -dpi 96 -nolisten tcp",
            ),
            daemon("pgrep -x xfce4-session", "xfce4", "env DISPLAY=:0 startxfce4"),
            // x11vnc daemonises itself with -bg, so it is started directly.
            format!(
                "pgrep -x x11vnc >/dev/null || x11vnc -bg -display :0 -forever -shared \
                 -rfbport {RAW_VNC_PORT} -nopw >/tmp/guac-x11vnc.log 2>&1 ; sleep 1"
            ),
            // Guarded on the port rather than the process: novnc_proxy is a
            // shell script, so its executable name is the interpreter's and
            // `pgrep -x` cannot see it.
            daemon(
                &port_open(VNC_PORT),
                "novnc",
                &format!(
                    "/opt/noVNC/utils/novnc_proxy --vnc localhost:{RAW_VNC_PORT} \
                     --listen {VNC_PORT} --web /opt/noVNC"
                ),
            ),
        ] {
            self.run(sandbox, envd_token, &command).await?;
        }
        Ok(())
    }

    /// Starts a graphical program on the sandbox's screen.
    ///
    /// Brings the desktop up first, because an agent asked to open a browser
    /// should not have to know that a display exists, and both steps are
    /// idempotent. Detached the same way the desktop's own processes are, so
    /// the window outlives the call that opened it.
    pub async fn open_on_desktop(
        &self,
        sandbox: &str,
        envd_token: &str,
        program: &str,
    ) -> Result<Output, E2bError> {
        self.start_desktop(sandbox, envd_token).await?;

        // Chrome cannot use its own sandbox inside one, and refuses to start
        // without being told so. Harmless for anything else.
        let program = if program.trim_start().starts_with("google-chrome")
            && !program.contains("--no-sandbox")
        {
            program.replacen("google-chrome", "google-chrome --no-sandbox --no-first-run", 1)
        } else {
            program.to_string()
        };

        self.run(
            sandbox,
            envd_token,
            &format!("(setsid env DISPLAY=:0 {program} >/tmp/guac-desktop-app.log 2>&1 </dev/null &) ; sleep 2; echo started"),
        )
        .await
    }

    /// A picture of the screen, as a `data:` URL ready to hand to a model.
    ///
    /// Sent at the display's own resolution on purpose. Scaling it down would
    /// shrink the payload, but every coordinate the model then gives back would
    /// be in a different space from the one clicks land in, and a click that is
    /// subtly wrong is worse than a larger image.
    ///
    /// JPEG rather than PNG: a desktop screenshot is a photograph-like image,
    /// and PNG costs about four times as much for no benefit a model can use.
    pub async fn screenshot(
        &self,
        sandbox: &str,
        envd_token: &str,
    ) -> Result<(String, String), E2bError> {
        self.start_desktop(sandbox, envd_token).await?;

        let out = self
            .run(
                sandbox,
                envd_token,
                "DISPLAY=:0 scrot -o /tmp/guac-screen.png \
                 && ffmpeg -y -loglevel error -i /tmp/guac-screen.png -q:v 5 /tmp/guac-screen.jpg \
                 && echo -n SIZE: && (DISPLAY=:0 xdotool getdisplaygeometry | tr ' ' 'x') \
                 && base64 -w0 /tmp/guac-screen.jpg",
            )
            .await?;

        let (geometry, encoded) = out
            .stdout
            .split_once('\n')
            .map(|(head, rest)| (head.trim_start_matches("SIZE:").trim().to_string(), rest.trim()))
            .unwrap_or_default();

        if encoded.is_empty() {
            return Err(E2bError::Protocol(format!(
                "the screen could not be captured ({})",
                out.stderr.trim()
            )));
        }

        Ok((format!("data:image/jpeg;base64,{encoded}"), geometry))
    }

    /// Drives the mouse and keyboard, the same way E2B's own desktop SDK does.
    pub async fn act_on_desktop(
        &self,
        sandbox: &str,
        envd_token: &str,
        action: &DesktopAction,
    ) -> Result<Output, E2bError> {
        self.start_desktop(sandbox, envd_token).await?;
        self.run(sandbox, envd_token, &format!("DISPLAY=:0 {}", action.command())).await
    }

    /// State plus, once the desktop answers, somewhere to watch it.
    pub async fn describe(
        &self,
        sandbox: &str,
        envd_token: &str,
        viewer_port: u16,
    ) -> Result<Computer, E2bError> {
        let alive = self.is_alive(sandbox).await?;
        let mut computer = Computer {
            sandbox_id: sandbox.to_string(),
            state: if alive { "running" } else { "stopped" }.to_string(),
            vnc_url: None,
        };

        if alive {
            // Asked of the port, not of the process list. A process that exists
            // is not the same as one that is serving, and this check used to
            // match the shell running it: the desktop was reported up when
            // nothing was listening, so the viewer was handed a dead address
            // and drew a black rectangle.
            let up = self
                .run(
                    sandbox,
                    envd_token,
                    &format!("{} 2>/dev/null && echo up || echo down", port_open(VNC_PORT)),
                )
                .await
                .map(|o| o.stdout.trim() == "up")
                .unwrap_or(false);

            if up {
                // Through the local viewer, never straight at E2B: these
                // sandboxes refuse public traffic without a header, and the
                // token that carries it must not reach the webview.
                computer.vnc_url = Some(format!(
                    "http://{VIEWER_HOST}:{viewer_port}/{sandbox}/{VNC_PORT}/vnc.html\
                     ?autoconnect=1&resize=scale&reconnect=1"
                ));
            }
        }

        Ok(computer)
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
fn create_body(agent: &str) -> serde_json::Value {
    serde_json::json!({
        "templateID": DESKTOP_TEMPLATE,
        "timeout": SANDBOX_TTL_SECS,
        // Without this an agent cannot look anything up, which is the failure
        // that ended the previous provider.
        "allow_internet_access": true,
        // envd refuses commands without the token it returns.
        "secure": true,
        // The public ports refuse traffic without the other token it returns.
        "network": { "allowPublicTraffic": false },
        "metadata": { "guac": "true", "guac-agent": agent },
    })
}

/// One thing an agent can do to its screen.
#[derive(Debug, Clone, PartialEq)]
pub enum DesktopAction {
    Click { x: i32, y: i32, button: u8, count: u8 },
    Move { x: i32, y: i32 },
    Type { text: String },
    Key { keys: String },
    Scroll { down: bool, amount: u8 },
}

impl DesktopAction {
    /// The xdotool invocation. Everything the model supplied is quoted, because
    /// this is model output going into a shell.
    pub fn command(&self) -> String {
        match self {
            DesktopAction::Click { x, y, button, count } => {
                format!("xdotool mousemove {x} {y} click --repeat {} {button}", (*count).max(1))
            }
            DesktopAction::Move { x, y } => format!("xdotool mousemove {x} {y}"),
            // `--` stops xdotool reading text that begins with a dash as flags.
            DesktopAction::Type { text } => {
                format!("xdotool type --delay 12 -- {}", quote(text))
            }
            DesktopAction::Key { keys } => format!("xdotool key -- {}", quote(keys)),
            DesktopAction::Scroll { down, amount } => {
                format!("xdotool click --repeat {} {}", (*amount).max(1), if *down { 5 } else { 4 })
            }
        }
    }
}

/// Single-quotes a string for a POSIX shell, including embedded quotes.
fn quote(raw: &str) -> String {
    format!("'{}'", raw.replace('\'', "'\\''"))
}

/// A command that starts a long-lived process if `guard` says it is not up.
///
/// Every part of this is load-bearing and each was learned from a failure.
///
/// `setsid` puts the process in its own session. Without it the process dies
/// the moment the shell that started it exits, so noVNC reported itself running
/// and had vanished a second later.
///
/// Redirecting all three streams stops it holding envd's reply open. envd keeps
/// a call open until the process releases them, so a daemon started without
/// this hangs the command that launched it until it times out.
///
/// The guard must be one that cannot match the shell performing it. `pgrep -f`
/// cannot be used at all here: the pattern and the command being guarded both
/// appear in that shell's own command line, so the check matches itself, every
/// guard reports the process already up, and nothing is ever started. Even the
/// usual `[n]ovnc_proxy` bracket trick fails, because the real path is in the
/// same command line. What is safe is `pgrep -x`, which matches the executable
/// name and therefore only ever sees `bash` for the caller, and asking the port
/// directly, which is the honest question for a service anyway.
fn daemon(guard: &str, name: &str, command: &str) -> String {
    format!(
        "{guard} >/dev/null 2>&1 || \
         (setsid {command} >/tmp/guac-{name}.log 2>&1 </dev/null &) ; sleep 1"
    )
}

/// A test for "is something serving here", using bash's own /dev/tcp so nothing
/// needs to be installed in the sandbox.
fn port_open(port: u16) -> String {
    format!("(exec 3<>/dev/tcp/127.0.0.1/{port})")
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
    const TABLE: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut bits = 0u32;
    let mut have = 0u8;
    let mut out: Vec<u8> = Vec::new();
    for byte in raw.bytes() {
        let Some(index) = TABLE.iter().position(|c| *c == byte) else {
            continue; // padding and whitespace
        };
        bits = (bits << 6) | index as u32;
        have += 6;
        if have >= 8 {
            have -= 8;
            out.push((bits >> have) as u8);
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_window_is_allowed_to_frame_the_viewer() {
        // The viewer moved from E2B's own host to a loopback proxy and the CSP
        // was left behind, so the webview blocked the iframe outright. Every
        // check at the HTTP layer passed, because curl does not enforce CSP,
        // and the screen stayed black.
        let conf: serde_json::Value =
            serde_json::from_str(include_str!("../tauri.conf.json")).expect("tauri.conf.json");
        let csp = conf["app"]["security"]["csp"].as_str().unwrap_or_default();
        let frame_src =
            csp.split(';').find(|part| part.trim().starts_with("frame-src")).unwrap_or_default();
        assert!(
            frame_src.contains(VIEWER_HOST),
            "the window must be allowed to frame {VIEWER_HOST}, got {frame_src:?}"
        );
    }

    #[test]
    fn model_supplied_text_cannot_escape_the_shell() {
        // Everything here is written by a model and handed to bash, so a stray
        // quote is a command injection rather than a typo.
        let command = DesktopAction::Type { text: "it's fine; rm -rf /".into() }.command();
        assert!(command.starts_with("xdotool type --delay 12 -- "), "{command}");
        // The embedded quote is closed and reopened rather than ending the
        // argument, so the rest stays text instead of becoming a command.
        assert!(command.contains("'it'\\''s fine; rm -rf /'"), "{command}");
        assert_eq!(quote("plain"), "'plain'");
    }

    #[test]
    fn a_click_moves_first_so_it_lands_where_the_model_meant() {
        assert_eq!(
            DesktopAction::Click { x: 40, y: 12, button: 1, count: 1 }.command(),
            "xdotool mousemove 40 12 click --repeat 1 1"
        );
        assert_eq!(
            DesktopAction::Click { x: 1, y: 2, button: 3, count: 2 }.command(),
            "xdotool mousemove 1 2 click --repeat 2 3"
        );
    }

    #[test]
    fn scrolling_down_and_up_are_different_buttons() {
        assert!(DesktopAction::Scroll { down: true, amount: 3 }.command().ends_with(" 5"));
        assert!(DesktopAction::Scroll { down: false, amount: 3 }.command().ends_with(" 4"));
        // A zero repeat is a no-op that reads as a broken tool.
        assert!(DesktopAction::Scroll { down: true, amount: 0 }.command().contains("--repeat 1"));
    }

    #[test]
    fn a_daemon_is_detached_from_the_shell_that_starts_it() {
        // Without setsid the process dies with its shell, and without the
        // redirections it holds the RPC open until the call times out. Both
        // failures look like a desktop that never appears.
        let command = daemon("pgrep -x Xvfb", "Xvfb", "Xvfb :0");
        assert!(command.contains("setsid"), "a process that dies with its shell never serves");
        assert!(command.contains("</dev/null"), "holding stdin hangs the call that started it");
        assert!(command.contains(">/tmp/"), "holding stdout hangs it too");
    }

    #[test]
    fn no_guard_can_match_the_shell_that_is_running_it() {
        // The desktop silently started nothing at all because of this. Both the
        // guard pattern and the command being guarded appear in the command line
        // of the shell doing the matching, so any `pgrep -f` matches itself and
        // reports the process already up. The bracket trick does not save it
        // either, because the real path is in that same line.
        let commands = [
            daemon("pgrep -x Xvfb", "Xvfb", "Xvfb :0"),
            daemon(&port_open(6080), "novnc", "/opt/noVNC/utils/novnc_proxy --listen 6080"),
        ];
        for command in commands {
            assert!(
                !command.contains("pgrep -f"),
                "pgrep -f matches the checking shell and starts nothing: {command}"
            );
        }
        assert_eq!(port_open(6080), "(exec 3<>/dev/tcp/127.0.0.1/6080)");
    }

    #[test]
    fn a_new_sandbox_is_created_with_both_locks_and_a_network() {
        // Every one of these has been wrong at least once, and each failure is
        // silent: E2B accepts an unrecognised field and returns a sandbox with
        // no token and its ports wide open.
        let body = create_body("Manager");
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
    }

    #[test]
    fn a_blank_key_means_not_configured_rather_than_a_client_that_always_fails() {
        assert!(E2bClient::new("   ").is_none());
        assert!(E2bClient::new("e2b_x").is_some());
    }

    #[test]
    fn an_envelope_carries_its_length_ahead_of_the_payload() {
        assert_eq!(envelope(b"hi"), vec![0, 0, 0, 0, 2, b'h', b'i']);
    }

    #[test]
    fn base64_round_trips_the_output_of_a_command() {
        // envd sends stdout as base64, so getting this wrong turns every
        // command's output into noise.
        assert_eq!(decode("aGVsbG8gd29ybGQ="), "hello world");
        assert_eq!(decode("eA=="), "x");
        assert_eq!(decode(""), "");
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

    #[test]
    fn rendering_favours_the_output_and_mentions_the_exit_code_only_when_it_matters() {
        let ok = Output { stdout: "72F sunny\n".into(), stderr: String::new(), exit_code: 0 };
        assert_eq!(ok.rendered(), "72F sunny");

        let bad = Output { stdout: String::new(), stderr: "not found".into(), exit_code: 127 };
        assert_eq!(bad.rendered(), "stderr: not found\n(exit code 127)");

        let quiet = Output { stdout: String::new(), stderr: String::new(), exit_code: 0 };
        assert_eq!(quiet.rendered(), "(no output)", "silence must not look like a failure");
    }
}
