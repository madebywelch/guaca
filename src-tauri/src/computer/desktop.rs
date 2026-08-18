//! The desktop, the browser and the cookie jar.
//!
//! Every one of these is a command, so none of them belongs to a provider: a
//! machine that can run `xdotool` gets a pointer, and one that can run `python3`
//! gets a browser. They sit above the boundary and every provider has them.
//!
//! One thing is still asked downwards: whether the browser can keep its own
//! sandbox is a fact about the machine rather than about Chrome, so the
//! provider states it and every launch here reads the answer.

use std::time::Duration;

use super::provider::{Output, ProviderError};
use super::Machine;

/// Where noVNC serves the desktop, once it has been started.
pub(super) const VNC_PORT: u16 = 6080;
/// The VNC server noVNC bridges to. Never exposed publicly.
const RAW_VNC_PORT: u16 = 5900;

/// Chrome's remote interface, used to drive pages exactly rather than by
/// aiming a pointer at pixels.
const CDP_PORT: u16 = 9222;

/// The driver Guac runs inside the sandbox. Kept as a file so it can be read
/// and tested as Python rather than as a Rust string.
const BROWSER_DRIVER: &str = include_str!("browser.py");

/// Reads what the browser is signed in to, from its own files on disk.
/// Separate from the driver because it deliberately does not need a browser.
const SESSION_READER: &str = include_str!("sessions.py");

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
pub const RUN_TIMEOUT: Duration = Duration::from_secs(120);

/// How long the browser gets to open its remote interface, and how often it is
/// asked.
///
/// A budget rather than a count of tries, because what varies is the machine
/// and not the number of questions. This was ten back-to-back checks, which on
/// a local machine is about three seconds of asking: enough for a browser on a
/// warm machine, and not enough after a wake, where Xvfb, the session, the VNC
/// server and Chromium all start together on a VM that has just booted. Timed
/// by hand there, the port opens about a second after Chromium itself is up,
/// so what ran out was the budget rather than the browser — and what an agent
/// saw was a tool that said the browser had no remote interface.
const BROWSER_WAIT: Duration = Duration::from_secs(20);
const BROWSER_POLL: Duration = Duration::from_millis(500);

/// The budget, in questions. One is asked immediately and the rest are a poll
/// apart, so the whole wait is what `BROWSER_WAIT` says it is.
const BROWSER_ATTEMPTS: u32 = (BROWSER_WAIT.as_millis() / BROWSER_POLL.as_millis()) as u32;

impl Machine {
    /// Whether Chrome on this machine may keep its own sandbox.
    ///
    /// Asked of the provider on every launch rather than remembered, because
    /// it is a fact about the machine underneath and the machine is the
    /// provider's. Nothing here caches it: the answer is a constant per
    /// provider and the call is a bool.
    fn sandboxed(&self) -> bool {
        self.provider.browser_keeps_its_sandbox()
    }

    /// Brings up the desktop: framebuffer, session, VNC server, noVNC bridge.
    ///
    /// Every step is idempotent by construction, because the pane asks for a
    /// desktop without tracking whether it has asked before. `pgrep` guards the
    /// ones that would otherwise stack up a second copy.
    pub async fn start_desktop(&self) -> Result<(), ProviderError> {
        for command in [
            // Before anything can be opened on the screen, so there is no
            // window through which the wrong profile can be reached.
            install_chrome_shim(self.sandboxed()),
            evict_wrong_profile_browser(),
            keep_browser_signin_off(),
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
            self.run_plain(&command).await?;
        }
        Ok(())
    }

    /// Starts a graphical program on the sandbox's screen.
    ///
    /// Brings the desktop up first, because an agent asked to open a browser
    /// should not have to know that a display exists, and both steps are
    /// idempotent. Detached the same way the desktop's own processes are, so
    /// the window outlives the call that opened it.
    pub async fn open_on_desktop(&self, program: &str) -> Result<Output, ProviderError> {
        self.start_desktop().await?;

        let program = chrome_flags(program, self.sandboxed());

        // The agent's own program, so it gets the agent's credentials: a
        // browser opened here is the one an operator signs in with.
        self.run(&format!(
            "(setsid env DISPLAY=:0 {program} >/tmp/guac-desktop-app.log 2>&1 </dev/null &) ; \
             sleep 2; echo started"
        ))
        .await
    }

