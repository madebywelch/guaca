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

/// Chrome's remote interface, used to drive pages exactly rather than by
/// aiming a pointer at pixels.
const CDP_PORT: u16 = 9222;

/// The driver Guac runs inside the sandbox. Kept as a file so it can be read
/// and tested as Python rather than as a Rust string.
const BROWSER_DRIVER: &str = include_str!("browser.py");

/// Reads what the browser is signed in to, from its own files on disk.
/// Separate from the driver because it deliberately does not need a browser.
const SESSION_READER: &str = include_str!("sessions.py");

/// envd, the agent daemon inside every sandbox.
const ENVD_PORT: u16 = 49983;

const API_BASE: &str = "https://api.e2b.app";

/// Everything Guaca puts on a machine. Spelled absolutely rather than as `~`,
/// because a command daemon that runs as a different user resolves `~`
/// somewhere else, and the failure that produces is not an error: it is a
/// browser profile written in one place and read from another, reporting an
/// empty jar on a machine that is signed in.
const GUAC_DIR: &str = "/home/user/.guac";

/// Where the browser keeps its profile. Under the home directory rather than
/// /tmp so a sign-in survives: this is the whole reason a machine sleeps
/// instead of being destroyed.
const CHROME_PROFILE: &str = "/home/user/.guac/chrome";

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

