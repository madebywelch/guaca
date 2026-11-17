use std::time::Duration;

use serde::{Deserialize, Serialize};

const DESKTOP_TEMPLATE: &str = "desktop";

const VNC_PORT: u16 = 6080;
const RAW_VNC_PORT: u16 = 5900;
pub const VIEWER_HOST: &str = "127.0.0.1";

const SCREEN_WIDTH: u16 = 1024;
const SCREEN_HEIGHT: u16 = 768;

const SESSION_READER: &str = include_str!("sessions.py");

const ENVD_PORT: u16 = 49983;

const API_BASE: &str = "https://api.e2b.app";

fn api_base() -> String {
    match std::env::var("GUAC_E2B_API_BASE") {
        Ok(base) if !base.trim().is_empty() => base.trim().trim_end_matches('/').to_string(),
        _ => API_BASE.to_string(),
    }
}

const GUAC_DIR: &str = "/home/user/.guac";

const CHROME_PROFILE: &str = "/home/user/.guac/chrome";

const LOCAL_BIN: &str = "/home/user/.local/bin";

const LOCAL_APPS: &str = "/home/user/.local/share/applications";

const LOCAL_HELPERS: &str = "/home/user/.local/share/xfce4/helpers";
const XFCE_CONFIG: &str = "/home/user/.config/xfce4";

const BROWSER: &str = "google-chrome";

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

const CHROME_PROCESSES: [&str; 2] = ["google-chrome", "chromium"];

const OTHER_PROCESSES: [&str; 1] = ["firefox"];

const RUN_TIMEOUT: Duration = Duration::from_secs(120);