    /// Makes sure the browser is running with its remote interface open, and
    /// that the driver script is on the machine.
    ///
    /// Chrome ignores `--remote-debugging-port` when it re-attaches to an
    /// existing profile, so the browser Guac drives gets a profile of its own.
    /// Everything here is idempotent; an agent should be able to browse without
    /// knowing any of it happened.
    async fn ensure_browser(&self) -> Result<(), ProviderError> {
        self.start_desktop().await?;

        let script = base64_encode(BROWSER_DRIVER.as_bytes());
        // The one launch that spells its own flags out rather than going
        // through `chrome_flags`, because it names the profile and the port
        // itself. What it still has to ask is the same question: whether the
        // machine underneath lets the browser keep its sandbox.
        let no_sandbox = if self.sandboxed() { "" } else { "--no-sandbox " };
        self.run_plain(&format!(
            "mkdir -p ~/.guac && echo {script} | base64 -d > ~/.guac/browser.py; \
             python3 -c 'import websocket' 2>/dev/null || pip install -q websocket-client; \
             {guard} >/dev/null 2>&1 || (setsid env DISPLAY=:0 google-chrome {no_sandbox}\
             --no-first-run --user-data-dir={CHROME_PROFILE} \
             --remote-debugging-port={CDP_PORT} about:blank \
             >/tmp/guac-chrome.log 2>&1 </dev/null &) ; sleep 1",
            guard = port_open(CDP_PORT)
        ))
        .await?;

        // Chrome takes a moment to open the port, and a browse that arrives
        // first fails in a way that reads as the tool being broken.
        for attempt in 0..BROWSER_ATTEMPTS {
            if attempt > 0 {
                // Waited out on the host. A `sleep` inside the command would
                // spend the exec's own deadline and hold a connection open for
                // the whole of it.
                tokio::time::sleep(BROWSER_POLL).await;
            }
            let up = self
                .run_plain(&format!("{} 2>/dev/null && echo up || echo down", port_open(CDP_PORT)))
                .await?;
            if up.stdout.trim() == "up" {
                return Ok(());
            }
        }
        Err(ProviderError::Operation("the browser did not open its remote interface".into()))
    }

