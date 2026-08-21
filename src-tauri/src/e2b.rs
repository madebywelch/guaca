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
//!
//! ## A computer is looked at, never asked
//!
//! There used to be a second way to use the web on one of these machines:
//! Chrome was started with its remote debugging port open and driven over the
//! DevTools protocol, which knows where every element is and needs no pointer.
//! Exact when it worked, and it did not work here often enough to keep. The
//! port belongs to a profile, so it was lost every time anything re-attached to
//! that profile; a page had to be read, numbered, and then acted on by a number
//! that a re-render had already invalidated; and an agent reading the screen
//! and an agent asking the page disagreed about which window was in front. Each
//! of those was fixed once and came back wearing another name.
//!
//! So a computer is now exactly what it looks like: a screen, a pointer and a
//! keyboard. `screenshot` and `act_on_desktop` are the whole interface, and
//! there is no privileged channel into the browser for anything to fall out of
//! sync with. An agent that wants a page asked rather than looked at gets a
//! browser instead, which is a different thing on a different provider:
//! `kernel.rs`.

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

/// The screen every machine gets, and the coordinate space every click is in.
///
/// 1024x768 rather than something roomier, and the reason is accuracy rather
/// than bandwidth. A model aiming a pointer is reading pixels off a picture,
/// and both vendors who ship a computer-use tool train and evaluate it at about
/// this size: above it the image is resized somewhere out of Guaca's control
/// and every coordinate that comes back is in a space nothing here can name.
///
/// The alternative is to keep a larger screen and scale the picture on the way
/// out, which is what most harnesses do and what this deliberately does not.
/// Scaling means two coordinate spaces and a conversion between them at every
/// call site, and a conversion that is wrong is a click that lands near the
/// button. One space, chosen so nothing downstream wants to resize it, has no
/// such failure.
///
/// A machine made before this line changed keeps whatever screen it started
/// with: Xvfb is already running there and the guard finds it. That is safe,
/// because the screenshot reports the geometry it actually captured and clicks
/// are always in true screen pixels.
const SCREEN_WIDTH: u16 = 1024;
const SCREEN_HEIGHT: u16 = 768;

/// Reads what the browser is signed in to, from its own files on disk.
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

/// Where the shims go. First on PATH, so a name typed into a shell, an icon on
/// the desktop and a script asking the system for "a browser" all reach the
/// same one.
const LOCAL_BIN: &str = "/home/user/.local/bin";

/// The desktop entries the menu, the file manager and `xdg-open` read. A user
/// entry takes precedence over the packaged one of the same name, which is how
/// a browser that is installed on the machine stops being reachable from it.
const LOCAL_APPS: &str = "/home/user/.local/share/applications";

/// Where the desktop keeps its own answer to "a web browser", and the file that
/// picks which answer is used. Neither names a browser, which is why shadowing
/// entries by the browser they run cannot reach them.
const LOCAL_HELPERS: &str = "/home/user/.local/share/xfce4/helpers";
const XFCE_CONFIG: &str = "/home/user/.config/xfce4";

/// The one browser on a machine. Every other name is rewritten to this, and the
/// wrapper resolves it to whichever browser is actually installed.
const BROWSER: &str = "google-chrome";

/// Every name a browser is launched by on these machines.
///
/// Rewriting the ones that are not Chrome is not pedantry about brands. Only
/// Chrome is on the profile holding the accounts, and only that profile is read
/// when Guaca asks the machine what it is signed in to. A sign-in performed in
/// any other window is one nothing can see: the operator signs in on the
/// screen, the roster keeps saying the agent has no account, and the crew
/// routes work to a machine that will hit a login wall. The template ships that
/// other browser, with an icon, a menu entry and a name on PATH, so declining
/// to use it has to be a property of the machine rather than a line in a
/// prompt.
///
/// The last four are what the *system* reaches for when something asks for a
/// browser without naming one, which is how a link in any other app opens.
const BROWSER_NAMES: [&str; 10] = [
    "google-chrome",
    "google-chrome-stable",
    "chromium",
    "chromium-browser",
    "firefox",
    "firefox-esr",
    "x-www-browser",
    "sensible-browser",
    "www-browser",
    "gnome-www-browser",
];

/// The names the one browser *runs* under. Two, because the wrapper resolves to
/// whichever of them is installed, and both are matched against a command line
/// rather than looked up: `google-chrome` catches `google-chrome-stable`.
const CHROME_PROCESSES: [&str; 2] = ["google-chrome", "chromium"];