/// What a machine is doing.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SandboxState {
    Running,
    /// Asleep. The disk is intact and it can be woken.
    Paused,
    /// Reclaimed. Nothing to wake.
    Gone,
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
    /// Credentials put into the environment of every command this client runs.
    ///
    /// Carried on the client rather than threaded through each call because
    /// "which agent is this acting for" is a property of the whole session, and
    /// a parameter on `run` would be one that eight call sites could each
    /// forget. Nothing here is written to the sandbox's disk: it lives in the
    /// process environment of the command that needs it and goes when the
    /// command does.
    env: std::collections::BTreeMap<String, String>,
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
        Some(Self { http, api_key: api_key.to_string(), env: Default::default() })
    }

    /// Hands this client the credentials its agent's group holds.
    ///
    /// The only route a stored secret takes out of the database, and it ends in
    /// a process environment inside a sandbox. It never passes through a
    /// prompt, a transcript, or the webview.
    pub fn with_env(mut self, env: std::collections::BTreeMap<String, String>) -> Self {
        self.env = env;
        self
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
    pub async fn create(&self, agent: &str, idle_seconds: u32) -> Result<Sandbox, E2bError> {
        let row: SandboxRow = self
            .control(
                self.http
                    .post(format!("{API_BASE}/sandboxes"))
                    .json(&create_body(agent, idle_seconds)),
            )
            .await?;
        Ok(Sandbox {
            id: row.sandbox_id,
            envd_token: row.envd_token.unwrap_or_default(),
            traffic_token: row.traffic_token.unwrap_or_default(),
        })
    }

    /// What this sandbox is doing, without waking it.
    ///
    /// Asked of the sandbox itself rather than of the running list, because a
    /// sleeping machine is absent from that list and treating it as gone would
    /// throw away the disk this whole feature exists to keep.
    pub async fn state(&self, sandbox: &str) -> Result<SandboxState, E2bError> {
        let response = self
            .http
            .get(format!("{API_BASE}/sandboxes/{sandbox}"))
            .header("X-API-Key", &self.api_key)
            .send()
            .await
            .map_err(|e| E2bError::Transport(e.to_string()))?;

        if response.status().as_u16() == 404 {
            return Ok(SandboxState::Gone);
        }
        let body = response.text().await.unwrap_or_default();
        let state = serde_json::from_str::<serde_json::Value>(&body)
            .ok()
            .and_then(|v| v["state"].as_str().map(str::to_string))
            .unwrap_or_default();

        Ok(match state.as_str() {
            "running" => SandboxState::Running,
            "paused" => SandboxState::Paused,
            _ => SandboxState::Gone,
        })
    }

    /// Puts the machine to sleep. The disk is kept; the bill is not.
    ///
    /// Deliberately without its memory. E2B keeps memory by default, which
    /// preserves running processes and open tabs, but a desktop has 8 GiB of it
    /// and that snapshot is stored for as long as the machine sleeps. The disk
    /// is what carries a signed-in browser, and the browser is restarted on the
    /// next use anyway, so this costs a few seconds on waking and saves storing
    /// eight gigabytes per sleeping agent.
    pub async fn pause(&self, sandbox: &str) -> Result<(), E2bError> {
        let response = self
            .http
            .post(format!("{API_BASE}/sandboxes/{sandbox}/pause"))
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
        })
    }

    /// Wakes it, and hands back the tokens it now answers to.
    ///
    /// Both tokens are reissued on waking. Keeping the old ones is a machine
    /// that is running and unreachable, which looks exactly like one that is
    /// broken.
    pub async fn resume(&self, sandbox: &str, idle_seconds: u32) -> Result<Sandbox, E2bError> {
        let row: SandboxRow = self
            .control(
                self.http
                    .post(format!("{API_BASE}/sandboxes/{sandbox}/resume"))
                    .json(&serde_json::json!({ "timeout": idle_seconds })),
            )
            .await?;
        Ok(Sandbox {
            id: row.sandbox_id,
            envd_token: row.envd_token.unwrap_or_default(),
            traffic_token: row.traffic_token.unwrap_or_default(),
        })
    }

    /// Pushes the sleep deadline back to the full idle period.
    ///
    /// Called on every use, which is what turns a fixed lifetime into an idle
    /// timeout. Failure is not worth interrupting an agent for: the worst case
    /// is that the machine sleeps sooner and is woken again.
    pub async fn keep_awake(&self, sandbox: &str, idle_seconds: u32) {
        let _ = self
            .http
            .post(format!("{API_BASE}/sandboxes/{sandbox}/timeout"))
            .header("X-API-Key", &self.api_key)
            .json(&serde_json::json!({ "timeout": idle_seconds }))
            .send()
            .await;
    }

    /// Every sandbox this app made, whether or not anything still refers to it.
    ///
    /// Used to sweep up: a sandbox nobody holds a reference to bills exactly as
    /// much as one in use, and is invisible from inside the app.
    ///
    /// The v2 list is used because it includes sleeping ones. A sleeping orphan
    /// still holds its disk, so listing only the running ones would leave it
    /// billing quietly forever.
    pub async fn list_ours(&self) -> Result<Vec<String>, E2bError> {
        let rows: Vec<SandboxRow> =
            self.control(self.http.get(format!("{API_BASE}/v2/sandboxes"))).await?;
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
        let request = process_body(command, &self.env);

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
            // Before anything can be opened on the screen, so there is no
            // window through which the wrong profile can be reached.
            install_chrome_shim(),
            evict_wrong_profile_browser(),
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

        let program = chrome_flags(program);

        self.run(
            sandbox,
            envd_token,
            &format!("(setsid env DISPLAY=:0 {program} >/tmp/guac-desktop-app.log 2>&1 </dev/null &) ; sleep 2; echo started"),
        )
        .await
    }

    /// Makes sure the browser is running with its remote interface open, and
    /// that the driver script is on the machine.
    ///
    /// Chrome ignores `--remote-debugging-port` when it re-attaches to an
    /// existing profile, so the browser Guac drives gets a profile of its own.
    /// Everything here is idempotent; an agent should be able to browse without
    /// knowing any of it happened.
    async fn ensure_browser(&self, sandbox: &str, envd_token: &str) -> Result<(), E2bError> {
        self.start_desktop(sandbox, envd_token).await?;

        let script = base64_encode(BROWSER_DRIVER.as_bytes());
        self.run(
            sandbox,
            envd_token,
            &format!(
                "mkdir -p ~/.guac && echo {script} | base64 -d > ~/.guac/browser.py; \
                 python3 -c 'import websocket' 2>/dev/null || pip install -q websocket-client; \
                 {guard} >/dev/null 2>&1 || (setsid env DISPLAY=:0 google-chrome --no-sandbox \
                 --no-first-run --user-data-dir={CHROME_PROFILE} \
                 --remote-debugging-port={CDP_PORT} about:blank \
                 >/tmp/guac-chrome.log 2>&1 </dev/null &) ; sleep 1",
                guard = port_open(CDP_PORT)
            ),
        )
        .await?;

        // Chrome takes a moment to open the port, and a browse that arrives
        // first fails in a way that reads as the tool being broken.
        for _ in 0..10 {
            let up = self
                .run(
                    sandbox,
                    envd_token,
                    &format!("{} 2>/dev/null && echo up || echo down", port_open(CDP_PORT)),
                )
                .await?;
            if up.stdout.trim() == "up" {
                return Ok(());
            }
        }
        Err(E2bError::Protocol("the browser did not open its remote interface".into()))
    }

    /// One browser action, answered as the driver's JSON.
    pub async fn browse(
        &self,
        sandbox: &str,
        envd_token: &str,
        action: &str,
        args: &serde_json::Value,
    ) -> Result<String, E2bError> {
        self.ensure_browser(sandbox, envd_token).await?;

        let out = self
            .run(
                sandbox,
                envd_token,
                &format!(
                    "python3 ~/.guac/browser.py {action} {}",
                    quote(&serde_json::to_string(args).unwrap_or_else(|_| "{}".into()))
                ),
            )
            .await?;

        if out.exit_code != 0 || out.stdout.trim().is_empty() {
            // The driver reports what went wrong on stderr in words meant for
            // the model, so it is passed through rather than summarised.
            let why = if out.stderr.trim().is_empty() {
                out.stdout.trim().to_string()
            } else {
                out.stderr.trim().to_string()
            };
            return Err(E2bError::Protocol(why.chars().take(300).collect()));
        }
        Ok(out.stdout)
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
        let state = self.state(sandbox).await?;
        let mut computer = Computer {
            sandbox_id: sandbox.to_string(),
            state: match state {
                SandboxState::Running => "running",
                SandboxState::Paused => "asleep",
                SandboxState::Gone => "gone",
            }
            .to_string(),
            vnc_url: None,
        };

        if state == SandboxState::Running {
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
fn create_body(agent: &str, idle_seconds: u32) -> serde_json::Value {
    serde_json::json!({
        "templateID": DESKTOP_TEMPLATE,
        // Counted from the last time the machine was used, because the runtime
        // pushes this forward on every action. What expires is idle time.
        "timeout": idle_seconds,
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
        "metadata": { "guac": "true", "guac-agent": agent },
    })
}

/// The body that starts one command, with whatever credentials it should see.
///
/// Built here rather than inline so the environment can be asserted on. A
/// silently empty `envs` is a connector that appears configured everywhere in
/// the app and does nothing on the machine.
fn process_body(
    command: &str,
    env: &std::collections::BTreeMap<String, String>,
) -> serde_json::Value {
    serde_json::json!({
        "process": {
            // Through a login shell so PATH and the usual environment are what
            // a person would get, not a bare exec.
            "cmd": "/bin/bash",
            "args": ["-l", "-c", command],
            "cwd": "/home/user",
            // Passed per command rather than written into a dotfile: a file on
            // the sandbox's disk survives the sleep this app relies on, and
            // would leave tokens on a machine long after the connector holding
            // them was deleted.
            "envs": env,
        }
    })
}

/// Asks a machine what its browser is signed in to.
///
/// Reads the profile `browse` drives, which is the one that matters: Chrome
/// ignores `--remote-debugging-port` when it re-attaches to an existing
/// profile, so Guaca's browser keeps a profile of its own and a session in any
/// other window is one no agent can use.
///
/// Deliberately not routed through `browse`. Connecting to the browser would
/// start it if it were closed, so merely asking the question would boot Chrome
/// on every machine; and `ensure_browser` costs several seconds it does not
/// need to spend here. Cookies are on disk, so this is one command.
pub async fn signed_in_state(
    client: &E2bClient,
    sandbox: &str,
    envd_token: &str,
) -> Result<crate::domain::signin::BrowserState, E2bError> {
    let script = base64_encode(SESSION_READER.as_bytes());
    let out = client
        .run(
            sandbox,
            envd_token,
            &format!(
                "mkdir -p {GUAC_DIR} && echo {script} | base64 -d > {GUAC_DIR}/sessions.py && \
                 python3 {GUAC_DIR}/sessions.py {CHROME_PROFILE}/Default"
            ),
        )
        .await?;

    serde_json::from_str(out.stdout.trim()).map_err(|e| {
        E2bError::Protocol(format!(
            "could not read what the browser is signed in to ({e}): {}",
            out.stderr.trim().chars().take(200).collect::<String>()
        ))
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

/// Base64, so a script can be written into the sandbox through a shell without
/// any of it being interpreted on the way.
///
/// Public because attachments take the same route onto a machine, and a second
/// encoder would be a second place for the alphabet or the padding to be wrong.
pub fn encode(raw: &[u8]) -> String {
    base64_encode(raw)
}

fn base64_encode(raw: &[u8]) -> String {
    const TABLE: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::new();
    for chunk in raw.chunks(3) {
        let b = [chunk[0], *chunk.get(1).unwrap_or(&0), *chunk.get(2).unwrap_or(&0)];
        let n = ((b[0] as u32) << 16) | ((b[1] as u32) << 8) | b[2] as u32;
        out.push(TABLE[(n >> 18) as usize & 63] as char);
        out.push(TABLE[(n >> 12) as usize & 63] as char);
        out.push(if chunk.len() > 1 { TABLE[(n >> 6) as usize & 63] as char } else { '=' });
        out.push(if chunk.len() > 2 { TABLE[n as usize & 63] as char } else { '=' });
    }
    out
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

/// Closes a browser that is already holding the wrong profile.
///
/// The shim decides where the *next* Chrome goes. A machine made before it, or
/// one where somebody launched Chrome another way, can already have a window
/// open on the old profile, and Chrome re-attaches to a running instance: the
/// operator would sign in again into the same invisible jar and see the same
/// empty list. So a browser on any other profile is ended, once, and whatever
/// opens next opens correctly.
///
/// Precise about which processes it looks at. Chrome's renderers and zygotes
/// are the same binary and do not all carry `--user-data-dir`, so matching them
/// would read the app's own browser as a stray and close it mid-task. Only the
/// main processes are considered, and only when one of them is on a profile
/// that is not ours.
fn evict_wrong_profile_browser() -> String {
    format!(
        "if pgrep -af 'google-chrome|chromium' | grep -v -- '--type=' | \
         grep -v -- '--user-data-dir={CHROME_PROFILE}' | grep -q .; then \
         pkill -f 'google-chrome|chromium' || true; sleep 1; fi"
    )
}

/// Rewrites a browser invocation so it lands in the one profile that counts.
///
/// The shim on PATH does this too, and this does it again at the call site,
/// because the two fail differently: the shim covers a name typed into a shell
/// and the icon on the desktop, and this covers the machine whose `~/.profile`
/// does not put `~/.local/bin` first. A duplicated flag with the same value is
/// nothing; a window on the wrong profile is a session no agent can use.
///
/// Chrome also cannot use its own sandbox inside one and refuses to start
/// without being told so, which is why `--no-sandbox` is here.
fn chrome_flags(program: &str) -> String {
    let trimmed = program.trim_start();
    let Some(binary) = ["google-chrome-stable", "google-chrome", "chromium-browser", "chromium"]
        .into_iter()
        .find(|name| trimmed.starts_with(name))
    else {
        return program.to_string();
    };

    // `--password-store=basic` keeps Chrome away from the system keyring.
    // There is no unlocked keyring daemon on these machines, so Chrome asks to
    // create a keyring password: a modal over the window, which an agent
    // reading the screen reports as a fresh profile that is not signed in to
    // anything. It also decides how cookies are encrypted, and a profile
    // written under one store and reopened under another cannot read its own
    // jar, which is a session that silently evaporates. Same flag everywhere,
    // or the profile is only usable by whichever route opened it first.
    let mut flags = vec!["--no-sandbox", "--no-first-run", "--password-store=basic"];
    let profile = format!("--user-data-dir={CHROME_PROFILE}");
    let port = format!("--remote-debugging-port={CDP_PORT}");
    // Without the port, a window opened here would hold the profile with no
    // remote interface, and `browse` would find Chrome running, re-attach, and
    // never get the port it needs. That is the failure the second profile was
    // invented to avoid, so it has to be closed here rather than reintroduced.
    if !program.contains("--user-data-dir") {
        flags.push(&profile);
    }
    if !program.contains("--remote-debugging-port") {
        flags.push(&port);
    }
    flags.retain(|flag| !program.contains(flag));

    program.replacen(binary, &format!("{binary} {}", flags.join(" ")), 1)
}

/// Puts one Chrome on the machine, and makes every route to it the same one.
///
/// There used to be two profiles. `browse` gave itself one because Chrome
/// ignores `--remote-debugging-port` when it re-attaches to an existing
/// profile; everything else — an agent's `open_on_desktop`, the icon on the
/// desktop, a `google-chrome` an agent typed into a shell — got the default. An
/// operator who signed in on the screen therefore signed in to a browser no
/// agent could use, and nothing said so: detection reads the profile `browse`
/// drives and truthfully reported an empty jar.
///
/// So the name is shadowed rather than the callers being trusted to remember.
/// A wrapper earlier on PATH takes the flags with it wherever it is invoked
/// from, and a desktop entry in the user's own XDG directory takes precedence
/// over the packaged one, which is what the icon and the menu read. Both are
/// written every time the desktop starts, because the alternative is a machine
/// that behaves differently depending on when it was made.
fn install_chrome_shim() -> String {
    // Resolved past the shim itself: `/usr/bin/google-chrome` is a symlink to
    // the first of these, and calling by name would find the wrapper again.
    let wrapper = format!(
        "#!/bin/sh\n\
         # Guaca: one profile on this machine, the one agents can use.\n\
         for real in /opt/google/chrome/google-chrome /usr/bin/google-chrome-stable \
         /usr/bin/chromium /usr/bin/chromium-browser; do\n\
         \x20 [ -x \"$real\" ] && exec \"$real\" --no-sandbox --no-first-run \
         --password-store=basic --user-data-dir={CHROME_PROFILE} \
         --remote-debugging-port={CDP_PORT} \"$@\"\n\
         done\n\
         echo 'no chrome on this machine' >&2\n\
         exit 127\n"
    );
    let entry = "[Desktop Entry]\n\
                 Version=1.0\n\
                 Type=Application\n\
                 Name=Google Chrome\n\
                 Exec=/home/user/.local/bin/google-chrome %U\n\
                 Icon=google-chrome\n\
                 Terminal=false\n\
                 Categories=Network;WebBrowser;\n\
                 MimeType=text/html;x-scheme-handler/http;x-scheme-handler/https;\n";

    format!(
        "mkdir -p /home/user/.local/bin /home/user/.local/share/applications && \
         echo {wrapper} | base64 -d > /home/user/.local/bin/google-chrome && \
         chmod +x /home/user/.local/bin/google-chrome && \
         ln -sf /home/user/.local/bin/google-chrome /home/user/.local/bin/google-chrome-stable && \
         echo {entry} | base64 -d > /home/user/.local/share/applications/google-chrome.desktop && \
         (grep -q '.local/bin' ~/.profile 2>/dev/null || \
          echo 'PATH=\"$HOME/.local/bin:$PATH\"' >> ~/.profile)",
        wrapper = base64_encode(wrapper.as_bytes()),
        entry = base64_encode(entry.as_bytes()),
    )
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
    String::from_utf8_lossy(&decode_bytes(raw)).into_owned()
}

/// The same, kept as bytes.
///
/// Public because a file pulled off a machine comes back this way, and a
/// document is not text: running it through the lossy conversion above would
/// replace every byte that is not valid UTF-8 and hand back a corrupt file that
/// looks fine until somebody opens it.
pub fn decode_bytes(raw: &str) -> Vec<u8> {
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
    out
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
    fn every_way_of_opening_chrome_lands_in_the_profile_agents_can_use() {
        // The bug this closes: an operator signs in on the screen, and the
        // session goes to a profile no agent drives. Nothing errors, and
        // detection truthfully reports an empty jar for the browser it reads.
        for typed in [
            "google-chrome https://mail.google.com",
            "google-chrome-stable",
            "chromium https://example.com",
        ] {
            let launched = chrome_flags(typed);
            assert!(launched.contains(CHROME_PROFILE), "{typed} -> {launched}");
            assert!(
                launched.contains(&format!("--remote-debugging-port={CDP_PORT}")),
                "a window without the port re-attaches and leaves browse with no interface: \
                 {launched}"
            );
            assert!(launched.contains("--no-sandbox"), "{launched}");
            // Observed: Chrome opened on the screen, asked to create a system
            // keyring password, and the agent driving it reported that the
            // profile was fresh and Gmail unavailable. The flag also fixes how
            // the cookie jar is encrypted, so a session survives being reopened
            // by a different route.
            assert!(
                launched.contains("--password-store=basic"),
                "without this Chrome blocks on a keyring prompt: {launched}"
            );
        }

        // The shim covers what the call site cannot see: a name typed into a
        // shell, and the icon on the desktop.
        let shim = install_chrome_shim();
        assert!(shim.contains("/home/user/.local/bin/google-chrome"), "{shim}");
        assert!(shim.contains("applications/google-chrome.desktop"), "the icon reads this one");

        // What actually lands on the machine, rather than what the command
        // looks like: the wrapper and the desktop entry travel base64'd.
        let written: String = shim
            .split_whitespace()
            .filter(|token| token.len() > 40)
            .map(decode)
            .collect::<Vec<_>>()
            .join("\n");
        assert!(written.contains("exec"), "the wrapper should have decoded: {written}");
        // The operator's own click has to land on the same store as an agent's
        // launch, or one of them writes a cookie jar the other cannot read.
        assert!(written.contains("--password-store=basic"), "{written}");
        assert!(written.contains(CHROME_PROFILE), "{written}");
    }

    #[test]
    fn a_browser_already_on_the_wrong_profile_is_ended_but_ours_is_left_alone() {
        // Chrome re-attaches to a running instance, so a window left open on
        // the old profile would swallow the next sign-in too. The filter has to
        // be exact: renderers and zygotes are the same binary and do not all
        // carry the flag, and matching them would close the app's own browser
        // in the middle of a task.
        let evict = evict_wrong_profile_browser();
        assert!(evict.contains(&format!("--user-data-dir={CHROME_PROFILE}")), "{evict}");
        assert!(evict.contains("--type="), "helper processes must be excluded: {evict}");
        assert!(evict.contains("pkill"), "{evict}");
    }

    #[test]
    fn a_flag_the_caller_already_set_is_not_set_twice() {
        let once = chrome_flags("google-chrome --no-sandbox --user-data-dir=/tmp/x");
        assert_eq!(once.matches("--no-sandbox").count(), 1, "{once}");
        assert_eq!(once.matches("--user-data-dir").count(), 1, "{once}");
        // And a caller that named its own profile keeps it: only `browse` does
        // that, and it names this one.
        assert!(once.contains("--user-data-dir=/tmp/x"), "{once}");
    }

    #[test]
    fn a_program_that_is_not_a_browser_is_left_alone() {
        assert_eq!(
            chrome_flags("xdg-open /home/user/report.pdf"),
            "xdg-open /home/user/report.pdf"
        );
        assert_eq!(chrome_flags("thunar"), "thunar");
    }

    #[test]
    fn the_session_reader_is_pointed_at_the_profile_the_browser_actually_uses() {
        // These two have to name one directory. When they did not, nothing
        // failed: the browser wrote its cookies where it was told and the
        // reader looked somewhere else, so every machine came back signed in to
        // nothing and no error was raised anywhere. A path is a contract
        // between two files here, so it is passed in rather than spelled twice.
        assert!(
            SESSION_READER.contains("sys.argv[1]"),
            "the reader must take the profile from its caller, not guess at it"
        );
        assert!(
            !SESSION_READER.contains("expanduser"),
            "`~` resolves against whoever runs the command, which is not who runs the browser"
        );
        assert!(CHROME_PROFILE.starts_with(GUAC_DIR), "the profile lives inside Guaca's directory");
        assert!(CHROME_PROFILE.starts_with('/'), "an absolute path, for the same reason");
    }

    #[test]
    fn base64_round_trips_through_the_decoder_it_is_paired_with() {
        // The driver script is written into the sandbox this way, so a wrong
        // encoder is a syntax error in a file nobody looks at.
        for sample in ["", "a", "ab", "abc", "abcd", "hello world", "{\"x\": 1}\n"] {
            assert_eq!(decode(&base64_encode(sample.as_bytes())), sample);
        }
    }

    #[test]
    fn the_browser_driver_is_shipped_with_the_binary() {
        assert!(BROWSER_DRIVER.contains("__guacEls"), "the driver must be the real script");
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
    fn a_machine_sleeps_when_idle_rather_than_dying() {
        // The disk is what carries a signed-in browser between sessions, so an
        // idle machine must pause, not be destroyed.
        let body = create_body("Manager", 900);
        assert_eq!(body["autoPause"], true, "without this the timeout kills it");
        assert_eq!(body["timeout"], 900, "the idle period, pushed back on every use");
    }

    #[test]
    fn a_new_sandbox_is_created_with_both_locks_and_a_network() {
        // Every one of these has been wrong at least once, and each failure is
        // silent: E2B accepts an unrecognised field and returns a sandbox with
        // no token and its ports wide open.
        let body = create_body("Manager", 900);
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
    fn a_command_carries_the_credentials_its_agent_was_given() {
        // Silently empty, every connector in the app looks configured and does
        // nothing on the machine, which reads as the API rejecting the token.
        let mut env = std::collections::BTreeMap::new();
        env.insert("GITHUB_TOKEN".to_string(), "ghp_hunter2".to_string());

        let body = process_body("curl -s api.github.com", &env);
        assert_eq!(body["process"]["envs"]["GITHUB_TOKEN"], "ghp_hunter2");
        assert_eq!(body["process"]["args"][2], "curl -s api.github.com");

        // And an agent whose group has none gets exactly what it got before.
        let bare = process_body("echo hi", &Default::default());
        assert_eq!(bare["process"]["envs"], serde_json::json!({}));
    }

    #[test]
    fn a_credential_is_never_written_to_the_machines_disk() {
        // A dotfile would survive the sleep this app relies on, so a token
        // would sit on a sandbox long after the connector holding it was
        // deleted. It goes in the process environment and nowhere else.
        let mut env = std::collections::BTreeMap::new();
        env.insert("TOKEN".to_string(), "secret".to_string());
        let body = process_body("echo hi", &env);
        let command = body["process"]["args"][2].as_str().unwrap_or_default();
        assert!(!command.contains("secret"), "the value reached the command line: {command}");
        assert!(!command.contains("TOKEN="), "nothing writes it into a file: {command}");
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