#[derive(Debug, thiserror::Error)]
pub enum E2bError {
    #[error("no E2B API key is set; add one in app settings to give agents a computer")]
    NoKey,
    #[error("this agent has not been given a computer; the operator gives one from its panel")]
    NotGiven,
    #[error("E2B request failed: {0}")]
    Transport(String),
    #[error("E2B rejected the request ({status}): {message}")]
    Api { status: u16, message: String },
    #[error("the sandbox replied in a form this build does not understand: {0}")]
    Protocol(String),
    #[error("Guaca could not use its own records: {0}")]
    Local(String),
    #[error("The computer provider did not confirm the action's outcome. It may have happened, so check before doing it again.")]
    OutcomeUnknown,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Computer {
    pub sandbox_id: String,
    pub state: String,
    pub vnc_url: Option<String>,
}

impl Computer {
    pub fn is_running(&self) -> bool {
        self.state == "running"
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SandboxState {
    Running,
    Paused,
    Gone,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Sandbox {
    pub id: String,
    pub envd_token: String,
    pub traffic_token: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Output {
    pub stdout: String,
    pub stderr: String,
    pub exit_code: i32,
}

impl Output {
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

#[derive(Debug, Deserialize)]
struct SandboxRow {
    #[serde(rename = "sandboxID", alias = "sandboxId", alias = "sandbox_id")]
    sandbox_id: String,
    #[serde(default, rename = "envdAccessToken")]
    envd_token: Option<String>,
    #[serde(default, rename = "trafficAccessToken")]
    traffic_token: Option<String>,
    #[serde(default)]
    metadata: std::collections::HashMap<String, String>,
}

#[derive(Debug, Clone)]
pub struct E2bClient {
    http: reqwest::Client,
    api_key: String,
    env: std::sync::Arc<crate::secrets::ProcessEnv>,
    base: String,
}

impl E2bClient {
    pub fn new(api_key: &str) -> Option<Self> {
        let api_key = api_key.trim();
        if api_key.is_empty() {
            return None;
        }
        let http = reqwest::Client::builder().timeout(RUN_TIMEOUT).build().ok()?;
        Some(Self { http, api_key: api_key.to_string(), env: Default::default(), base: api_base() })
    }

    pub fn with_env(mut self, env: crate::secrets::ProcessEnv) -> Self {
        self.env = std::sync::Arc::new(env);
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
        if body.trim().is_empty() {
            return serde_json::from_str("null")
                .map_err(|e| E2bError::Protocol(format!("empty reply: {e}")));
        }
        serde_json::from_str(&body)
            .map_err(|e| E2bError::Protocol(format!("could not read E2B's reply: {e}")))
    }

    pub async fn create(&self, agent: &str, idle_seconds: u32) -> Result<Sandbox, E2bError> {
        self.create_with_workspace(agent, idle_seconds, None).await
    }

    pub async fn create_for_workspace(
        &self,
        agent: &str,
        idle_seconds: u32,
        workspace: &str,
    ) -> Result<Sandbox, E2bError> {
        self.create_with_workspace(agent, idle_seconds, Some(workspace)).await
    }

    async fn create_with_workspace(
        &self,
        agent: &str,
        idle_seconds: u32,
        workspace: Option<&str>,
    ) -> Result<Sandbox, E2bError> {
        let mut body = create_body(agent, idle_seconds);
        if let Some(workspace) = workspace {
            body["metadata"]["guac-workspace"] = serde_json::json!(workspace);
        }
        let row: SandboxRow =
            self.control(self.http.post(format!("{}/sandboxes", self.base)).json(&body)).await?;
        Ok(Sandbox {
            id: row.sandbox_id,
            envd_token: row.envd_token.unwrap_or_default(),
            traffic_token: row.traffic_token.unwrap_or_default(),
        })
    }

    pub async fn state(&self, sandbox: &str) -> Result<SandboxState, E2bError> {
        let response = self
            .http
            .get(format!("{}/sandboxes/{sandbox}", self.base))
            .header("X-API-Key", &self.api_key)
            .send()
            .await
            .map_err(|e| E2bError::Transport(e.to_string()))?;

        let status = response.status();
        if status.as_u16() == 404 {
            return Ok(SandboxState::Gone);
        }
        let body = response.text().await.map_err(|e| E2bError::Transport(e.to_string()))?;
        if !status.is_success() {
            let message = serde_json::from_str::<serde_json::Value>(&body)
                .ok()
                .and_then(|v| v["message"].as_str().or(v["error"].as_str()).map(str::to_string))
                .unwrap_or_else(|| body.chars().take(200).collect());
            return Err(E2bError::Api { status: status.as_u16(), message });
        }
        let value: serde_json::Value = serde_json::from_str(&body)
            .map_err(|e| E2bError::Protocol(format!("could not read E2B's reply: {e}")))?;
        match value.get("state").and_then(|state| state.as_str()) {
            Some("running") => Ok(SandboxState::Running),
            Some("paused") => Ok(SandboxState::Paused),
            _ => Err(E2bError::Protocol("unrecognized sandbox state".into())),
        }
    }

    pub async fn pause(&self, sandbox: &str) -> Result<(), E2bError> {
        let response = self
            .http
            .post(format!("{}/sandboxes/{sandbox}/pause", self.base))
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

    pub async fn resume(&self, sandbox: &str, idle_seconds: u32) -> Result<Sandbox, E2bError> {
        let row: SandboxRow = self
            .control(
                self.http
                    .post(format!("{}/sandboxes/{sandbox}/resume", self.base))
                    .json(&serde_json::json!({ "timeout": idle_seconds })),
            )
            .await?;
        Ok(Sandbox {
            id: row.sandbox_id,
            envd_token: row.envd_token.unwrap_or_default(),
            traffic_token: row.traffic_token.unwrap_or_default(),
        })
    }

    pub async fn keep_awake(&self, sandbox: &str, idle_seconds: u32) {
        let _ = self
            .http
            .post(format!("{}/sandboxes/{sandbox}/timeout", self.base))
            .header("X-API-Key", &self.api_key)
            .json(&serde_json::json!({ "timeout": idle_seconds }))
            .send()
            .await;
    }

    pub async fn list_ours(&self) -> Result<Vec<String>, E2bError> {
        let rows: Vec<SandboxRow> =
            self.control(self.http.get(format!("{}/v2/sandboxes", self.base))).await?;
        Ok(rows
            .into_iter()
            .filter(|r| r.metadata.get("guac").map(String::as_str) == Some("true"))
            .map(|r| r.sandbox_id)
            .collect())
    }

    pub async fn list_for_workspace(
        &self,
        workspace: &str,
    ) -> Result<Vec<(String, bool)>, E2bError> {
        let rows: Vec<SandboxRow> =
            self.control(self.http.get(format!("{}/v2/sandboxes", self.base))).await?;
        Ok(rows
            .into_iter()
            .filter(|row| row.metadata.get("guac").map(String::as_str) == Some("true"))
            .map(|row| {
                (
                    row.sandbox_id,
                    row.metadata.get("guac-workspace").map(String::as_str) == Some(workspace),
                )
            })
            .collect())
    }

    pub async fn kill(&self, sandbox: &str) -> Result<(), E2bError> {
        let response = self
            .http
            .delete(format!("{}/sandboxes/{sandbox}", self.base))
            .header("X-API-Key", &self.api_key)
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
                message: self.env.quote(&String::from_utf8_lossy(&body), 200),
            });
        }

        collect_private(&body, &self.env)
    }

    pub async fn start_desktop(&self, sandbox: &str, envd_token: &str) -> Result<(), E2bError> {
        for command in [
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
            daemon(
                "pgrep -x xfce4-session",
                "xfce4",
                &format!("env DISPLAY=:0 PATH={LOCAL_BIN}:$PATH startxfce4"),
            ),
            format!(
                "pgrep -x x11vnc >/dev/null || x11vnc -bg -display :0 -forever -shared \
                 -rfbport {RAW_VNC_PORT} -nopw >/tmp/guac-x11vnc.log 2>&1 ; sleep 1"
            ),
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

    pub async fn look_at_screen(
        &self,
        sandbox: &str,
        envd_token: &str,
        action: Option<&DesktopAction>,
    ) -> Result<Screen, E2bError> {
        self.start_desktop(sandbox, envd_token).await?;

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

fn create_body(agent: &str, idle_seconds: u32) -> serde_json::Value {
    serde_json::json!({
        "templateID": DESKTOP_TEMPLATE,
        "timeout": idle_seconds,
        "autoPause": true,
        "allow_internet_access": true,
        "secure": true,
        "network": { "allowPublicTraffic": false },
        "metadata": { "guac": "true", "guac-agent": agent },
    })
}

fn process_body(command: &str, env: &crate::secrets::ProcessEnv) -> serde_json::Value {
    serde_json::json!({
        "process": {
            "cmd": "/bin/bash",
            "args": ["-l", "-c", command],
            "cwd": "/home/user",
            "envs": env.process_envs(),
        }
    })
}

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

#[derive(Debug, Clone, PartialEq)]
pub struct Screen {
    pub image: String,
    pub geometry: String,
    pub exit_code: i32,
}

const ACTED: &str = "ACT:";
const SIZED: &str = "SIZE:";

fn settle_for(action: &DesktopAction) -> &'static str {
    match action {
        DesktopAction::Move { .. } | DesktopAction::Wait { .. } => "",
        _ => "sleep 0.4 ;",
    }
}

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
            image = Some(line.to_string());
        }
    }

    Some(Screen {
        image: format!("data:image/jpeg;base64,{}", image?),
        geometry: geometry.unwrap_or_default(),
        exit_code,
    })
}

const TYPE_CHUNK: usize = 48;
const TYPE_DELAY_MS: u16 = 12;

#[derive(Debug, Clone, PartialEq)]
pub enum DesktopAction {
    Click { x: i32, y: i32, button: u8, count: u8 },
    Move { x: i32, y: i32 },
    Drag { from: (i32, i32), to: (i32, i32) },
    Type { text: String },
    Key { keys: String },
    Scroll { x: i32, y: i32, down: bool, amount: u8 },
    Wait { ms: u32 },
}

impl DesktopAction {
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
            DesktopAction::Scroll { x, y, down, amount } => format!(
                "xdotool mousemove --sync {x} {y} click --repeat {} {}",
                (*amount).max(1),
                if *down { 5 } else { 4 }
            ),
            DesktopAction::Wait { ms } => format!("sleep {}", (*ms as f64 / 1000.0).min(10.0)),
        }
    }
}

fn chunks(text: &str, size: usize) -> impl Iterator<Item = &str> + '_ {
    let mut rest = text;
    std::iter::from_fn(move || {
        if rest.is_empty() {
            return None;
        }
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

fn quote(raw: &str) -> String {
    format!("'{}'", raw.replace('\'', "'\\''"))
}

fn daemon(guard: &str, name: &str, command: &str) -> String {
    format!(
        "{guard} >/dev/null 2>&1 || \
         (setsid {command} >/tmp/guac-{name}.log 2>&1 </dev/null &) ; sleep 1"
    )
}

fn port_open(port: u16) -> String {
    format!("(exec 3<>/dev/tcp/127.0.0.1/{port})")
}

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

pub fn as_chrome(program: &str) -> String {
    let trimmed = program.trim();
    let (binary, rest) = trimmed.split_once(char::is_whitespace).unwrap_or((trimmed, ""));
    if !BROWSER_NAMES.contains(&binary) {
        return program.to_string();
    }

    let mut command = vec![
        BROWSER.to_string(),
        "--no-sandbox".to_string(),
        "--no-first-run".to_string(),
        "--password-store=basic".to_string(),
        format!("--user-data-dir={CHROME_PROFILE}"),
    ];
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

fn install_browser_shims() -> String {
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
    let shadow = format!("{entry}NoDisplay=true\n");

    let others: Vec<&str> = BROWSER_NAMES.iter().copied().filter(|name| *name != BROWSER).collect();

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

fn envd_base(sandbox: &str) -> String {
    format!("https://{ENVD_PORT}-{sandbox}.e2b.app")
}

fn envelope(payload: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(payload.len() + 5);
    out.push(0);
    out.extend_from_slice(&(payload.len() as u32).to_be_bytes());
    out.extend_from_slice(payload);
    out
}

fn collect_private(body: &[u8], env: &crate::secrets::ProcessEnv) -> Result<Output, E2bError> {
    let mut output = collect(body).map_err(|error| match error {
        E2bError::Api { status, message } => {
            E2bError::Api { status, message: env.redact(&message) }
        }
        other => other,
    })?;
    output.stdout = env.redact(&output.stdout);
    output.stderr = env.redact(&output.stderr);
    Ok(output)
}

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
            Err(_) => continue,
        };

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
            exit_code = end["exitCode"].as_i64().unwrap_or(0) as i32;
        }
    }

    Ok(Output { stdout, stderr, exit_code })
}

fn decode(raw: &str) -> String {
    String::from_utf8_lossy(&decode_bytes(raw)).into_owned()
}

pub fn decode_bytes(raw: &str) -> Vec<u8> {
    const TABLE: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut bits = 0u32;
    let mut have = 0u8;
    let mut out: Vec<u8> = Vec::new();
    for byte in raw.bytes() {
        let Some(index) = TABLE.iter().position(|c| *c == byte) else {
            continue;
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

    async fn state_server(response: Option<String>) -> E2bClient {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};

        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        tokio::spawn(async move {
            let (mut stream, _) = listener.accept().await.unwrap();
            let mut request = [0; 1024];
            let _ = stream.read(&mut request).await;
            if let Some(response) = response {
                stream.write_all(response.as_bytes()).await.unwrap();
            } else {
                std::future::pending::<()>().await;
            }
        });
        let mut client = E2bClient::new("test").unwrap();
        client.base = format!("http://{address}");
        client.http =
            reqwest::Client::builder().timeout(Duration::from_millis(100)).build().unwrap();
        client
    }

    #[tokio::test]
    async fn state_server_error_is_not_gone() {
        let client = state_server(Some(
            "HTTP/1.1 500 Internal Server Error\r\nContent-Length: 0\r\n\r\n".into(),
        ))
        .await;
        assert!(matches!(client.state("test").await, Err(E2bError::Api { status: 500, .. })));
    }

    #[tokio::test]
    async fn state_malformed_reply_is_not_gone() {
        for body in ["{}", "{\"state\":\"unexpected\"}", "not json"] {
            let response =
                format!("HTTP/1.1 200 OK\r\nContent-Length: {}\r\n\r\n{body}", body.len());
            let client = state_server(Some(response)).await;
            assert!(matches!(client.state("test").await, Err(E2bError::Protocol(_))), "{body}");
        }
    }

    #[test]
    fn a_provider_reply_error_keeps_its_provider_prefix() {
        assert_eq!(E2bError::Protocol("unrecognized sandbox state".into()).to_string(), "the sandbox replied in a form this build does not understand: unrecognized sandbox state");
    }

    #[tokio::test]
    async fn state_not_found_is_gone() {
        let client =
            state_server(Some("HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\n\r\n".into())).await;
        assert_eq!(client.state("test").await.unwrap(), SandboxState::Gone);
    }

    #[tokio::test(start_paused = true)]
    async fn state_timeout_is_not_gone() {
        let client = state_server(None).await;
        assert!(matches!(client.state("test").await, Err(E2bError::Transport(_))));
    }

    #[tokio::test]
    async fn state_known_values_are_preserved() {
        for (body, expected) in [
            ("{\"state\":\"running\"}", SandboxState::Running),
            ("{\"state\":\"paused\"}", SandboxState::Paused),
        ] {
            let response =
                format!("HTTP/1.1 200 OK\r\nContent-Length: {}\r\n\r\n{body}", body.len());
            let client = state_server(Some(response)).await;
            assert_eq!(client.state("test").await.unwrap(), expected);
        }
    }

    #[test]
    fn a_computer_scrubs_output_and_error_trailers() {
        let env = crate::secrets::ProcessEnv::new(std::collections::BTreeMap::from([(
            "TOKEN".into(),
            "private-value".into(),
        )]));
        let out =
            stream(&[(0, serde_json::json!({"event":{"data":{"stdout":"cHJpdmF0ZS12YWx1ZQ=="}}}))]);
        assert_eq!(collect_private(&out, &env).unwrap().stdout, "[REDACTED]");
        let failed =
            stream(&[(2, serde_json::json!({"error":{"message":"rejected private-value"}}))]);
        let error = collect_private(&failed, &env).unwrap_err().to_string();
        assert!(!error.contains("private-value"));
        assert!(error.contains("[REDACTED]"));
    }

    #[test]
    fn the_window_is_allowed_to_frame_the_viewer() {
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
        for typed in [
            "google-chrome https://mail.google.com",
            "google-chrome-stable",
            "chromium https://example.com",
            "firefox https://mail.google.com",
            "firefox-esr",
            "x-www-browser https://example.com",
        ] {
            let launched = as_chrome(typed);
            assert!(launched.starts_with(BROWSER), "{typed} -> {launched}");
            assert!(launched.contains(CHROME_PROFILE), "{typed} -> {launched}");
            assert!(launched.contains("--no-sandbox"), "{launched}");
            assert!(
                launched.contains("--password-store=basic"),
                "without this Chrome blocks on a keyring prompt: {launched}"
            );
            for other in ["firefox", "chromium", "www-browser"] {
                assert!(!launched.contains(other), "{typed} -> {launched}");
            }
        }

        assert!(as_chrome("firefox https://mail.google.com").ends_with(" https://mail.google.com"));
    }

    #[test]
    fn nothing_on_a_machine_opens_a_remote_interface_into_the_browser() {
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
        let shim = install_browser_shims();
        assert!(shim.contains(&format!("{LOCAL_BIN}/{BROWSER}")), "{shim}");
        assert!(shim.contains(&format!("{LOCAL_APPS}/{BROWSER}.desktop")), "the menu reads this");
        assert!(shim.contains("/home/user/Desktop/*.desktop"), "so does the icon on the screen");
        assert!(shim.contains("/usr/share/applications"), "and the packaged entries: {shim}");
        for name in BROWSER_NAMES.iter().filter(|name| **name != BROWSER) {
            assert!(shim.contains(name), "{name} is a way to open a browser here: {shim}");
        }
        let shadowing = shim.find("basename").unwrap_or_default();
        let ours = shim.find(&format!("> {LOCAL_APPS}/{BROWSER}.desktop")).unwrap_or_default();
        assert!(shadowing < ours, "{shim}");
        assert!(shim.contains("~/.bash_profile"), "{shim}");

        let written: String = shim
            .split_whitespace()
            .filter(|token| token.len() > 40)
            .map(decode)
            .collect::<Vec<_>>()
            .join("\n");
        assert!(written.contains("exec"), "the wrapper should have decoded: {written}");
        assert!(written.contains("--password-store=basic"), "{written}");
        assert!(written.contains(CHROME_PROFILE), "{written}");
        assert!(written.contains("NoDisplay=true"), "{written}");
        assert!(written.contains(&format!("Exec={LOCAL_BIN}/{BROWSER}")), "{written}");
    }

    #[test]
    fn the_browser_button_on_the_dock_is_shimmed_although_it_names_no_browser() {
        let helper = web_browser_helper();
        assert!(helper.contains("X-XFCE-Category=WebBrowser"), "{helper}");
        for key in ["X-XFCE-Commands", "X-XFCE-CommandsWithParameter"] {
            assert!(helper.contains(&format!("{key}={LOCAL_BIN}/{BROWSER}")), "{key}: {helper}");
        }
        assert!(!helper.contains("%B"), "no binary to look up on PATH: {helper}");

        let shim = install_browser_shims();
        assert!(shim.contains(&base64_encode(helper.as_bytes())), "and it is put on the machine");
        assert!(shim.contains(&format!("{LOCAL_HELPERS}/custom-WebBrowser.desktop")), "{shim}");
        assert!(shim.contains("WebBrowser=custom-WebBrowser"), "{shim}");
        assert!(shim.contains(&format!("{XFCE_CONFIG}/helpers.rc")), "{shim}");
        assert!(
            shim.contains(&format!("grep -v '^WebBrowser=' {XFCE_CONFIG}/helpers.rc")),
            "every other key in that file survives being rewritten: {shim}"
        );
    }

    #[test]
    fn a_browser_on_the_wrong_profile_or_of_the_wrong_kind_is_ended() {
        let evict = evict_other_browsers();
        assert!(evict.contains(&format!("--user-data-dir={CHROME_PROFILE}")), "{evict}");
        assert!(evict.contains("--type="), "helper processes must be excluded: {evict}");
        assert!(evict.contains("pkill"), "{evict}");
        assert!(evict.contains("[f]irefox"), "{evict}");
        for stem in [CHROME_PROCESSES.as_slice(), OTHER_PROCESSES.as_slice()].concat() {
            assert!(!evict.contains(stem), "{stem} would match the shell running this: {evict}");
        }
    }

    #[test]
    fn a_caller_cannot_ask_for_a_second_profile() {
        let once = as_chrome("google-chrome --no-sandbox --user-data-dir=/tmp/x");
        assert_eq!(once.matches("--no-sandbox").count(), 1, "{once}");
        assert_eq!(once.matches("--user-data-dir").count(), 1, "{once}");
        assert!(once.contains(CHROME_PROFILE), "{once}");
        assert!(!once.contains("/tmp/x"), "{once}");

        let twice = as_chrome(&once);
        assert_eq!(twice, once);
    }

    #[test]
    fn a_program_that_is_not_a_browser_is_left_alone() {
        assert_eq!(as_chrome("xdg-open /home/user/report.pdf"), "xdg-open /home/user/report.pdf");
        assert_eq!(as_chrome("thunar"), "thunar");
        assert_eq!(as_chrome("firefox-history-reader x"), "firefox-history-reader x");
    }

    #[test]
    fn the_session_reader_is_pointed_at_the_profile_the_browser_actually_uses() {
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
        for sample in ["", "a", "ab", "abc", "abcd", "hello world", "{\"x\": 1}\n"] {
            assert_eq!(decode(&base64_encode(sample.as_bytes())), sample);
        }
    }

    #[test]
    fn model_supplied_text_cannot_escape_the_shell() {
        let command = DesktopAction::Type { text: "it's fine; rm -rf /".into() }.command();
        assert!(command.starts_with("xdotool type --clearmodifiers --delay 12 -- "), "{command}");
        assert!(command.contains("'it'\\''s fine; rm -rf /'"), "{command}");
        assert_eq!(quote("plain"), "'plain'");
    }

    #[test]
    fn long_text_is_typed_in_pieces_so_a_page_can_keep_up() {
        let text = "x".repeat(TYPE_CHUNK * 2 + 5);
        let command = DesktopAction::Type { text }.command();
        assert_eq!(command.matches("xdotool type").count(), 3, "{command}");
        assert_eq!(command.matches(" && ").count(), 2, "{command}");

        let typed: String = command
            .split(" && ")
            .map(|piece| piece.trim_start_matches("xdotool type --clearmodifiers --delay 12 -- "))
            .map(|piece| piece.trim_matches('\''))
            .collect();
        assert_eq!(typed.len(), TYPE_CHUNK * 2 + 5);
    }

    #[test]
    fn text_is_split_on_characters_rather_than_bytes() {
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
        assert!(down.contains("mousemove --sync 500 400"), "{down}");
        assert!(DesktopAction::Scroll { x: 1, y: 1, down: true, amount: 0 }
            .command()
            .contains("--repeat 1"));
    }

    #[test]
    fn a_drag_holds_the_button_down_across_the_move() {
        let command = DesktopAction::Drag { from: (10, 20), to: (90, 20) }.command();
        assert!(command.starts_with("xdotool mousemove --sync 10 20 mousedown 1"), "{command}");
        assert!(command.contains("mousemove --sync 90 20"), "{command}");
        assert!(command.ends_with("mouseup 1"), "{command}");
    }

    #[test]
    fn only_actions_whose_effect_arrives_later_are_waited_for() {
        for changes in [
            DesktopAction::Click { x: 1, y: 1, button: 1, count: 1 },
            DesktopAction::Type { text: "hello".into() },
            DesktopAction::Key { keys: "Return".into() },
            DesktopAction::Scroll { x: 1, y: 1, down: true, amount: 3 },
            DesktopAction::Drag { from: (0, 0), to: (9, 9) },
        ] {
            assert!(settle_for(&changes).contains("sleep"), "{changes:?} finishes after it exits");
        }
        assert_eq!(settle_for(&DesktopAction::Move { x: 1, y: 1 }), "");
        assert_eq!(settle_for(&DesktopAction::Wait { ms: 500 }), "");
    }

    #[test]
    fn a_screen_is_read_out_of_one_command_that_did_three_things() {
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

        let failed = Output {
            stdout: "ACT:0\nxdotool: something".into(),
            stderr: "scrot: failed".into(),
            exit_code: 1,
        };
        assert_eq!(read_screen(&failed), None);
    }

    #[test]
    fn waiting_is_bounded_so_a_model_cannot_hold_a_turn_open() {
        assert_eq!(DesktopAction::Wait { ms: 800 }.command(), "sleep 0.8");
        assert_eq!(DesktopAction::Wait { ms: 600_000 }.command(), "sleep 10");
    }

    #[test]
    fn a_daemon_is_detached_from_the_shell_that_starts_it() {
        let command = daemon("pgrep -x Xvfb", "Xvfb", "Xvfb :0");
        assert!(command.contains("setsid"), "a process that dies with its shell never serves");
        assert!(command.contains("</dev/null"), "holding stdin hangs the call that started it");
        assert!(command.contains(">/tmp/"), "holding stdout hangs it too");
    }

    #[test]
    fn no_guard_can_match_the_shell_that_is_running_it() {
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
        let body = create_body("Manager", 900);
        assert_eq!(body["autoPause"], true, "without this the timeout kills it");
        assert_eq!(body["timeout"], 900, "the idle period, pushed back on every use");
    }

    #[test]
    fn a_new_sandbox_is_created_with_both_locks_and_a_network() {
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
        let env = crate::secrets::ProcessEnv::new(std::collections::BTreeMap::from([(
            "GITHUB_TOKEN".to_string(),
            "ghp_hunter2".to_string(),
        )]));

        let body = process_body("curl -s api.github.com", &env);
        assert_eq!(body["process"]["envs"]["GITHUB_TOKEN"], "ghp_hunter2");
        assert_eq!(body["process"]["args"][2], "curl -s api.github.com");

        let bare = process_body("echo hi", &Default::default());
        assert_eq!(bare["process"]["envs"], serde_json::json!({}));
    }

    #[test]
    fn a_credential_is_never_written_to_the_machines_disk() {
        let env = crate::secrets::ProcessEnv::new(std::collections::BTreeMap::from([(
            "TOKEN".to_string(),
            "secret".to_string(),
        )]));
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