    /// One browser action, answered as the driver's JSON.
    pub async fn browse(
        &self,
        action: &str,
        args: &serde_json::Value,
    ) -> Result<String, ProviderError> {
        self.ensure_browser().await?;

        // With credentials: the driver is acting for the agent, and a site it
        // is sent to may need one.
        let out = self
            .run(&format!(
                "python3 ~/.guac/browser.py {action} {}",
                quote(&serde_json::to_string(args).unwrap_or_else(|_| "{}".into()))
            ))
            .await?;

        if out.exit_code != 0 || out.stdout.trim().is_empty() {
            // The driver reports what went wrong on stderr in words meant for
            // the model, so it is passed through rather than summarised.
            let why = if out.stderr.trim().is_empty() {
                out.stdout.trim().to_string()
            } else {
                out.stderr.trim().to_string()
            };
            return Err(ProviderError::Operation(why.chars().take(300).collect()));
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
    pub async fn screenshot(&self) -> Result<(String, String), ProviderError> {
        self.start_desktop().await?;

        let out = self
            .run_plain(
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
            return Err(ProviderError::Operation(format!(
                "the screen could not be captured ({})",
                out.stderr.trim()
            )));
        }

        Ok((format!("data:image/jpeg;base64,{encoded}"), geometry))
    }

    /// Drives the mouse and keyboard, the same way E2B's own desktop SDK does.
    pub async fn act_on_desktop(&self, action: &DesktopAction) -> Result<Output, ProviderError> {
        self.start_desktop().await?;
        self.run_plain(&format!("DISPLAY=:0 {}", action.command())).await
    }

    /// Asks this machine what its browser is signed in to.
    ///
    /// Reads the profile `browse` drives, which is the one that matters: Chrome
    /// ignores `--remote-debugging-port` when it re-attaches to an existing
    /// profile, so Guaca's browser keeps a profile of its own and a session in
    /// any other window is one no agent can use.
    ///
    /// Deliberately not routed through `browse`. Connecting to the browser
    /// would start it if it were closed, so merely asking the question would
    /// boot Chrome on every machine; and `ensure_browser` costs several seconds
    /// it does not need to spend here. Cookies are on disk, so this is one
    /// command.
    pub async fn signed_in_state(
        &self,
    ) -> Result<crate::domain::signin::BrowserState, ProviderError> {
        let script = base64_encode(SESSION_READER.as_bytes());
        let out = self
            .run_plain(&format!(
                "mkdir -p {GUAC_DIR} && echo {script} | base64 -d > {GUAC_DIR}/sessions.py && \
                 python3 {GUAC_DIR}/sessions.py {CHROME_PROFILE}/Default"
            ))
            .await?;

        serde_json::from_str(out.stdout.trim()).map_err(|e| {
            ProviderError::Operation(format!(
                "could not read what the browser is signed in to ({e}): {}",
                out.stderr.trim().chars().take(200).collect::<String>()
            ))
        })
    }
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

/// The decoder that pairs with it, kept as bytes.
///
/// Public because a file pulled off a machine comes back this way, and a
/// document is not text: running it through a lossy string conversion would
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
pub(super) fn port_open(port: u16) -> String {
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
///
/// The crash handler is excluded by name. `chrome_crashpad_handler` lives in
/// the browser's directory, carries neither `--type=` nor `--user-data-dir=`,
/// and matched the pattern on the Debian build: every desktop start then read
/// it as a stray browser and killed the real one under the operator, which is
/// how a Google sign-in went stale — Chrome flushes rotated session cookies
/// lazily, and a browser killed before the flush comes back with the old
/// ones and is told it is signed out.
fn evict_wrong_profile_browser() -> String {
    format!(
        "if pgrep -af 'google-chrome|chromium' | grep -v -- '--type=' | \
         grep -v crashpad | grep -v -- '--user-data-dir={CHROME_PROFILE}' | grep -q .; then \
         pkill -f 'google-chrome|chromium' || true; sleep 1; fi"
    )
}

/// Keeps the browser from signing *itself* in when the operator signs in to a
/// Google site.
///
/// Chromium's account consistency (Dice) turns a Gmail sign-in into a browser
/// sign-in as well, by fetching a token with the browser's own API keys. The
/// image runs unbranded Chromium, which has none, so that step fails, and on
/// the next start Chromium's reconciler resolves "web signed in, browser not"
/// by deleting every `.google.com` account cookie: the operator was signed out
/// of Gmail after any close, wake or restart, over a jar that still held the
/// account. Measured on a live machine, and gone with `signin.allowed=false`.
/// The image ships the preference in a fresh profile; this puts it on a
/// profile made before that, and only while no browser is running, because
/// Chrome rewrites the file on exit and would undo an edit made underneath it.
/// Google Chrome (E2B) has keys and never hit this, and browser sign-in is not
/// something an agent's browser should do there either.
fn keep_browser_signin_off() -> String {
    let script = base64_encode(SIGNIN_OFF.as_bytes());
    format!(
        "if ! pgrep -x chromium >/dev/null && ! pgrep -x chrome >/dev/null; then \
         mkdir -p {CHROME_PROFILE}/Default && \
         echo {script} | base64 -d | python3 - {CHROME_PROFILE}/Default/Preferences; fi"
    )
}

/// The edit itself, as Python because the file is JSON and a shell has no
/// safe way to say "set this key and leave everything else". Idempotent: a
/// profile that already says no is left byte-identical.
const SIGNIN_OFF: &str = "import json, sys, os\n\
path = sys.argv[1]\n\
try:\n\
    prefs = json.load(open(path))\n\
except (OSError, ValueError):\n\
    prefs = {}\n\
signin = prefs.setdefault('signin', {})\n\
if signin.get('allowed') is False and signin.get('allowed_on_next_startup') is False:\n\
    sys.exit(0)\n\
signin['allowed'] = False\n\
signin['allowed_on_next_startup'] = False\n\
prefs.get('google', {}).pop('services', None)\n\
tmp = path + '.guaca'\n\
json.dump(prefs, open(tmp, 'w'))\n\
os.replace(tmp, path)\n";

/// Rewrites a browser invocation so it lands in the one profile that counts.
///
/// The shim on PATH does this too, and this does it again at the call site,
/// because the two fail differently: the shim covers a name typed into a shell
/// and the icon on the desktop, and this covers the machine whose `~/.profile`
/// does not put `~/.local/bin` first. A duplicated flag with the same value is
/// nothing; a window on the wrong profile is a session no agent can use.
///
/// Whether Chrome may keep its own sandbox is a fact about the machine rather
/// than about Chrome, so it arrives as an argument: a hosted sandbox refuses to
/// start a browser that still has one, and a VM per agent has no reason to take
/// it away. `ComputerProvider::browser_keeps_its_sandbox` is what answers.
fn chrome_flags(program: &str, sandboxed: bool) -> String {
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
    let mut flags = vec!["--no-first-run", "--password-store=basic"];
    if !sandboxed {
        flags.insert(0, "--no-sandbox");
    }
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
///
/// It carries the same flags as the call site and for the same reasons,
/// `--no-sandbox` included: a wrapper that dropped it on a machine whose Chrome
/// needs it would be a browser that starts from one route and not the other.
fn install_chrome_shim(sandboxed: bool) -> String {
    let no_sandbox = if sandboxed { "" } else { "--no-sandbox " };
    // Resolved past the shim itself: `/usr/bin/google-chrome` is a symlink to
    // the first of these, and calling by name would find the wrapper again.
    let wrapper = format!(
        "#!/bin/sh\n\
         # Guaca: one profile on this machine, the one agents can use.\n\
         for real in /opt/google/chrome/google-chrome /usr/bin/google-chrome-stable \
         /usr/bin/chromium /usr/bin/chromium-browser; do\n\
         \x20 [ -x \"$real\" ] && exec \"$real\" {no_sandbox}--no-first-run \
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::computer::fake::FakeProvider;
    use crate::computer::provider::ProviderHandle;
    use crate::computer::VIEWER_HOST;
    use crate::domain::computer::Secret;
    use crate::domain::ids::ComputerId;
    use std::collections::BTreeMap;
    use std::sync::Arc;

    fn machine(provider: Arc<FakeProvider>) -> Machine {
        let handle = ProviderHandle {
            computer: ComputerId::new(),
            provider_id: "m".into(),
            control_secret: Secret::default(),
            viewer_secret: Secret::default(),
        };
        Machine::new(provider, handle, BTreeMap::new(), 0)
    }

    fn said(what: &str) -> Output {
        Output { stdout: what.into(), stderr: String::new(), exit_code: 0 }
    }

    /// How many times the port was asked whether the browser is listening.
    fn port_checks(provider: &FakeProvider) -> usize {
        provider
            .execs
            .lock()
            .iter()
            .filter(|request| request.argv.join(" ").contains("echo up"))
            .count()
    }

    #[tokio::test(start_paused = true)]
    async fn the_browser_is_given_time_rather_than_a_number_of_questions() {
        // What this closes: after a wake, Xvfb, the session, the VNC server and
        // Chromium all start together on a VM that has just booted, and ten
        // back-to-back checks are about three seconds of asking. The browser
        // was fine; the budget was not, and what the agent read was that its
        // browser had no remote interface.
        let provider = Arc::new(FakeProvider::default());
        *provider.matched.lock() =
            vec![("echo up".to_string(), vec![said("down"), said("down"), said("up")])];

        let started = tokio::time::Instant::now();
        machine(provider.clone())
            .ensure_browser()
            .await
            .expect("the port opened while there was still budget");

        assert_eq!(port_checks(&provider), 3, "it stopped asking the moment the port answered");
        assert!(
            started.elapsed() >= BROWSER_POLL * 2,
            "the questions were a poll apart rather than back to back"
        );
    }

    #[tokio::test(start_paused = true)]
    async fn a_browser_that_never_opens_its_port_is_reported_once_the_budget_is_spent() {
        let provider = Arc::new(FakeProvider::default());
        *provider.matched.lock() = vec![("echo up".to_string(), vec![said("down")])];

        let err =
            machine(provider.clone()).ensure_browser().await.expect_err("the port never opened");

        assert!(err.to_string().contains("did not open its remote interface"), "{err}");
        assert_eq!(
            port_checks(&provider),
            BROWSER_ATTEMPTS as usize,
            "the whole budget was spent before giving up"
        );
    }

    /// Connect's JSON mapping sends `bytes` as base64, and the encoder above is
    /// the one that has to survive it, so the pair is asserted rather than
    /// either half.
    fn decode(raw: &str) -> String {
        String::from_utf8_lossy(&decode_bytes(raw)).into_owned()
    }

    /// What a shim command actually puts on the machine, rather than what the
    /// command looks like: the wrapper and the desktop entry travel base64'd.
    fn written_by(shim: &str) -> String {
        shim.split_whitespace()
            .filter(|token| token.len() > 40)
            .map(decode)
            .collect::<Vec<_>>()
            .join("\n")
    }

    #[test]
    fn the_window_is_allowed_to_frame_the_viewer() {
        // The viewer moved from E2B's own host to a loopback proxy and the CSP
        // was left behind, so the webview blocked the iframe outright. Every
        // check at the HTTP layer passed, because curl does not enforce CSP,
        // and the screen stayed black.
        let conf: serde_json::Value =
            serde_json::from_str(include_str!("../../tauri.conf.json")).expect("tauri.conf.json");
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
        //
        // Asserted on both kinds of machine, because the profile and the port
        // are the machine's business either way and only the sandbox flag is
        // allowed to move with it.
        for sandboxed in [false, true] {
            for typed in [
                "google-chrome https://mail.google.com",
                "google-chrome-stable",
                "chromium https://example.com",
            ] {
                let launched = chrome_flags(typed, sandboxed);
                assert!(launched.contains(CHROME_PROFILE), "{typed} -> {launched}");
                assert!(
                    launched.contains(&format!("--remote-debugging-port={CDP_PORT}")),
                    "a window without the port re-attaches and leaves browse with no interface: \
                     {launched}"
                );
                assert_eq!(
                    launched.matches("--no-sandbox").count(),
                    usize::from(!sandboxed),
                    "the flag is on a machine whose Chrome needs it and nowhere else: {launched}"
                );
                // Observed: Chrome opened on the screen, asked to create a
                // system keyring password, and the agent driving it reported
                // that the profile was fresh and Gmail unavailable. The flag
                // also fixes how the cookie jar is encrypted, so a session
                // survives being reopened by a different route.
                assert!(
                    launched.contains("--password-store=basic"),
                    "without this Chrome blocks on a keyring prompt: {launched}"
                );
            }

            // The shim covers what the call site cannot see: a name typed into
            // a shell, and the icon on the desktop.
            let shim = install_chrome_shim(sandboxed);
            assert!(shim.contains("/home/user/.local/bin/google-chrome"), "{shim}");
            assert!(shim.contains("applications/google-chrome.desktop"), "the icon reads this one");

            let written = written_by(&shim);
            assert!(written.contains("exec"), "the wrapper should have decoded: {written}");
            // The operator's own click has to land on the same store as an
            // agent's launch, or one of them writes a cookie jar the other
            // cannot read.
            assert!(written.contains("--password-store=basic"), "{written}");
            assert!(written.contains(CHROME_PROFILE), "{written}");
            assert_eq!(
                written.matches("--no-sandbox").count(),
                usize::from(!sandboxed),
                "the wrapper and the call site have to agree about this too: {written}"
            );
        }
    }

    #[test]
    fn a_machine_whose_browser_keeps_its_sandbox_is_launched_identically_without_the_flag() {
        // What this closes: `--no-sandbox` was written as a fact about Chrome,
        // so it went onto an Apple Container guest too, where the browser's own
        // sandbox works — and the operator watched every desktop from behind
        // Chrome's yellow "Stability and security will suffer" bar.
        //
        // Compared rather than spelled out twice on purpose: that one flag is
        // the whole difference, and a profile or a port lost along with it
        // would be a browser broken on one kind of machine and nowhere else.
        let hosted = chrome_flags("google-chrome https://example.com", false);
        let local = chrome_flags("google-chrome https://example.com", true);
        assert!(hosted.contains("--no-sandbox "), "{hosted}");
        assert_eq!(hosted.replace("--no-sandbox ", ""), local);

        let hosted_shim = written_by(&install_chrome_shim(false));
        let local_shim = written_by(&install_chrome_shim(true));
        assert!(hosted_shim.contains("--no-sandbox "), "{hosted_shim}");
        assert_eq!(hosted_shim.replace("--no-sandbox ", ""), local_shim);
    }

    #[tokio::test(start_paused = true)]
    async fn the_provider_says_whether_the_browser_keeps_its_sandbox_and_every_launch_follows() {
        // The machine is the provider's, so this is the provider's to answer.
        // All three routes onto a screen are read back, because each is written
        // somewhere else: the wrapper every desktop start installs, the window
        // an agent opens, and the browser `browse` drives.
        for sandboxed in [false, true] {
            let provider = Arc::new(FakeProvider::keeping_browser_sandbox(sandboxed));
            *provider.matched.lock() = vec![("echo up".to_string(), vec![said("up")])];

            let computer = machine(provider.clone());
            computer
                .open_on_desktop("google-chrome https://example.com")
                .await
                .expect("the desktop came up and a window was opened on it");
            computer.ensure_browser().await.expect("the browser answered on its port");

            let commands: Vec<String> =
                provider.execs.lock().iter().map(|request| request.argv.join(" ")).collect();
            let found = |needle: &str| {
                commands
                    .iter()
                    .find(|command| command.contains(needle))
                    .unwrap_or_else(|| panic!("no command containing {needle:?} in {commands:#?}"))
                    .clone()
            };
            let shim = found("chmod +x /home/user/.local/bin/google-chrome");
            // Named by what each launch is for rather than by the binary: both
            // of them run `google-chrome` on the display.
            let opened = found("https://example.com");
            let driven = found("about:blank");

            for command in [&written_by(&shim), &opened, &driven] {
                assert_eq!(
                    command.contains("--no-sandbox"),
                    !sandboxed,
                    "sandboxed={sandboxed}: {command}"
                );
                // The rest of the launch does not move with the flag.
                assert!(command.contains(CHROME_PROFILE), "{command}");
            }
        }
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
        // Seen live: the crash handler has no `--type=` and no profile flag,
        // so without this every desktop start killed the operator's own
        // browser and their Google session with it.
        assert!(evict.contains("grep -v crashpad"), "the crash handler is not a browser: {evict}");
    }

    #[test]
    fn browser_sign_in_is_switched_off_only_when_no_browser_is_running() {
        // Chrome rewrites Preferences on exit; an edit made under a running
        // browser is undone by it, so the step is guarded on the process list.
        let step = keep_browser_signin_off();
        assert!(step.contains("pgrep -x chromium"), "{step}");
        assert!(step.contains("pgrep -x chrome "), "{step}");
        assert!(step.contains(&format!("{CHROME_PROFILE}/Default/Preferences")), "{step}");
        // What lands on the machine, not what the command looks like.
        let script = step
            .split("echo ")
            .nth(1)
            .and_then(|rest| rest.split(' ').next())
            .map(|b64| String::from_utf8(decode_bytes(b64)).unwrap())
            .expect("the edit travels base64");
        assert!(script.contains("signin['allowed'] = False"), "{script}");
        assert!(script.contains("os.replace"), "written whole, then swapped in: {script}");
    }

    #[test]
    fn a_flag_the_caller_already_set_is_not_set_twice() {
        let once = chrome_flags("google-chrome --no-sandbox --user-data-dir=/tmp/x", false);
        assert_eq!(once.matches("--no-sandbox").count(), 1, "{once}");
        assert_eq!(once.matches("--user-data-dir").count(), 1, "{once}");
        // And a caller that named its own profile keeps it: only `browse` does
        // that, and it names this one.
        assert!(once.contains("--user-data-dir=/tmp/x"), "{once}");
    }

    #[test]
    fn a_program_that_is_not_a_browser_is_left_alone() {
        for sandboxed in [false, true] {
            assert_eq!(
                chrome_flags("xdg-open /home/user/report.pdf", sandboxed),
                "xdg-open /home/user/report.pdf"
            );
            assert_eq!(chrome_flags("thunar", sandboxed), "thunar");
        }
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
    fn base64_round_trips_the_output_of_a_command() {
        // envd sends stdout as base64, so getting this wrong turns every
        // command's output into noise.
        assert_eq!(decode("aGVsbG8gd29ybGQ="), "hello world");
        assert_eq!(decode("eA=="), "x");
        assert_eq!(decode(""), "");
    }
}