/// And the names a browser that is not ours runs under. Stems for the same
/// reason: `firefox` catches `firefox-esr`, and every other name above is a
/// symlink that execs one of these.
const OTHER_PROCESSES: [&str; 1] = ["firefox"];

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
            // window through which the wrong browser can be reached.
            install_browser_shims(),
            evict_other_browsers(),
            daemon(
                "pgrep -x Xvfb",
                "Xvfb",
                &format!(
                    "Xvfb :0 -ac -screen 0 {SCREEN_WIDTH}x{SCREEN_HEIGHT}x24 -dpi 96 \
                     -nolisten tcp"
                ),
            ),
            // The session's PATH is inherited by every icon, menu entry and
            // terminal on that screen, so it is where the shims either shadow
            // the other browsers or do not. Set here rather than trusted to
            // `~/.profile`, which a login shell reads only when there is no
            // `~/.bash_profile` beside it.
            daemon(
                "pgrep -x xfce4-session",
                "xfce4",
                &format!("env DISPLAY=:0 PATH={LOCAL_BIN}:$PATH startxfce4"),
            ),
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

        let program = as_chrome(program);

        self.run(
            sandbox,
            envd_token,
            &format!("(setsid env DISPLAY=:0 {program} >/tmp/guac-desktop-app.log 2>&1 </dev/null &) ; sleep 2; echo started"),
        )
        .await
    }

    /// Does one thing to the screen and photographs the result.
    ///
    /// One method rather than an act and a look, because it is one round trip
    /// rather than two. Every screen action answers with a picture now, so the
    /// pair would be two commands over envd for every click, and the desktop
    /// guard would run twice for each of them: four requests where one does.
    /// The settle between them belongs on the machine for the same reason, and
    /// because a sleep on this side is time spent by a process that is not the
    /// one waiting.
    ///
    /// `action` is `None` for a plain look.
    pub async fn look_at_screen(
        &self,
        sandbox: &str,
        envd_token: &str,
        action: Option<&DesktopAction>,
    ) -> Result<Screen, E2bError> {
        self.start_desktop(sandbox, envd_token).await?;

        // Sequenced with `;` rather than `&&`, so a failed action is still
        // photographed. Whatever went wrong is on the screen, and a model told
        // only that its click failed does it again; shown the modal that
        // swallowed it, it deals with the modal.
        let acted = match action {
            Some(action) => format!(
                "DISPLAY=:0 {} ; echo \"{ACTED}$?\" ; {}",
                action.command(),
                settle_for(action)
            ),
            None => format!("echo \"{ACTED}0\""),
        };

        let out = self
            .run(
                sandbox,
                envd_token,
                &format!(
                    "{acted} ; DISPLAY=:0 scrot --pointer -o /tmp/guac-screen.png \
                     && ffmpeg -y -loglevel error -i /tmp/guac-screen.png -q:v 5 \
                        /tmp/guac-screen.jpg \
                     && echo -n {SIZED} \
                     && (DISPLAY=:0 xdotool getdisplaygeometry | tr ' ' 'x') \
                     && base64 -w0 /tmp/guac-screen.jpg"
                ),
            )
            .await?;

        read_screen(&out).ok_or_else(|| {
            E2bError::Protocol(format!(
                "the screen could not be captured ({})",
                out.stderr.trim().chars().take(200).collect::<String>()
            ))
        })
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
                //
                // The page named here is the app's own rather than noVNC's,
                // which is what lets the frame be rid of noVNC's chrome. It
                // hands these options straight on, so they are still decided
                // here.
                computer.vnc_url = Some(format!(
                    "http://{VIEWER_HOST}:{viewer_port}/{sandbox}/{VNC_PORT}{page}\
                     ?autoconnect=1&resize=scale&reconnect=1",
                    page = crate::proxy::VIEWER_DOCUMENT
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
/// Reads the one profile every route on the machine is pinned to, which is the
/// only one that matters: a session in any other window is one no agent can
/// reach, because no agent can reach that window.
///
/// Reads the files rather than the browser. Cookies are on disk, so a machine
/// that has just woken can be asked without Chrome being started first, and
/// asking a question should not have the side effect of opening a window on
/// somebody's screen.
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

/// What one look at a screen answers with.
#[derive(Debug, Clone, PartialEq)]
pub struct Screen {
    /// A `data:` URL, ready to hand to a model.
    ///
    /// Sent at the display's own resolution on purpose. Scaling it down would
    /// shrink the payload, but every coordinate the model then gives back would
    /// be in a different space from the one clicks land in, and a click that is
    /// subtly wrong is worse than a larger image. The screen is sized so that
    /// nothing wants to scale it: `SCREEN_WIDTH`.
    ///
    /// The pointer is drawn into it. Without that a model has no way to tell a
    /// hover from a click that missed, and no way to see that what it is aiming
    /// at has moved under the cursor.
    ///
    /// JPEG rather than PNG: a desktop screenshot is a photograph-like image,
    /// and PNG costs about four times as much for no benefit a model can use.
    pub image: String,
    /// `1024x768`, read off the display rather than assumed. A machine made
    /// before the screen size changed still has the one it started with.
    pub geometry: String,
    /// What the action before the picture exited with, and zero for a plain
    /// look. Reported rather than turned into an error, because the picture is
    /// usually the explanation.
    pub exit_code: i32,
}

/// Markers the capture command prints, so one reply carries three answers.
const ACTED: &str = "ACT:";
const SIZED: &str = "SIZE:";

/// How long the screen is given to finish changing before it is photographed.
///
/// On the machine rather than here, and only for an action whose effect arrives
/// after the command returns. A click that opens a menu, submits a form or
/// follows a link has not finished doing so when xdotool exits, and a picture
/// taken then shows the screen the model was already looking at, which is
/// indistinguishable to a model from a click that did nothing.
///
/// A pointer move and a wait are finished when they return, and charging them
/// for a delay they cannot use would put it on every action in a batch.
///
/// Short. Not long enough for a page load, which is what `wait` is for and what
/// a model should ask for rather than have guessed at on its behalf.
fn settle_for(action: &DesktopAction) -> &'static str {
    match action {
        DesktopAction::Move { .. } | DesktopAction::Wait { .. } => "",
        _ => "sleep 0.4 ;",
    }
}

/// Reads the three answers back out of one command's output.
///
/// `None` when there is no picture, which is the only part that cannot be
/// missing: a caller with a geometry and no image has nothing to show a model.
fn read_screen(out: &Output) -> Option<Screen> {
    let mut exit_code = 0;
    let mut geometry = None;
    let mut image = None;

    for line in out.stdout.lines() {
        let line = line.trim();
        if let Some(code) = line.strip_prefix(ACTED) {
            exit_code = code.trim().parse().unwrap_or(0);
        } else if let Some(size) = line.strip_prefix(SIZED) {
            geometry = Some(size.trim().to_string());
        } else if geometry.is_some() && !line.is_empty() {
            // Only after the size, and that ordering is the whole check. An
            // action is free to write to stdout, and a picture taken from a
            // line the action printed is a data URL of somebody's shell output.
            // The capture short-circuits on failure, so no size means no
            // picture rather than the wrong one.
            image = Some(line.to_string());
        }
    }

    Some(Screen {
        image: format!("data:image/jpeg;base64,{}", image?),
        geometry: geometry.unwrap_or_default(),
        exit_code,
    })
}

/// How long a chunk of typed text is, and how long xdotool pauses between the
/// keystrokes inside it.
///
/// One `xdotool type` of a long string arrives faster than a page can process
/// it: a React field that re-renders per keystroke dropped characters out of
/// the middle, and a form that validates as you type rejected the half it had.
/// Broken into chunks, each its own invocation, the page gets a gap to catch up
/// in. 12ms between keys is quick enough that a paragraph is not a wait and
/// slow enough that nothing has been observed to drop.
const TYPE_CHUNK: usize = 48;
const TYPE_DELAY_MS: u16 = 12;

/// One thing an agent can do to its screen.
#[derive(Debug, Clone, PartialEq)]
pub enum DesktopAction {
    Click {
        x: i32,
        y: i32,
        button: u8,
        count: u8,
    },
    Move {
        x: i32,
        y: i32,
    },
    /// Press at one point, move to another, release. A slider, a file dragged
    /// onto a drop zone, a selection across a block of text: all of them are
    /// impossible with clicks alone, and a model that has no drag reaches for a
    /// sequence of clicks that does something else.
    Drag {
        from: (i32, i32),
        to: (i32, i32),
    },
    Type {
        text: String,
    },
    Key {
        keys: String,
    },
    Scroll {
        x: i32,
        y: i32,
        down: bool,
        amount: u8,
    },
    /// Do nothing for a moment. The one action whose point is the time it
    /// takes: a page that is still loading is not a page a screenshot can be
    /// read off, and without this the only way to wait is to spend a model call
    /// looking again.
    Wait {
        ms: u32,
    },
}

impl DesktopAction {
    /// The xdotool invocation. Everything the model supplied is quoted, because
    /// this is model output going into a shell.
    ///
    /// `--sync` on every move, and it is load-bearing rather than tidy. xdotool
    /// hands X a request and returns; the click that follows is a separate
    /// request, and the server is free to deliver it before the pointer has
    /// finished moving. On an idle machine the two arrive in order and this
    /// looks like superstition. Under load they do not, and the click lands
    /// wherever the pointer happened to be, which reads as a model that cannot
    /// aim.
    ///
    /// `--clearmodifiers` on typing and keys for the same class of reason: a
    /// modifier left held by an earlier chord turns the next word into a series
    /// of shortcuts, and nothing in a screenshot shows that a key is down.
    pub fn command(&self) -> String {
        match self {
            DesktopAction::Click { x, y, button, count } => {
                format!(
                    "xdotool mousemove --sync {x} {y} click --clearmodifiers --repeat {} {button}",
                    (*count).max(1)
                )
            }
            DesktopAction::Move { x, y } => format!("xdotool mousemove --sync {x} {y}"),
            DesktopAction::Drag { from, to } => format!(
                "xdotool mousemove --sync {} {} mousedown 1 mousemove --sync {} {} \
                 sleep 0.2 mouseup 1",
                from.0, from.1, to.0, to.1
            ),
            // Chunked, so a page that re-renders per keystroke keeps up. `--`
            // stops xdotool reading text that begins with a dash as flags.
            DesktopAction::Type { text } => chunks(text, TYPE_CHUNK)
                .map(|chunk| {
                    format!(
                        "xdotool type --clearmodifiers --delay {TYPE_DELAY_MS} -- {}",
                        quote(chunk)
                    )
                })
                .collect::<Vec<_>>()
                .join(" && "),
            DesktopAction::Key { keys } => {
                format!("xdotool key --clearmodifiers -- {}", quote(keys))
            }
            // Moved first, because a wheel event goes to whatever is under the
            // pointer. Scrolling without aiming scrolls the last thing clicked,
            // which is how a model reading a long page ends up scrolling a
            // sidebar it had no interest in.
            DesktopAction::Scroll { x, y, down, amount } => format!(
                "xdotool mousemove --sync {x} {y} click --repeat {} {}",
                (*amount).max(1),
                if *down { 5 } else { 4 }
            ),
            // `sleep` rather than a Rust delay, so the whole action set is one
            // kind of thing: a string that runs on the machine.
            DesktopAction::Wait { ms } => format!("sleep {}", (*ms as f64 / 1000.0).min(10.0)),
        }
    }
}

/// Splits a string into pieces of at most `size` characters, never mid-char.
fn chunks(text: &str, size: usize) -> impl Iterator<Item = &str> + '_ {
    let mut rest = text;
    std::iter::from_fn(move || {
        if rest.is_empty() {
            return None;
        }
        // A char boundary at or before the limit. Slicing by bytes would panic
        // on any text a person actually types. If the limit falls inside the
        // very first character the whole of it goes, because a chunk of zero
        // characters is a loop that never ends.
        let mut cut = size.min(rest.len());
        while cut > 0 && !rest.is_char_boundary(cut) {
            cut -= 1;
        }
        if cut == 0 {
            cut = rest.chars().next().map(char::len_utf8).unwrap_or(rest.len());
        }
        let (head, tail) = rest.split_at(cut);
        rest = tail;
        Some(head)
    })
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

/// A pattern that matches a running browser without matching the shell doing
/// the matching.
///
/// Every name here appears in the command line of the bash running the
/// eviction, so an unbracketed `pkill -f` kills its own parent halfway through.
/// The first letter is bracketed: the pattern still matches `firefox`, and the
/// literal `[f]irefox` in the shell's own command line does not match it.
fn unmatchable(names: &[&str]) -> String {
    names
        .iter()
        .map(|name| {
            let mut chars = name.chars();
            match chars.next() {
                Some(first) => format!("[{first}]{}", chars.as_str()),
                None => String::new(),
            }
        })
        .collect::<Vec<_>>()
        .join("|")
}

/// Closes any browser on the machine that is not the one agents can use.
///
/// Two shapes of wrong browser, and each is a window that reads as working.
/// Chrome on another profile: Chrome re-attaches to a running instance, so a
/// window left on the old profile would swallow the next sign-in too, and the
/// operator would sign in again into the same invisible jar. And a browser that
/// is not Chrome at all, which the template ships and which an agent can still
/// be looking at from before the shims landed: it holds none of the accounts
/// and sign-in detection cannot see it. The operator's own window is not
/// spared, because a sign-in performed there is one no agent can ever use,
/// which is the failure this whole arrangement exists to stop.
///
/// Precise about which processes it looks at. Chrome's renderers and zygotes
/// are the same binary and do not all carry `--user-data-dir`, so matching them
/// would read the app's own browser as a stray and close it mid-task. Only the
/// main processes are considered, and only when one of them is on a profile
/// that is not ours. Nothing else on the machine has a profile worth sparing.
fn evict_other_browsers() -> String {
    let chrome = unmatchable(&CHROME_PROCESSES);
    let strangers = unmatchable(&OTHER_PROCESSES);
    format!(
        "if pgrep -af '{chrome}' | grep -v -- '--type=' | \
         grep -v -- '--user-data-dir={CHROME_PROFILE}' | grep -q .; then \
         pkill -f '{chrome}' || true; sleep 1; fi; \
         pkill -f '{strangers}' || true"
    )
}

/// Rewrites any browser invocation as the one browser, in the one profile.
///
/// A name that is not Chrome is not refused, it is answered: an agent that asks
/// for a browser wants a web page, and the machine has one browser to give it.
/// Refusing would only teach the model to reach for the same window through
/// `run_command` instead.
///
/// The shim on PATH does this too, and this does it again at the call site,
/// because the two fail differently: the shim covers a name typed into a shell
/// and the icon on the desktop, and this covers the machine whose `~/.profile`
/// does not put `~/.local/bin` first. A duplicated flag with the same value is
/// nothing; a window on the wrong profile is a session no agent can use.
///
/// Chrome also cannot use its own sandbox inside one and refuses to start
/// without being told so, which is why `--no-sandbox` is here.
pub fn as_chrome(program: &str) -> String {
    let trimmed = program.trim();
    let (binary, rest) = trimmed.split_once(char::is_whitespace).unwrap_or((trimmed, ""));
    // The whole first word or nothing. `starts_with` reads `firefox-esr` as
    // `firefox` and leaves `-esr` behind as an argument to Chrome.
    if !BROWSER_NAMES.contains(&binary) {
        return program.to_string();
    }

    // `--password-store=basic` keeps Chrome away from the system keyring.
    // There is no unlocked keyring daemon on these machines, so Chrome asks to
    // create a keyring password: a modal over the window, which an agent
    // reading the screen reports as a fresh profile that is not signed in to
    // anything. It also decides how cookies are encrypted, and a profile
    // written under one store and reopened under another cannot read its own
    // jar, which is a session that silently evaporates. Same flag everywhere,
    // or the profile is only usable by whichever route opened it first.
    let mut command = vec![
        BROWSER.to_string(),
        "--no-sandbox".to_string(),
        "--no-first-run".to_string(),
        "--password-store=basic".to_string(),
        format!("--user-data-dir={CHROME_PROFILE}"),
    ];
    // A caller's own profile or port is dropped rather than kept. Nothing in
    // this app names either, so anything that does is a model asking for a
    // second profile: a window holding no accounts, invisible to sign-in
    // detection, and indistinguishable from a fresh machine. Dropping the
    // caller's flags rather than appending ours after them also means the
    // command line says once what it does, instead of contradicting itself and
    // relying on which end Chrome reads first.
    let args: Vec<String> = rest
        .split_whitespace()
        .filter(|arg| {
            !arg.starts_with("--user-data-dir") && !arg.starts_with("--remote-debugging-port")
        })
        .filter(|arg| !command.iter().any(|flag| flag == arg))
        .map(str::to_string)
        .collect();
    command.extend(args);
    command.join(" ")
}

/// The desktop's own name for a web browser, pointed at ours.
///
/// This is the route that names no browser, and it is the one an agent looking
/// at the screen is most likely to take: the dock along the bottom has a
/// browser button on it, and the entry behind that button runs `exo-open
/// --launch WebBrowser`. Which browser that is lives in `helpers.rc`, three
/// indirections away and in a different file: the shipped answer is
/// `debian-sensible-browser`, which runs `sensible-browser`, which runs the
/// `x-www-browser` alternative, which the template points at Firefox. Nothing
/// in that chain is a browser's name until the last link, so shadowing entries
/// by what they run walks straight past it. `xdg-open` on a URL arrives here
/// too, by the same call.
///
/// The command is absolute rather than a name found on PATH, and that is the
/// point of fixing it here rather than trusting the wrapper. Every other shim
/// wins by being earlier on PATH, and the process reading this one is the panel
/// belonging to a session whose PATH was fixed when it started: these machines
/// sleep and wake for weeks, so a desktop that came up before the shims existed
/// can be corrected by a file and can never be corrected by PATH.
fn web_browser_helper() -> String {
    format!(
        "[Desktop Entry]\n\
         NoDisplay=true\n\
         Version=1.0\n\
         Encoding=UTF-8\n\
         Type=X-XFCE-Helper\n\
         Name=Google Chrome\n\
         Icon=google-chrome\n\
         X-XFCE-Category=WebBrowser\n\
         X-XFCE-Binaries={LOCAL_BIN}/{BROWSER};\n\
         X-XFCE-Commands={LOCAL_BIN}/{BROWSER};\n\
         X-XFCE-CommandsWithParameter={LOCAL_BIN}/{BROWSER} \"%s\";\n"
    )
}

/// Puts one browser on the machine, and makes every route to a browser that
/// one.
///
/// There used to be two profiles, because the browser Guaca drove over the
/// debugging port had to have one to itself. Everything else got the default:
/// an agent's `open_on_desktop`, the icon on the desktop, a `google-chrome` an
/// agent typed into a shell. An operator who signed in on the screen
/// therefore signed in to a browser no agent could use, and nothing said so:
/// detection read the driven profile and truthfully reported an empty jar. The
/// driven profile is gone and the pinning is not, because it is what makes a
/// sign-in visible at all.
///
/// So the name is shadowed rather than the callers being trusted to remember,
/// and every other browser's name is shadowed the same way, because the machine
/// ships one and an agent that finds it uses it. Five routes, five shims: a
/// wrapper earlier on PATH takes the flags with it wherever it is invoked from,
/// symlinks put every other name on that wrapper, a desktop entry in the user's
/// own XDG directory takes precedence over the packaged one of the same name,
/// a launcher sitting on the desktop is rewritten in place, because it is a
/// file rather than an entry anything looks up, and `web_browser_helper` takes
/// the one route that names no browser at all. All of it is written every time
/// the desktop starts, because the alternative is a machine that behaves
/// differently depending on when it was made.
///
/// `helpers.rc` is the one file here edited rather than written. It also says
/// which terminal and which file manager the desktop opens, and this app is
/// neither of those.
///
/// The entries to shadow are read off the machine rather than listed here. A
/// name guessed wrong is a browser still on the menu with nothing reporting it,
/// and the packaged entry is the one route where the file names itself: it is
/// found by what it runs. Ours is written after them, so a machine whose
/// packaged entry has the same name as ours ends up with ours.
fn install_browser_shims() -> String {
    // Resolved past the shim itself: `/usr/bin/google-chrome` is a symlink to
    // the first of these, and calling by name would find the wrapper again.
    let wrapper = format!(
        "#!/bin/sh\n\
         # Guaca: one browser on this machine, the one agents can use.\n\
         for real in /opt/google/chrome/google-chrome /usr/bin/google-chrome-stable \
         /usr/bin/chromium /usr/bin/chromium-browser; do\n\
         \x20 [ -x \"$real\" ] && exec \"$real\" --no-sandbox --no-first-run \
         --password-store=basic --user-data-dir={CHROME_PROFILE} \"$@\"\n\
         done\n\
         echo 'no chrome on this machine' >&2\n\
         exit 127\n"
    );
    let entry = format!(
        "[Desktop Entry]\n\
         Version=1.0\n\
         Type=Application\n\
         Name=Google Chrome\n\
         Exec={LOCAL_BIN}/{BROWSER} %U\n\
         Icon=google-chrome\n\
         Terminal=false\n\
         Categories=Network;WebBrowser;\n\
         MimeType=text/html;x-scheme-handler/http;x-scheme-handler/https;\n"
    );
    // The shadows are the same launcher with the menu item taken away. Hidden
    // outright would delete the association too, and then a link clicked in
    // another app would go looking for the next handler; this way anything that
    // asks for that browser by name still gets this one, and a person or an
    // agent looking at the menu sees one browser on it.
    let shadow = format!("{entry}NoDisplay=true\n");

    let others: Vec<&str> = BROWSER_NAMES.iter().copied().filter(|name| *name != BROWSER).collect();

    // Ours is on the list too. The packaged Chrome launcher runs the real
    // binary by its own path, which is a window on the default profile: the
    // same wrong browser, wearing the right name.
    let stems = [CHROME_PROCESSES.as_slice(), OTHER_PROCESSES.as_slice()].concat().join("|");

    format!(
        "mkdir -p {LOCAL_BIN} {LOCAL_APPS} {LOCAL_HELPERS} {XFCE_CONFIG}; \
         echo {wrapper} | base64 -d > {LOCAL_BIN}/{BROWSER} && chmod +x {LOCAL_BIN}/{BROWSER}; \
         for name in {others}; do ln -sf {LOCAL_BIN}/{BROWSER} {LOCAL_BIN}/$name; done; \
         grep -lriE '^Exec=.*({stems})' /usr/share/applications /usr/local/share/applications \
         2>/dev/null | while read -r packaged; do \
         echo {shadow} | base64 -d > \"{LOCAL_APPS}/$(basename \"$packaged\")\"; done; \
         echo {entry} | base64 -d > {LOCAL_APPS}/{BROWSER}.desktop; \
         for icon in /home/user/Desktop/*.desktop; do \
         grep -qiE '^Exec=.*({stems})' \"$icon\" 2>/dev/null && \
         cp {LOCAL_APPS}/{BROWSER}.desktop \"$icon\"; done; \
         echo {helper} | base64 -d > {LOCAL_HELPERS}/custom-WebBrowser.desktop; \
         touch {XFCE_CONFIG}/helpers.rc; \
         grep -v '^WebBrowser=' {XFCE_CONFIG}/helpers.rc > {XFCE_CONFIG}/helpers.rc.guac; \
         echo 'WebBrowser=custom-WebBrowser' >> {XFCE_CONFIG}/helpers.rc.guac; \
         mv {XFCE_CONFIG}/helpers.rc.guac {XFCE_CONFIG}/helpers.rc; \
         grep -q '.local/bin' ~/.profile 2>/dev/null || \
         echo 'PATH=\"$HOME/.local/bin:$PATH\"' >> ~/.profile; \
         ! [ -f ~/.bash_profile ] || grep -q '.local/bin' ~/.bash_profile || \
         echo 'PATH=\"$HOME/.local/bin:$PATH\"' >> ~/.bash_profile",
        wrapper = base64_encode(wrapper.as_bytes()),
        entry = base64_encode(entry.as_bytes()),
        shadow = base64_encode(shadow.as_bytes()),
        helper = base64_encode(web_browser_helper().as_bytes()),
        others = others.join(" "),
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
    fn every_way_of_opening_a_browser_lands_in_the_profile_agents_can_use() {
        // The bug this closes: an operator signs in on the screen, and the
        // session goes to a profile no agent drives. Nothing errors, and
        // detection truthfully reports an empty jar for the browser it reads.
        for typed in [
            "google-chrome https://mail.google.com",
            "google-chrome-stable",
            "chromium https://example.com",
            // And the one an agent reached for on its own. Naming another
            // browser is not refused, it is answered with the one that works.
            "firefox https://mail.google.com",
            "firefox-esr",
            "x-www-browser https://example.com",
        ] {
            let launched = as_chrome(typed);
            assert!(launched.starts_with(BROWSER), "{typed} -> {launched}");
            assert!(launched.contains(CHROME_PROFILE), "{typed} -> {launched}");
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
            // The name is replaced rather than prefixed. `starts_with` reads
            // `firefox-esr` as `firefox` and hands Chrome an `-esr` it has
            // never heard of.
            for other in ["firefox", "chromium", "www-browser"] {
                assert!(!launched.contains(other), "{typed} -> {launched}");
            }
        }

        // What the agent asked for survives: the page it wanted, on the
        // browser that can open it.
        assert!(as_chrome("firefox https://mail.google.com").ends_with(" https://mail.google.com"));
    }

    #[test]
    fn nothing_on_a_machine_opens_a_remote_interface_into_the_browser() {
        // The debugging port is gone, and it has to stay gone rather than
        // surviving in one of the five places a browser gets launched from.
        // While it existed, a machine had two ways to use the web that
        // disagreed about which window was in front, and every route that
        // forgot the port silently produced a browser the driver could not
        // attach to. A computer is looked at now; a page is asked on a browser,
        // which is `kernel.rs`.
        for launched in [
            as_chrome("google-chrome https://example.com"),
            as_chrome("firefox"),
            install_browser_shims(),
            web_browser_helper(),
        ] {
            assert!(
                !launched.contains("remote-debugging"),
                "a remote interface on a machine is the failure this deleted: {launched}"
            );
        }
    }

    #[test]
    fn every_route_to_a_browser_on_the_machine_is_shimmed_onto_the_one() {
        // The shim covers what the call site cannot see: a name typed into a
        // shell, an icon double-clicked on the screen, and anything that asks
        // the system for a browser without naming one.
        let shim = install_browser_shims();
        assert!(shim.contains(&format!("{LOCAL_BIN}/{BROWSER}")), "{shim}");
        assert!(shim.contains(&format!("{LOCAL_APPS}/{BROWSER}.desktop")), "the menu reads this");
        assert!(shim.contains("/home/user/Desktop/*.desktop"), "so does the icon on the screen");
        assert!(shim.contains("/usr/share/applications"), "and the packaged entries: {shim}");
        for name in BROWSER_NAMES.iter().filter(|name| **name != BROWSER) {
            assert!(shim.contains(name), "{name} is a way to open a browser here: {shim}");
        }
        // Ours last, or a packaged entry with the same name would be shadowed
        // by the copy that is kept off the menu, and the machine would have no
        // browser on it at all.
        let shadowing = shim.find("basename").unwrap_or_default();
        let ours = shim.find(&format!("> {LOCAL_APPS}/{BROWSER}.desktop")).unwrap_or_default();
        assert!(shadowing < ours, "{shim}");
        // A login shell reads `~/.profile` only when there is no
        // `~/.bash_profile`, and on that machine the shims are off PATH and
        // every name resolves to the browser they were meant to shadow.
        assert!(shim.contains("~/.bash_profile"), "{shim}");

        // What actually lands on the machine, rather than what the command
        // looks like: the wrapper and the desktop entries travel base64'd.
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
        // The shadowing entries keep the association and lose the menu item,
        // so the screen offers one browser and a link still opens.
        assert!(written.contains("NoDisplay=true"), "{written}");
        assert!(written.contains(&format!("Exec={LOCAL_BIN}/{BROWSER}")), "{written}");
    }

    #[test]
    fn the_browser_button_on_the_dock_is_shimmed_although_it_names_no_browser() {
        // Observed: an agent read the screen, clicked the browser button on the
        // dock, and Firefox opened on a machine where every browser was
        // supposedly shimmed. That button runs `exo-open --launch WebBrowser`,
        // and no shim looked at it, because every shim here matches on a
        // browser's name and there is not one in that entry. The operator saw
        // Chrome, then Firefox, then Chrome again, the last of those being the
        // eviction pass on the next tool call.
        let helper = web_browser_helper();
        assert!(helper.contains("X-XFCE-Category=WebBrowser"), "{helper}");
        // Absolute. This is read by a session process, and a session that came
        // up before the shims existed has a PATH nothing can now change.
        for key in ["X-XFCE-Commands", "X-XFCE-CommandsWithParameter"] {
            assert!(helper.contains(&format!("{key}={LOCAL_BIN}/{BROWSER}")), "{key}: {helper}");
        }
        assert!(!helper.contains("%B"), "no binary to look up on PATH: {helper}");

        let shim = install_browser_shims();
        assert!(shim.contains(&base64_encode(helper.as_bytes())), "and it is put on the machine");
        assert!(shim.contains(&format!("{LOCAL_HELPERS}/custom-WebBrowser.desktop")), "{shim}");
        // Written where the desktop looks for the answer, and only that answer:
        // the same file says which terminal and which file manager to open, and
        // a machine with no terminal is a worse machine than one with two
        // browsers.
        assert!(shim.contains("WebBrowser=custom-WebBrowser"), "{shim}");
        assert!(shim.contains(&format!("{XFCE_CONFIG}/helpers.rc")), "{shim}");
        assert!(
            shim.contains(&format!("grep -v '^WebBrowser=' {XFCE_CONFIG}/helpers.rc")),
            "every other key in that file survives being rewritten: {shim}"
        );
    }

    #[test]
    fn a_browser_on_the_wrong_profile_or_of_the_wrong_kind_is_ended() {
        // Chrome re-attaches to a running instance, so a window left open on
        // the old profile would swallow the next sign-in too. The filter has to
        // be exact: renderers and zygotes are the same binary and do not all
        // carry the flag, and matching them would close the app's own browser
        // in the middle of a task.
        let evict = evict_other_browsers();
        assert!(evict.contains(&format!("--user-data-dir={CHROME_PROFILE}")), "{evict}");
        assert!(evict.contains("--type="), "helper processes must be excluded: {evict}");
        assert!(evict.contains("pkill"), "{evict}");
        // A browser that is not ours at all has no profile worth sparing: it
        // holds none of the accounts and detection cannot see it.
        assert!(evict.contains("[f]irefox"), "{evict}");
        // And every pattern is bracketed, because each of these names is in the
        // command line of the shell doing the matching: an unbracketed
        // `pkill -f` kills its own parent halfway through the eviction.
        for stem in [CHROME_PROCESSES.as_slice(), OTHER_PROCESSES.as_slice()].concat() {
            assert!(!evict.contains(stem), "{stem} would match the shell running this: {evict}");
        }
    }

    #[test]
    fn a_caller_cannot_ask_for_a_second_profile() {
        // A window on another profile is the failure this file exists to stop,
        // so the caller's own is dropped rather than deduplicated: it used to
        // win, and nothing in the app names one any more.
        let once = as_chrome("google-chrome --no-sandbox --user-data-dir=/tmp/x");
        assert_eq!(once.matches("--no-sandbox").count(), 1, "{once}");
        assert_eq!(once.matches("--user-data-dir").count(), 1, "{once}");
        assert!(once.contains(CHROME_PROFILE), "{once}");
        assert!(!once.contains("/tmp/x"), "{once}");

        // Rewriting an already-rewritten command changes nothing, because both
        // `open_on_desktop` and the runtime that tells the agent what happened
        // run it.
        let twice = as_chrome(&once);
        assert_eq!(twice, once);
    }

    #[test]
    fn a_program_that_is_not_a_browser_is_left_alone() {
        // A document is opened by whatever the machine has for that kind of
        // file, and there are more of those than this runtime knows about.
        assert_eq!(as_chrome("xdg-open /home/user/report.pdf"), "xdg-open /home/user/report.pdf");
        assert_eq!(as_chrome("thunar"), "thunar");
        // A whole word or nothing: this is a text editor, not a browser.
        assert_eq!(as_chrome("firefox-history-reader x"), "firefox-history-reader x");
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
        // Scripts and attachments are written into the sandbox this way, so a
        // wrong encoder is a syntax error in a file nobody looks at.
        for sample in ["", "a", "ab", "abc", "abcd", "hello world", "{\"x\": 1}\n"] {
            assert_eq!(decode(&base64_encode(sample.as_bytes())), sample);
        }
    }

    #[test]
    fn model_supplied_text_cannot_escape_the_shell() {
        // Everything here is written by a model and handed to bash, so a stray
        // quote is a command injection rather than a typo.
        let command = DesktopAction::Type { text: "it's fine; rm -rf /".into() }.command();
        assert!(command.starts_with("xdotool type --clearmodifiers --delay 12 -- "), "{command}");
        // The embedded quote is closed and reopened rather than ending the
        // argument, so the rest stays text instead of becoming a command.
        assert!(command.contains("'it'\\''s fine; rm -rf /'"), "{command}");
        assert_eq!(quote("plain"), "'plain'");
    }

    #[test]
    fn long_text_is_typed_in_pieces_so_a_page_can_keep_up() {
        // One `xdotool type` of a long string outruns a field that re-renders
        // per keystroke: characters go missing out of the middle, and the model
        // sees a screenshot of text it did not write.
        let text = "x".repeat(TYPE_CHUNK * 2 + 5);
        let command = DesktopAction::Type { text }.command();
        assert_eq!(command.matches("xdotool type").count(), 3, "{command}");
        // Sequenced with `&&`, so a chunk that fails stops the rest rather than
        // typing the tail of a sentence into whatever is focused next.
        assert_eq!(command.matches(" && ").count(), 2, "{command}");

        // Every character survives the split, in order, once.
        let typed: String = command
            .split(" && ")
            .map(|piece| piece.trim_start_matches("xdotool type --clearmodifiers --delay 12 -- "))
            .map(|piece| piece.trim_matches('\''))
            .collect();
        assert_eq!(typed.len(), TYPE_CHUNK * 2 + 5);
    }

    #[test]
    fn text_is_split_on_characters_rather_than_bytes() {
        // A model typing an em dash, an emoji or any non-Latin script hands
        // this multi-byte characters, and a byte-indexed split would panic
        // inside one of them.
        let text = "→".repeat(TYPE_CHUNK);
        let command = DesktopAction::Type { text: text.clone() }.command();
        let typed: String = command
            .split(" && ")
            .map(|piece| piece.trim_start_matches("xdotool type --clearmodifiers --delay 12 -- "))
            .map(|piece| piece.trim_matches('\''))
            .collect();
        assert_eq!(typed, text);
    }

    #[test]
    fn a_click_moves_first_and_waits_for_the_pointer_to_land() {
        // `--sync` is the whole point. Without it the move and the click are
        // two requests to X and the second can be delivered first, so a click
        // lands wherever the pointer happened to be: a model that aimed
        // correctly and missed anyway.
        assert_eq!(
            DesktopAction::Click { x: 40, y: 12, button: 1, count: 1 }.command(),
            "xdotool mousemove --sync 40 12 click --clearmodifiers --repeat 1 1"
        );
        assert_eq!(
            DesktopAction::Click { x: 1, y: 2, button: 3, count: 2 }.command(),
            "xdotool mousemove --sync 1 2 click --clearmodifiers --repeat 2 3"
        );
    }

    #[test]
    fn scrolling_down_and_up_are_different_buttons_at_a_named_place() {
        let down = DesktopAction::Scroll { x: 500, y: 400, down: true, amount: 3 }.command();
        let up = DesktopAction::Scroll { x: 500, y: 400, down: false, amount: 3 }.command();
        assert!(down.ends_with(" 5"), "{down}");
        assert!(up.ends_with(" 4"), "{up}");
        // A wheel event goes to whatever is under the pointer, so where it is
        // aimed is part of the action rather than a leftover from the last
        // click: a model reading an article scrolled the sidebar instead.
        assert!(down.contains("mousemove --sync 500 400"), "{down}");
        // A zero repeat is a no-op that reads as a broken tool.
        assert!(DesktopAction::Scroll { x: 1, y: 1, down: true, amount: 0 }
            .command()
            .contains("--repeat 1"));
    }

    #[test]
    fn a_drag_holds_the_button_down_across_the_move() {
        // Press, move, release, in that order and in one invocation. Split
        // across three commands the button is released by the shell exiting
        // between them, and the drag is a click followed by a click.
        let command = DesktopAction::Drag { from: (10, 20), to: (90, 20) }.command();
        assert!(command.starts_with("xdotool mousemove --sync 10 20 mousedown 1"), "{command}");
        assert!(command.contains("mousemove --sync 90 20"), "{command}");
        assert!(command.ends_with("mouseup 1"), "{command}");
    }

    #[test]
    fn only_actions_whose_effect_arrives_later_are_waited_for() {
        // A click that opens a menu, submits a form or follows a link has not
        // finished doing so when xdotool exits, and a picture taken then shows
        // the screen the model was already looking at, which is
        // indistinguishable to a model from a click that did nothing.
        for changes in [
            DesktopAction::Click { x: 1, y: 1, button: 1, count: 1 },
            DesktopAction::Type { text: "hello".into() },
            DesktopAction::Key { keys: "Return".into() },
            DesktopAction::Scroll { x: 1, y: 1, down: true, amount: 3 },
            DesktopAction::Drag { from: (0, 0), to: (9, 9) },
        ] {
            assert!(settle_for(&changes).contains("sleep"), "{changes:?} finishes after it exits");
        }
        // The two that are finished when they return are not charged for a
        // delay they cannot use, which would otherwise be paid on every action
        // in a sequence. A wait has already waited.
        assert_eq!(settle_for(&DesktopAction::Move { x: 1, y: 1 }), "");
        assert_eq!(settle_for(&DesktopAction::Wait { ms: 500 }), "");
    }

    #[test]
    fn a_screen_is_read_out_of_one_command_that_did_three_things() {
        // The action, the settle and the capture are one round trip, so one
        // reply has to carry the action's exit code, the geometry and the
        // picture. `echo -n` runs the marker into the geometry and the capture
        // puts the base64 on its own line.
        let out = Output {
            stdout: "ACT:0\nSIZE:1024x768\nAAAABBBB".into(),
            stderr: String::new(),
            exit_code: 0,
        };
        let screen = read_screen(&out).expect("a picture and a size");
        assert_eq!(screen.geometry, "1024x768");
        assert_eq!(screen.image, "data:image/jpeg;base64,AAAABBBB");
        assert_eq!(screen.exit_code, 0);
    }

    #[test]
    fn a_refused_action_is_reported_and_photographed_anyway() {
        // Sequenced with `;` rather than `&&` on purpose: whatever refused the
        // action is on the screen, and a model told only that its click failed
        // does it again.
        let out = Output {
            stdout: "ACT:1\nSIZE:1024x768\nAAAA".into(),
            stderr: "no such key".into(),
            exit_code: 0,
        };
        let screen = read_screen(&out).expect("a picture even so");
        assert_eq!(screen.exit_code, 1);
        assert_eq!(screen.image, "data:image/jpeg;base64,AAAA");
    }

    #[test]
    fn output_from_the_action_is_never_mistaken_for_a_picture() {
        // An action is free to write to stdout. Reading a picture out of a line
        // it printed hands a model a data URL of somebody's shell output, drawn
        // as a broken image with no error anywhere.
        let noisy = Output {
            stdout: "ACT:0\nxdotool: something\nSIZE:1024x768\nREALIMAGE".into(),
            stderr: String::new(),
            exit_code: 0,
        };
        assert_eq!(
            read_screen(&noisy).unwrap().image,
            "data:image/jpeg;base64,REALIMAGE",
            "only what follows the size is the picture"
        );

        // And a capture that failed short-circuits before printing a size, so
        // there is no picture at all rather than the wrong one.
        let failed = Output {
            stdout: "ACT:0\nxdotool: something".into(),
            stderr: "scrot: failed".into(),
            exit_code: 1,
        };
        assert_eq!(read_screen(&failed), None);
    }

    #[test]
    fn waiting_is_bounded_so_a_model_cannot_hold_a_turn_open() {
        // A model asked to be patient will ask for a minute, and the turn it is
        // spending is the operator's.
        assert_eq!(DesktopAction::Wait { ms: 800 }.command(), "sleep 0.8");
        assert_eq!(DesktopAction::Wait { ms: 600_000 }.command(), "sleep 10");
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
