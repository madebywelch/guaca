//! Guaca's end of a running coding job.
//!
//! A job is a process that runs for up to [`super::CEILING`] in a directory the
//! operator linked, and until this module existed it was write-only. Guaca read
//! its stdout and could say nothing back. An operator watching a job go the
//! wrong way at minute three had one move, which was to wait thirty-seven more
//! minutes for it to finish and then start another one.
//!
//! Claude Code has a second interface besides its stdout, and this is it. Hooks
//! run at fixed points in the job's own loop, are handed the event on stdin, and
//! what they print back is *acted on*: a `PostToolUse` hook can put words in
//! front of the model, a `Stop` hook can refuse to let the run finish, and a
//! `PreToolUse` hook can deny a tool call that the run's permission mode would
//! otherwise have allowed. That last one is not a guess. Measured against
//! 2.1.247, a `permissionDecision` of `deny` overrides `--permission-mode
//! bypassPermissions`, which is the mode every job here runs in.
//!
//! So the job gets three things it did not have:
//!
//! - **A mailbox.** The operator can redirect a job that is already running,
//!   and the correction arrives at the next tool boundary rather than at the
//!   end. [`Bridge::post`] stages it; the `PostToolUse` and `Stop` hooks deliver
//!   it.
//! - **A gate.** A push, a merge or a release is the operator's own name going
//!   somewhere it cannot be taken back from, and those can now stop and ask.
//!   [`Gate`] is what decides whether they do.
//! - **Two ways to report.** An MCP server with `note_progress` and
//!   `report_pull_request` on it, so what a job produced arrives as a value
//!   rather than as a sentence in its closing paragraph that the agent which
//!   asked has to parse.
//!
//! ## One server, two shapes of caller
//!
//! Everything above is one loopback HTTP server, bound on a port the OS picks,
//! exactly as [`crate::artifact`] and [`crate::proxy`] are. Two routes:
//! `POST /{token}/hook` carries a hook's stdin and answers with its stdout, and
//! `POST /{token}/mcp` is JSON-RPC.
//!
//! The hook scripts are three lines of `sh` around `curl` for that reason. They
//! parse nothing, decide nothing and hold no state, so there is no second
//! implementation of any rule here in a language nothing type-checks. What they
//! do have is one behavior worth stating: **they fail open**. A bridge that did
//! not start, a `curl` that is not installed, a server that has already dropped
//! the job all end the same way, with an empty answer and an exit status of
//! zero, which is a job that runs exactly as it did before this file existed.
//!
//! ## The token is the session id, and it is the whole of the authorization
//!
//! Loopback with an unguessable path, which is the posture [`crate::proxy`]
//! already takes. Anything else on this machine that can reach `127.0.0.1` can
//! reach this server, and what it would need to do anything with it is a v4
//! UUID that is never written down outside the job's own scratch directory and
//! that stops working the moment the job ends.
//!
//! It is the session id because Guaca now *chooses* that rather than reading it
//! back: `--session-id` takes a UUID, so one value is the job's name on the
//! bridge, the key of its mailbox, and what an operator hands to `claude
//! --resume` to open the same work in their own terminal.
//!
//! ## What this does not do
//!
//! It is not a sandbox and [`outward`] is not a security boundary. The gate
//! reads a shell command and decides whether it looks like it reaches outside
//! the repository, which is a judgment about the ordinary case: a model that
//! wanted to get around it could, and the process was already running as the
//! operator with their credentials and their network before any of this. What
//! the gate buys is that the ordinary push, made by a job doing what it was
//! asked, is one an operator gets to see first. `docs/CODING.md` says the same
//! thing at more length, and neither should ever be softened into a claim about
//! confinement.

use std::collections::{HashMap, HashSet};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use parking_lot::Mutex;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::{mpsc, oneshot};

use crate::domain::repository::Gate;

/// How long a hook may wait for Guaca to answer.
///
/// Only the gate ever waits, and what it waits on is `Runtime::park`, whose own
/// window is ten minutes. Comfortably longer than that, so the timeout that
/// fires is the one with a decision behind it: a `curl` that gave up first
/// would fail open, which is a permission gate that grants when the operator is
/// slow, and that is worse than not having one.
const HOOK_TIMEOUT_SECS: u64 = 900;

/// The most staged mail one job will hold.
///
/// An operator typing corrections faster than the job reaches a tool boundary
/// is an operator changing their mind, and the newest of those is the one they
/// mean. Past this the oldest is dropped, because a job handed twenty
/// instructions at once follows none of them.
const MAX_STAGED: usize = 8;

/// The cap on one staged message.
///
/// It is injected in front of a model mid-run, so it is a correction rather
/// than a brief. A whole new brief is a whole new job.
const MAX_MESSAGE_LEN: usize = 2_000;

/// The largest request body this server will read.
///
/// A `PostToolUse` payload carries the tool's own result, which for a `Read` of
/// a large file is the file. Read and thrown away rather than refused: the
/// answer does not depend on the body past its event name.
const BODY_LIMIT: usize = 4 * 1024 * 1024;

const HEAD_LIMIT: usize = 16 * 1024;

/// What a running job says to the runtime that started it.
///
/// A channel rather than a callback, because the job's own task is already
/// sitting there awaiting the process and can drain this beside it. Nothing
/// here carries the agent or the repository: the task on the other end has
/// both, and putting them on the wire would be two copies of one fact.
#[derive(Debug)]
pub enum Signal {
    /// The job said something about its own progress, on purpose, through the
    /// tool rather than in its narration.
    Note(String),
    /// It opened a pull request and said so.
    PullRequest { url: String, branch: String },
    /// It is about to do something outward-facing and the gate stopped it.
    ///
    /// Both the line the model wrote and what [`outward`] made of it, because
    /// they are not the same sentence and the operator needs both: `npm run
    /// release` is what was typed and `git push` is what it does.
    ///
    /// The `reply` is what unblocks the hook, and it must always be sent:
    /// dropping it answers `false`, which is a deny, which is the safe way for
    /// a bug here to fail but not a way to leave it.
    Permission { line: String, reach: Reach, reply: oneshot::Sender<bool> },
}

/// What a job's end of the bridge adds to the harness command line.
///
/// A plain value rather than a method on [`Session`], so the argument vector
/// stays a pure function of it and the offline suite can build one without
/// binding a port. `coding/claude_code.rs` is the only reader.
#[derive(Debug, Clone, PartialEq)]
pub struct Wiring {
    /// The job's own name, chosen rather than read back. Also the bridge token
    /// and what `claude --resume` takes.
    pub session_id: String,
    /// The settings file holding this job's hooks. Passed with `--settings`,
    /// which is additive: the operator's own settings still load.
    pub settings: PathBuf,
    /// The MCP server config, inline. Passed without `--strict-mcp-config`, so
    /// the operator's own servers still load too.
    pub mcp_config: String,
}

/// One job, while it is running.
struct Job {
    signals: mpsc::Sender<Signal>,
    gate: Gate,
    /// The work tree this job runs in.
    ///
    /// Held only so the gate can read what a line in it actually runs. Nothing
    /// else here needs it: the task on the other end of `signals` has the
    /// repository already, which is why nothing else about it is on the wire.
    root: PathBuf,
    /// Staged operator messages that have not reached the model yet.
    mail: Mutex<Vec<String>>,
}

#[derive(Default)]
struct Registry {
    jobs: HashMap<String, Arc<Job>>,
}

/// The bridge itself: one server, and a registry of the jobs reachable through
/// it.
#[derive(Clone, Default)]
pub struct Bridge {
    inner: Arc<Inner>,
}

#[derive(Default)]
struct Inner {
    registry: Mutex<Registry>,
    /// Bound on the first job that wants it and never again. A workspace whose
    /// repositories all run `pi` never opens a socket at all.
    port: tokio::sync::OnceCell<u16>,
}

impl Bridge {
    pub fn new() -> Self {
        Self::default()
    }

    /// Opens a job's end of the bridge.
    ///
    /// `None` when anything about it failed, and the caller runs the job
    /// without it. That is the whole of the error handling and it is
    /// deliberate: everything here is an improvement on a job that worked
    /// without any of it, so a bridge that cannot start must degrade to the
    /// job that worked, not to no job at all.
    pub async fn open(
        &self,
        signals: mpsc::Sender<Signal>,
        gate: Gate,
        root: PathBuf,
    ) -> Option<Session> {
        let port = *self.inner.port.get_or_try_init(|| listen(self.clone())).await.ok()?;

        let token = uuid::Uuid::new_v4().to_string();
        let dir = scratch(&token)?;
        let settings = dir.join("settings.json");
        let script = dir.join("hook.sh");

        write_script(&script, port, &token)?;
        write_settings(&settings, &script, gate)?;

        self.inner.registry.lock().jobs.insert(
            token.clone(),
            Arc::new(Job { signals, gate, root, mail: Mutex::new(Vec::new()) }),
        );

        Some(Session {
            bridge: self.clone(),
            dir,
            wiring: Wiring { mcp_config: mcp_config(port, &token), session_id: token, settings },
        })
    }

    /// Stages a message for a running job.
    ///
    /// `false` when nothing on the bridge answers to that token, which is a job
    /// that ended between the operator pressing the button and this call. Said
    /// rather than swallowed: the alternative is a panel that accepts a
    /// correction into a process that is not there to read it.
    pub fn post(&self, session_id: &str, message: &str) -> bool {
        let message = message.trim();
        if message.is_empty() {
            return false;
        }
        let mut cut: String = message.chars().take(MAX_MESSAGE_LEN).collect();
        if cut.len() < message.len() {
            cut.push('…');
        }

        let registry = self.inner.registry.lock();
        let Some(job) = registry.jobs.get(session_id) else {
            return false;
        };
        let mut mail = job.mail.lock();
        mail.push(cut);
        // The newest is the one the operator means. Dropping from the front
        // keeps the order the rest of this file renders in.
        while mail.len() > MAX_STAGED {
            mail.remove(0);
        }
        true
    }

    /// Whether a job is holding mail nobody has read yet.
    ///
    /// Only the tests ask. The hooks read and clear in one step, because the
    /// answer to "is there mail" is worthless a line later.
    #[cfg(test)]
    fn pending(&self, session_id: &str) -> usize {
        let registry = self.inner.registry.lock();
        registry.jobs.get(session_id).map(|job| job.mail.lock().len()).unwrap_or(0)
    }

    fn job(&self, token: &str) -> Option<Arc<Job>> {
        self.inner.registry.lock().jobs.get(token).cloned()
    }

    fn close(&self, token: &str) {
        self.inner.registry.lock().jobs.remove(token);
    }
}

/// A job's end of the bridge, for as long as it is running.
///
/// Dropping it deregisters the job and takes its scratch directory with it, so
/// the lifetime of the mailbox is the lifetime of this value and there is no
/// second place to remember to clean up. A job that panicked, was killed at the
/// ceiling or was stopped by the operator all unwind through the same drop.
pub struct Session {
    bridge: Bridge,
    dir: PathBuf,
    wiring: Wiring,
}

impl Session {
    pub fn wiring(&self) -> &Wiring {
        &self.wiring
    }

    /// The job's own id, which is also what `claude --resume` takes.
    pub fn session_id(&self) -> &str {
        &self.wiring.session_id
    }
}

impl Drop for Session {
    fn drop(&mut self) {
        self.bridge.close(&self.wiring.session_id);
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

// ---- what the job is started with ----------------------------------------

/// A directory only this user can read, holding one job's hooks.
///
/// The hook script has the bridge token in it, which is the whole of the
/// authorization, so the mode is not decoration. `None` on any failure, which
/// [`Bridge::open`] turns into a job that runs without a bridge.
fn scratch(token: &str) -> Option<PathBuf> {
    let dir = std::env::temp_dir().join(format!("guaca-job-{token}"));
    std::fs::create_dir_all(&dir).ok()?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o700)).ok()?;
    }
    Some(dir)
}

/// The one hook script, which every hook runs.
///
/// It knows nothing about which event it is handling. The payload names its own
/// event, so the branch is on this side, in Rust, where the rules already are:
/// a script that decided anything would be a second implementation of them in a
/// language nothing type-checks.
///
/// `--fail-with-body` is deliberately absent. This must not distinguish a
/// server that refused from a server that is not there, because both mean the
/// same thing to a job: carry on. Everything is discarded to make sure of it,
/// and the `exit 0` is unconditional for the same reason.
fn write_script(at: &Path, port: u16, token: &str) -> Option<()> {
    let script = format!(
        "#!/bin/sh\n\
         # Guaca's end of this coding job. Reads the hook payload on stdin, hands\n\
         # it to the app, prints back whatever the app answered with.\n\
         #\n\
         # Fails open on purpose: no curl, no bridge, or no answer all leave the\n\
         # job running exactly as it would without any of this.\n\
         curl --silent --show-error --max-time {HOOK_TIMEOUT_SECS} \\\n\
         \x20 --header 'content-type: application/json' \\\n\
         \x20 --data-binary @- \\\n\
         \x20 'http://127.0.0.1:{port}/{token}/hook' 2>/dev/null\n\
         exit 0\n"
    );
    let mut file = std::fs::File::create(at).ok()?;
    file.write_all(script.as_bytes()).ok()?;
    drop(file);
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(at, std::fs::Permissions::from_mode(0o700)).ok()?;
    }
    Some(())
}

/// The settings file this job is started with, holding its hooks and nothing
/// else.
///
/// Three events, and each is here for something a job could not do without it.
///
/// - **`PostToolUse`** is the mailbox's ordinary delivery. It is the only point
///   in a job's loop that comes round often enough to be worth waiting for and
///   is safe to interrupt: the model has just finished one thing and has not
///   started the next.
/// - **`Stop`** is the mailbox's other end. Without it a correction typed at
///   minute forty-four lands after the job has decided it is done, which is a
///   message the operator watched being delivered to nobody.
/// - **`PreToolUse`** is the gate, and it is registered only when there is one.
///   A matcher of `Bash` rather than `*`: everything that reaches outside the
///   repository goes through a shell, and a hook on every `Read` would be a
///   round trip per tool call for a question whose answer is always yes.
fn write_settings(at: &Path, script: &Path, gate: Gate) -> Option<()> {
    let script = script.to_str()?;
    let run = serde_json::json!([{ "type": "command", "command": script }]);

    let mut hooks = serde_json::json!({
        "PostToolUse": [{ "matcher": "*", "hooks": run }],
        "Stop": [{ "hooks": run }],
    });
    if gate == Gate::AskBeforePushing {
        hooks["PreToolUse"] = serde_json::json!([{ "matcher": "Bash", "hooks": run }]);
    }

    let body = serde_json::json!({ "hooks": hooks }).to_string();
    std::fs::write(at, body).ok()
}

/// The job's own MCP server, named so its tools read as Guaca's.
///
/// Claude Code prefixes a server's tools with its name, so these arrive as
/// `mcp__guaca__note_progress`. Passed *without* `--strict-mcp-config`, which
/// is what keeps the operator's own servers loaded: this adds one, it does not
/// replace theirs.
fn mcp_config(port: u16, token: &str) -> String {
    serde_json::json!({
        "mcpServers": {
            "guaca": { "type": "http", "url": format!("http://127.0.0.1:{port}/{token}/mcp") }
        }
    })
    .to_string()
}

// ---- the server ----------------------------------------------------------

async fn listen(bridge: Bridge) -> std::io::Result<u16> {
    let listener = TcpListener::bind(("127.0.0.1", 0)).await?;
    let port = listener.local_addr()?.port();

    tokio::spawn(async move {
        loop {
            let Ok((client, _)) = listener.accept().await else {
                continue;
            };
            let bridge = bridge.clone();
            tokio::spawn(async move {
                if let Err(err) = serve(client, bridge).await {
                    tracing::debug!(%err, "coding bridge connection ended");
                }
            });
        }
    });

    tracing::info!(port, "coding bridge listening");
    Ok(port)
}

/// What a request asked for, once the path has been believed as little as
/// possible.
#[derive(Debug, PartialEq)]
struct Asked {
    token: String,
    route: Route,
}

#[derive(Debug, PartialEq)]
enum Route {
    Hook,
    Mcp,
}

/// The token and route a request line names, if it names a pair this serves.
///
/// The token is checked for shape here rather than trusted, because it goes on
/// to be a map key: `/{uuid}/{hook|mcp}` and nothing else. A UUID is 36
/// characters of hex and dashes, and anything that is not one cannot be a job
/// whatever the map says.
fn asked(head: &[u8]) -> Option<Asked> {
    let text = String::from_utf8_lossy(head);
    let mut start = text.split("\r\n").next()?.split(' ');
    if start.next()? != "POST" {
        return None;
    }
    let path = start.next()?.split('?').next()?;
    let mut parts = path.strip_prefix('/')?.split('/');
    let token = parts.next()?;
    let route = match parts.next()? {
        "hook" => Route::Hook,
        "mcp" => Route::Mcp,
        _ => return None,
    };
    if parts.next().is_some() {
        return None;
    }
    let shaped =
        token.len() == 36 && token.bytes().all(|byte| byte.is_ascii_hexdigit() || byte == b'-');
    shaped.then(|| Asked { token: token.to_string(), route })
}

async fn serve(mut client: TcpStream, bridge: Bridge) -> std::io::Result<()> {
    let (head, body) = read_request(&mut client).await?;

    let Some(Asked { token, route }) = asked(&head) else {
        return answer(&mut client, "404 Not Found", "").await;
    };
    // A job that has ended is not an error worth a status of its own. The hook
    // fails open on an empty body, and an MCP call arriving after the process
    // it belongs to is gone has nobody to tell.
    let Some(job) = bridge.job(&token) else {
        return answer(&mut client, "404 Not Found", "").await;
    };

    let payload = serde_json::from_slice::<serde_json::Value>(&body).unwrap_or_default();
    match route {
        Route::Hook => {
            let said = hook(&job, &payload).await;
            answer(&mut client, "200 OK", &said).await
        }
        Route::Mcp => match rpc(&job, &payload).await {
            // A notification has no id and takes no answer, which is what the
            // 202 says. Answering one with a body is a protocol error the
            // client is entitled to complain about.
            None => answer(&mut client, "202 Accepted", "").await,
            Some(said) => answer(&mut client, "200 OK", &said.to_string()).await,
        },
    }
}

/// Reads a request, head and body.
///
/// The body is bounded and the excess is read and dropped rather than refused,
/// because a `PostToolUse` payload carries whatever the tool returned and this
/// answer does not depend on it. Refusing would turn a job that read a large
/// file into a job whose next hook call failed.
async fn read_request(client: &mut TcpStream) -> std::io::Result<(Vec<u8>, Vec<u8>)> {
    let mut head = Vec::new();
    let mut byte = [0u8; 1];
    while !head.ends_with(b"\r\n\r\n") {
        if head.len() > HEAD_LIMIT {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                "request head too long",
            ));
        }
        if client.read(&mut byte).await? == 0 {
            return Ok((head, Vec::new()));
        }
        head.push(byte[0]);
    }

    let text = String::from_utf8_lossy(&head).to_ascii_lowercase();
    let length: usize = text
        .split("\r\n")
        .find_map(|line| line.strip_prefix("content-length:"))
        .and_then(|value| value.trim().parse().ok())
        .unwrap_or(0);

    let mut body = vec![0u8; length.min(BODY_LIMIT)];
    client.read_exact(&mut body).await?;
    if length > BODY_LIMIT {
        let mut rest = client.take((length - BODY_LIMIT) as u64);
        let mut sink = Vec::new();
        let _ = rest.read_to_end(&mut sink).await;
    }
    Ok((head, body))
}

async fn answer(client: &mut TcpStream, status: &str, body: &str) -> std::io::Result<()> {
    let head = format!(
        "HTTP/1.1 {status}\r\n\
         content-type: application/json\r\n\
         cache-control: no-store\r\n\
         content-length: {}\r\n\
         connection: close\r\n\r\n",
        body.len()
    );
    client.write_all(head.as_bytes()).await?;
    client.write_all(body.as_bytes()).await
}

// ---- hooks ---------------------------------------------------------------

/// What Guaca answers one hook call with.
///
/// The empty string everywhere there is nothing to say, which is the common
/// case and the one measured to be safe: a hook that prints nothing is recorded
/// by the program as a success with no output and changes nothing about the
/// run.
async fn hook(job: &Job, payload: &serde_json::Value) -> String {
    match payload["hook_event_name"].as_str().unwrap_or_default() {
        // The ordinary delivery. `additionalContext` is put in front of the
        // model before its next round, which is exactly where a correction is
        // worth arriving.
        "PostToolUse" => match take_mail(job) {
            None => String::new(),
            Some(mail) => serde_json::json!({
                "hookSpecificOutput": {
                    "hookEventName": "PostToolUse",
                    "additionalContext": mail,
                }
            })
            .to_string(),
        },

        // The other end of the mailbox, and the reason it cannot loop. `reason`
        // reaches the model as feedback on the refusal to stop, so this both
        // blocks *and* delivers: the mail is taken in the same call that
        // refuses, so a second `Stop` finds nothing pending and lets the run
        // end. A version that blocked without delivering would refuse forever
        // and be killed at the ceiling.
        "Stop" => match take_mail(job) {
            None => String::new(),
            Some(mail) => serde_json::json!({ "decision": "block", "reason": mail }).to_string(),
        },

        "PreToolUse" => gate(job, payload).await,

        _ => String::new(),
    }
}

/// Everything staged for this job, rendered once and taken in the same breath.
///
/// Read and clear together, because a read that left the mail behind would
/// deliver it again on the next tool call, and an instruction a model is given
/// four times is one it was told four times.
fn take_mail(job: &Job) -> Option<String> {
    let staged: Vec<String> = std::mem::take(&mut *job.mail.lock());
    if staged.is_empty() {
        return None;
    }
    let mut said = String::from(
        "The operator sent this while you were working. It is more recent than your \
         brief, so where the two disagree this wins:\n",
    );
    for message in &staged {
        said.push_str("\n- ");
        said.push_str(message);
    }
    Some(said)
}

/// The gate: whether this tool call goes to the operator first.
///
/// Everything that is not an outward-facing shell command answers with nothing
/// at all, which leaves the run's own permission mode to decide, which is the
/// mode that allows it. Only a `deny` is ever written here. There is no `allow`
/// branch and there must not be one: a hook that answered `allow` would be
/// granting things the operator's own settings might have had an opinion about,
/// and this file has no business overriding those in that direction.
async fn gate(job: &Job, payload: &serde_json::Value) -> String {
    if job.gate != Gate::AskBeforePushing {
        return String::new();
    }
    let command = payload["tool_input"]["command"].as_str().unwrap_or_default();
    let Some(reach) = outward(command, &job.root).await else {
        return String::new();
    };

    let (reply, verdict) = oneshot::channel();
    let asking = Signal::Permission { line: command.to_string(), reach, reply };
    if job.signals.send(asking).await.is_err() {
        // Nothing is listening, which means the job's own task has gone. Say
        // nothing and let the run decide: this is the one place a refusal would
        // be about Guaca's plumbing rather than about the operator's answer.
        return String::new();
    }

    // A dropped sender answers `false`. That is a deny, which is the direction
    // a bug here should fail in, and it is why the runtime's side must always
    // send rather than return early.
    if verdict.await.unwrap_or(false) {
        return String::new();
    }

    serde_json::json!({
        "hookSpecificOutput": {
            "hookEventName": "PreToolUse",
            "permissionDecision": "deny",
            "permissionDecisionReason":
                "The operator did not allow this. Do not try it again or work around it: \
                 finish everything else you can, leave the work committed on the branch, \
                 and say in your final message that this step is waiting on them.",
        }
    })
    .to_string()
}

/// What a line reaches outside the work tree with, and how it gets there.
///
/// Two fields rather than one string, because the operator needs both and they
/// are usually not the same words. `git push` says what it does in the words
/// the agent typed. `npm run release` says nothing at all, and a card that
/// repeated that back would be asking somebody to approve something
/// irreversible by its nickname.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Reach {
    /// The outward-facing command itself: `git push`, `gh pr create`.
    pub what: String,
    /// The script in the tree that runs it, when the line does not say so
    /// itself: `package.json "release"`, `scripts/ship.sh`.
    pub through: Option<String>,
}

/// How far the gate follows a line into the tree.
///
/// A script that runs a script that runs a script is already unusual. Four
/// deep is a knot nobody ties by hand, and the bound is what stops a pair of
/// scripts that call each other from being a question with no answer.
const SCRIPT_DEPTH: usize = 3;

/// How many distinct scripts one line may cost.
///
/// A `package.json` with two hundred entries that call each other is not a
/// reason to stat two hundred files in front of one tool call.
const MAX_SCRIPTS: usize = 16;

/// The largest file this reads looking for a push.
///
/// A quarter of a megabyte is a very large shell script and a very small
/// anything else. See [`file_script`] for why a file past it is left alone
/// rather than asked about.
const MAX_SCRIPT_BYTES: u64 = 256 * 1024;

/// What an outward-facing command in this line is, if there is one.
///
/// Outward-facing means it leaves the work tree under the operator's own name
/// and cannot be taken back by git: a push, a pull request, a merge, a release.
/// Everything else, including every edit and every test run, is what the
/// directory and git already cover.
///
/// ## The line is read, and then what the line runs is read
///
/// [`named`] answers about the words themselves, which is the whole of what
/// this used to do, and one level of indirection walked straight past it.
/// `./scripts/ship.sh` is not `git push` and never will be, so a repository
/// whose release is a script had a gate that was on, said it was holding, and
/// stopped nothing. That is worse than no gate: an operator who switched it on
/// was told it was working.
///
/// So a line that names nothing is looked at again for the scripts in *this
/// tree* that it runs, and those are read and asked the same question. A
/// package script and a file in the work tree, up to [`SCRIPT_DEPTH`] deep.
///
/// ## What it deliberately does not read
///
/// A Makefile target, a compiled program, an interpreted script it cannot
/// parse as text. Those are all indirection too, and reading none of them is
/// the decision rather than the gap. The alternative is to treat "there is
/// something here I cannot see through" as a reason to ask, and the ordinary
/// case for that is `./target/release/app` and `./node_modules/.bin/vite`:
/// every locally built program in the tree, on every line that runs one. A
/// gate that parks a turn for running the binary it just compiled is the
/// wrong-yes that teaches an operator to switch it off, and then it holds
/// nothing at all.
///
/// This is a judgment about the ordinary case and not a boundary. A shell line
/// is not something anything can parse without a shell, and a job that wanted
/// to get around this could: the process runs as the operator with their
/// credentials either way, which was true before this existed and is stated at
/// the top of this file and in `docs/CODING.md`. What it buys is that the
/// ordinary push, made by a job doing what it was asked, is one the operator
/// sees first — whether the job spells it out or keeps it in a script.
///
/// It errs toward asking. A wrong yes costs one prompt on the desk; a wrong no
/// is the behavior the operator switched this on to stop.
pub async fn outward(line: &str, root: &Path) -> Option<Reach> {
    if let Some(what) = named(line) {
        return Some(Reach { what, through: None });
    }

    // Once, rather than per candidate: every path found below is checked
    // against it, and a root that is not there resolves nothing, which is the
    // answer this gave before any of this existed.
    let root = tokio::fs::canonicalize(root).await.ok()?;

    let mut seen = HashSet::new();
    let mut frontier = scripts_in(&root, line, &mut seen).await;
    for _ in 0..SCRIPT_DEPTH {
        let mut deeper = Vec::new();
        for (script, body) in &frontier {
            if let Some(what) = named(body) {
                return Some(Reach { what, through: Some(script.clone()) });
            }
            // The script the *line* ran is what is carried down, not the one
            // that happens to hold the push. An operator asked about
            // `package.json "release"` can go and read it; asked about the
            // third file in a chain they have to reconstruct how the agent got
            // there before they can answer.
            for (_, further) in scripts_in(&root, body, &mut seen).await {
                deeper.push((script.clone(), further));
            }
        }
        if deeper.is_empty() {
            break;
        }
        frontier = deeper;
    }
    None
}

/// What this line says itself, with nothing read off the disk.
///
/// Split out of [`outward`] because it is asked twice: once about the words
/// the agent wrote, and once per script those words turn out to run.
fn named(line: &str) -> Option<String> {
    for segment in segments(line) {
        // `continue` rather than `?`, and that is a fix rather than a
        // preference. A segment with no program in it used to end the whole
        // walk, so `(cd sub); git push` — which `segments` splits into an
        // empty piece and then the push — answered that nothing here reaches
        // outside the repository.
        let Some((word, args)) = invocation(&segment) else {
            continue;
        };
        let program = word.rsplit('/').next().unwrap_or(&word);
        let args: Vec<&str> = args.iter().map(String::as_str).collect();

        let verb = match program {
            "git" => git_subcommand(&args),
            "gh" | "glab" => hosted_subcommand(&args),
            _ => None,
        };
        if let Some(verb) = verb {
            return Some(format!("{program} {verb}"));
        }
    }
    None
}

/// The program one segment runs, as the word that named it, and its arguments.
///
/// A wrapper is how the same command arrives wearing a hat. A leading
/// environment assignment, `sudo`, and a shell handed the whole line as one
/// `-c` argument are the three that turn up in practice, and none of them
/// changes what actually runs. Flags are skipped with them, which is what takes
/// the `-c` off in the third case.
///
/// The word comes back whole rather than as its base name, because the two
/// callers want different halves of it: `git` is a program name and
/// `scripts/ship.sh` is a path, and taking the base name early is what left
/// nothing to resolve the second one against.
fn invocation(segment: &str) -> Option<(String, Vec<String>)> {
    let mut rest = words(segment).into_iter();
    let program = loop {
        let word = rest.next()?;
        let skip = {
            let bare = word.rsplit('/').next().unwrap_or(word.as_str());
            WRAPPERS.contains(&bare) || bare.starts_with('-') || word.contains('=')
        };
        if !skip {
            break word;
        }
    };
    Some((program, rest.collect()))
}

/// Every script in this tree that the line runs, with the text of each.
///
/// One level: what comes back is fed to [`named`], and to this again. The pair
/// is what the operator would need to see — the script named as they would
/// recognize it, and its body for the same rule to read.
///
/// `seen` is shared across the whole walk rather than per level, so a pair of
/// scripts that run each other is read once and a file reached two ways is read
/// once. It is also the budget: [`MAX_SCRIPTS`] distinct scripts per line.
async fn scripts_in(root: &Path, line: &str, seen: &mut HashSet<String>) -> Vec<(String, String)> {
    let mut out = Vec::new();
    for segment in segments(line) {
        if seen.len() >= MAX_SCRIPTS {
            break;
        }
        let Some((word, args)) = invocation(&segment) else {
            continue;
        };
        let found = match word.rsplit('/').next().unwrap_or(&word) {
            "npm" | "pnpm" | "yarn" | "bun" => package_script(root, &args).await,
            _ => file_script(root, &word).await,
        };
        let Some((script, body)) = found else {
            continue;
        };
        if seen.insert(script.clone()) {
            out.push((script, body));
        }
    }
    out
}

/// The body of the package script this invocation runs, if it runs one.
///
/// `npm run release`, `pnpm release` and `yarn release` all reach the same
/// entry in the same file, so they are one case rather than three. A name with
/// no entry behind it is a command that is about to fail, which is nothing to
/// look into and nothing to ask about.
///
/// The file is the one at the root of the repository. A workspace that keeps
/// its release script in a package one directory down is not read, for the
/// reason in [`outward`]: this follows the ordinary case and says so.
async fn package_script(root: &Path, args: &[String]) -> Option<(String, String)> {
    let mut plain = args.iter().filter(|arg| !arg.starts_with('-'));
    let mut name = plain.next()?.as_str();
    // `npm run release` and `pnpm release` are the same request. Anything else
    // in that position is a name, and a name with no script behind it stops
    // this a line later.
    if matches!(name, "run" | "run-script" | "exec") {
        name = plain.next()?.as_str();
    }
    let text = read_text(&root.join("package.json")).await?;
    let manifest: serde_json::Value = serde_json::from_str(&text).ok()?;
    let body = manifest.get("scripts")?.get(name)?.as_str()?;
    Some((format!("package.json \"{name}\""), body.to_string()))
}

/// The text of the script this word runs, if it is one inside the tree.
///
/// Inside the work tree and readable as text is the whole of the test, and both
/// halves are load-bearing. A path that resolves outside the repository is not
/// this job's tree and is read by nothing here. Something that is not text is
/// something this cannot judge, and is left exactly as it was found: see
/// [`outward`] on why that is the decision and not the gap.
async fn file_script(root: &Path, word: &str) -> Option<(String, String)> {
    // `join` on an absolute path replaces rather than appends, so an absolute
    // word arrives here as itself. That is what makes the check below the thing
    // that keeps this inside the repository, rather than the join.
    let path = tokio::fs::canonicalize(root.join(word)).await.ok()?;
    if !path.starts_with(root) {
        return None;
    }
    Some((word.to_string(), read_text(&path).await?))
}

/// A file's text, when it is a file, is small enough to be a script, and is
/// text at all.
///
/// The size is asked of the metadata rather than of what was read, so a huge
/// file costs a `stat` instead of its own length in memory.
async fn read_text(path: &Path) -> Option<String> {
    let meta = tokio::fs::metadata(path).await.ok()?;
    if !meta.is_file() || meta.len() > MAX_SCRIPT_BYTES {
        return None;
    }
    String::from_utf8(tokio::fs::read(path).await.ok()?).ok()
}

/// Programs that run another program, and are therefore not the answer.
///
/// A shell is on the list because `sh -c "git push"` reaches the network
/// exactly as `git push` does, and the words are the same words once the quotes
/// come off.
const WRAPPERS: [&str; 10] =
    ["sudo", "command", "env", "nohup", "time", "xargs", "sh", "bash", "zsh", "dash"];

/// Splits a shell line into the commands it runs.
///
/// Crude on purpose. It splits on the separators that start a new command and
/// keeps quoting out of it, because the thing being looked for is a program
/// name and a subcommand, and no amount of shell grammar here would make this a
/// boundary. See [`outward`].
fn segments(line: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut current = String::new();
    let mut chars = line.chars().peekable();
    while let Some(ch) = chars.next() {
        match ch {
            ';' | '\n' | '&' | '|' => {
                // `&&` and `||` collapse into the one separator they are.
                if chars.peek() == Some(&ch) {
                    chars.next();
                }
                out.push(std::mem::take(&mut current));
            }
            '(' | ')' | '{' | '}' => out.push(std::mem::take(&mut current)),
            _ => current.push(ch),
        }
    }
    out.push(current);
    out
}

/// One segment's words, with the quotes taken off.
///
/// `sh -c "git push"` is the case this exists for: the inner line arrives as one
/// quoted word, so stripping the quotes and splitting again is what finds the
/// program inside it.
fn words(segment: &str) -> Vec<String> {
    segment
        .split_whitespace()
        .map(|word| word.trim_matches(['"', '\'', '`']).to_string())
        .filter(|word| !word.is_empty())
        .collect()
}

/// Git's subcommand, if it is one that leaves the machine.
///
/// The walk over global options is what makes `git -C /repo push` and `git push`
/// the same answer. Reading "any argument equal to `push`" instead would call
/// `git log --grep push` an outward-facing command, which is the kind of wrong
/// no that teaches an operator to switch the gate off.
fn git_subcommand(args: &[&str]) -> Option<String> {
    let mut rest = args.iter();
    while let Some(arg) = rest.next() {
        match *arg {
            // Global options that take a value, so the value is not the
            // subcommand.
            "-C" | "-c" | "--git-dir" | "--work-tree" | "--namespace" | "--exec-path" => {
                rest.next();
            }
            other if other.starts_with('-') => {}
            "push" => return Some("push".to_string()),
            _ => return None,
        }
    }
    None
}

/// A forge CLI's topic and verb, if the pair reaches outside the repository.
///
/// Named rather than pattern-matched. `gh pr view` and `gh pr create` differ by
/// one word and by everything else, and a rule about the topic alone would park
/// a job for reading its own pull request.
fn hosted_subcommand(args: &[&str]) -> Option<String> {
    let plain: Vec<&str> = args.iter().copied().filter(|arg| !arg.starts_with('-')).collect();
    let topic = plain.first()?;
    let verb = plain.get(1).copied().unwrap_or_default();

    let reaches = match *topic {
        "pr" | "mr" => matches!(
            verb,
            "create" | "merge" | "close" | "reopen" | "comment" | "edit" | "ready" | "review"
        ),
        "release" => matches!(verb, "create" | "edit" | "delete" | "upload"),
        "issue" => matches!(verb, "create" | "close" | "reopen" | "comment" | "edit" | "delete"),
        "repo" => matches!(verb, "create" | "delete" | "edit" | "fork" | "sync"),
        "workflow" | "run" => matches!(verb, "run" | "dispatch" | "cancel" | "rerun"),
        // `gh api` is every one of the above with the topic taken off, so it is
        // read by its method instead. A GET is a read and is left alone.
        "api" => args.iter().any(|arg| {
            matches!(*arg, "POST" | "PATCH" | "PUT" | "DELETE")
                || matches!(*arg, "-f" | "--field" | "-F" | "--raw-field")
        }),
        _ => false,
    };

    reaches.then(|| match *topic {
        "api" => "api".to_string(),
        _ => format!("{topic} {verb}").trim().to_string(),
    })
}

// ---- the job's own MCP server --------------------------------------------

/// What a job may tell Guaca, deliberately, rather than in its narration.
///
/// Two, and there is not a third. In particular there is no way to ask the
/// operator a question: [`super::APPENDED_PROMPT`] tells a job that nobody will
/// answer one, and a tool that contradicted it would invite a run to spend ten
/// of its forty-five minutes waiting on somebody who is not there. Where a
/// human genuinely has to be asked, it is Guaca that decides, in [`gate`], about
/// something it can name.
fn tools() -> serde_json::Value {
    serde_json::json!([
        {
            "name": "note_progress",
            "description":
                "Tell the operator watching this job what you are doing. Use it when you \
                 have finished something that stands on its own, or when you are about to \
                 start something long, so somebody reading over your shoulder knows where \
                 you are. One sentence. It reaches a person, not a model, and it is not a \
                 substitute for your final message.",
            "inputSchema": {
                "type": "object",
                "additionalProperties": false,
                "required": ["note"],
                "properties": {
                    "note": { "type": "string", "description": "One sentence, in plain words." }
                },
            },
        },
        {
            "name": "report_pull_request",
            "description":
                "Report a pull request you opened. Call this straight after opening one. \
                 Guaca records the link against this job, so the agent that asked for the \
                 work and the operator both get it as a link rather than having to find it \
                 in your closing paragraph.",
            "inputSchema": {
                "type": "object",
                "additionalProperties": false,
                "required": ["url", "branch"],
                "properties": {
                    "url": { "type": "string", "description": "The full URL of the pull request." },
                    "branch": { "type": "string", "description": "The branch it was opened from." },
                },
            },
        },
    ])
}

/// Answers one JSON-RPC request, or `None` for a notification.
///
/// Both protocol eras are answered, because which one is used is the client's
/// choice and not this server's. Measured against 2.1.247 the program probes
/// `server/discover` and then falls back to `initialize` anyway, so the
/// handshake is the path that actually runs today; `server/discover` is here so
/// that stops being true without this becoming a server nothing can talk to.
/// `crate::mcp` is the other end of the same two eras, from the client side.
async fn rpc(job: &Job, request: &serde_json::Value) -> Option<serde_json::Value> {
    let id = request.get("id").cloned()?;
    let method = request["method"].as_str().unwrap_or_default();

    let result = match method {
        "initialize" => {
            let asked =
                request["params"]["protocolVersion"].as_str().unwrap_or(crate::mcp::LEGACY_VERSION);
            serde_json::json!({
                "protocolVersion": asked,
                "capabilities": { "tools": {} },
                "serverInfo": { "name": "guaca", "version": env!("CARGO_PKG_VERSION") },
            })
        }
        "server/discover" => serde_json::json!({
            "protocolVersion": crate::mcp::PROTOCOL_VERSION,
            "capabilities": { "tools": {} },
            "serverInfo": { "name": "guaca", "version": env!("CARGO_PKG_VERSION") },
        }),
        "tools/list" => serde_json::json!({ "tools": tools() }),
        "tools/call" => {
            let name = request["params"]["name"].as_str().unwrap_or_default();
            let args = &request["params"]["arguments"];
            match call(job, name, args).await {
                Ok(said) => serde_json::json!({ "content": [{ "type": "text", "text": said }] }),
                // A tool failure is reported inside the result rather than as a
                // JSON-RPC error, which is what the protocol says and what lets
                // the model read it and act on it instead of seeing a transport
                // fault.
                Err(why) => serde_json::json!({
                    "content": [{ "type": "text", "text": why }],
                    "isError": true,
                }),
            }
        }
        _ => {
            return Some(serde_json::json!({
                "jsonrpc": "2.0",
                "id": id,
                "error": { "code": -32601, "message": format!("no method {method}") },
            }))
        }
    };

    Some(serde_json::json!({ "jsonrpc": "2.0", "id": id, "result": result }))
}

/// Runs one of the two tools.
///
/// Every refusal says what to do about it, because a coding harness reads this
/// mid-run and a refusal that only says no gets reworded and retried.
async fn call(job: &Job, name: &str, args: &serde_json::Value) -> Result<String, String> {
    let signal = match name {
        "note_progress" => {
            let note = args["note"].as_str().unwrap_or_default().trim();
            if note.is_empty() {
                return Err("A progress note needs a sentence in `note`. Say what you \
                            just finished or what you are starting."
                    .to_string());
            }
            Signal::Note(super::one_line(note))
        }
        "report_pull_request" => {
            let url = args["url"].as_str().unwrap_or_default().trim();
            let branch = args["branch"].as_str().unwrap_or_default().trim();
            if !url.starts_with("https://") {
                return Err("A pull request is reported by its full URL, starting with \
                            `https://`. Pass the link the forge printed when you opened it."
                    .to_string());
            }
            if branch.is_empty() {
                return Err("`branch` is the branch the pull request was opened from. \
                            `git branch --show-current` is it."
                    .to_string());
            }
            Signal::PullRequest { url: super::one_line(url), branch: super::one_line(branch) }
        }
        _ => {
            return Err(format!(
                "Guaca offers `note_progress` and `report_pull_request` and nothing else. \
                 `{name}` is not one of them."
            ))
        }
    };

    match job.signals.send(signal).await {
        Ok(()) => Ok("Noted.".to_string()),
        // The job's own task is gone, which from in here means the run is being
        // torn down. Said plainly rather than as a success, so a harness that
        // is still going does not report a link nobody received.
        Err(_) => Err("Guaca is no longer listening to this job, so that was not \
                       recorded. Put it in your final message instead."
            .to_string()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn a_job(gate: Gate) -> (Arc<Job>, mpsc::Receiver<Signal>) {
        job_in(gate, PathBuf::from("/nonexistent-work-tree"))
    }

    fn job_in(gate: Gate, root: PathBuf) -> (Arc<Job>, mpsc::Receiver<Signal>) {
        let (signals, heard) = mpsc::channel(8);
        (Arc::new(Job { signals, gate, root, mail: Mutex::new(Vec::new()) }), heard)
    }

    /// A work tree with the given files in it, and nothing else.
    fn a_tree(name: &str, files: &[(&str, &str)]) -> PathBuf {
        let root = std::env::temp_dir().join(format!("guac-gate-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        for (path, body) in files {
            let at = root.join(path);
            std::fs::create_dir_all(at.parent().unwrap()).unwrap();
            std::fs::write(&at, body).unwrap();
        }
        std::fs::create_dir_all(&root).unwrap();
        std::fs::canonicalize(&root).unwrap()
    }

    /// What the gate makes of a line, with no tree to read.
    async fn reach(line: &str) -> Option<Reach> {
        outward(line, Path::new("/nonexistent-work-tree")).await
    }

    // ---- what a job is started with --------------------------------------

    #[test]
    fn the_gate_is_the_only_hook_that_is_conditional() {
        // The mailbox is why this exists at all, so its two hooks are on every
        // job. The gate costs a round trip per shell command and is off unless
        // the operator asked for it.
        let dir = std::env::temp_dir().join(format!("guaca-settings-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        let at = dir.join("settings.json");

        write_settings(&at, Path::new("/tmp/hook.sh"), Gate::Open).unwrap();
        let open: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(&at).unwrap()).unwrap();
        assert!(open["hooks"]["PostToolUse"].is_array());
        assert!(open["hooks"]["Stop"].is_array());
        assert!(open["hooks"]["PreToolUse"].is_null(), "no gate, no cost");

        write_settings(&at, Path::new("/tmp/hook.sh"), Gate::AskBeforePushing).unwrap();
        let asked: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(&at).unwrap()).unwrap();
        // `Bash` rather than `*`: everything outward-facing goes through a
        // shell, and a hook on every `Read` is a round trip whose answer is
        // always yes.
        assert_eq!(asked["hooks"]["PreToolUse"][0]["matcher"], "Bash");

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn the_hook_script_fails_open_and_carries_its_own_address() {
        let dir = std::env::temp_dir().join(format!("guaca-script-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        let at = dir.join("hook.sh");
        write_script(&at, 4242, "abc").unwrap();
        let script = std::fs::read_to_string(&at).unwrap();

        assert!(script.contains("http://127.0.0.1:4242/abc/hook"));
        // The whole of the failure handling. No curl, no bridge and no answer
        // all have to leave the job running as it did before this existed.
        assert!(script.contains("exit 0"), "{script}");
        assert!(script.contains("2>/dev/null"), "{script}");
        // Longer than the ten minutes a parked approval may take, or the gate
        // grants whenever the operator is slow.
        const { assert!(HOOK_TIMEOUT_SECS > 10 * 60) };

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn the_operators_own_servers_and_settings_are_added_to_and_not_replaced() {
        // The one thing that makes a coding job different from a turn: it wants
        // the operator's tools. `--strict-mcp-config` is what would take them
        // away and it is deliberately not in the vector.
        let config: serde_json::Value = serde_json::from_str(&mcp_config(1, "t")).unwrap();
        assert_eq!(config["mcpServers"]["guaca"]["url"], "http://127.0.0.1:1/t/mcp");
        assert_eq!(config["mcpServers"]["guaca"]["type"], "http");
    }

    // ---- addressing ------------------------------------------------------

    #[test]
    fn serves_only_a_uuid_shaped_token_on_one_of_two_routes() {
        let id = "f96abc0a-5404-4e51-b465-b96677118cf9";
        let line = |raw: &str| asked(format!("{raw} HTTP/1.1\r\n\r\n").as_bytes());

        assert_eq!(
            line(&format!("POST /{id}/hook")),
            Some(Asked { token: id.to_string(), route: Route::Hook })
        );
        assert_eq!(line(&format!("POST /{id}/mcp")).map(|a| a.route), Some(Route::Mcp));
        // A token goes on to be a map key, so its shape is checked here rather
        // than trusted.
        assert_eq!(line("POST /../../etc/passwd/hook"), None);
        assert_eq!(line(&format!("POST /{id}/other")), None);
        assert_eq!(line(&format!("POST /{id}/hook/again")), None);
        assert_eq!(line(&format!("GET /{id}/hook")), None, "nothing here is a read");
    }

    // ---- the mailbox -----------------------------------------------------

    #[tokio::test]
    async fn a_message_reaches_the_model_at_the_next_tool_boundary() {
        let (job, _heard) = a_job(Gate::Open);
        job.mail.lock().push("stop and use the other endpoint".into());

        let said = hook(&job, &serde_json::json!({ "hook_event_name": "PostToolUse" })).await;
        let out: serde_json::Value = serde_json::from_str(&said).unwrap();
        let context = out["hookSpecificOutput"]["additionalContext"].as_str().unwrap();
        assert!(context.contains("stop and use the other endpoint"));
        // A correction is more recent than the brief, and a model handed two
        // instructions with nothing ordering them picks one.
        assert!(context.contains("this wins"), "{context}");
    }

    #[tokio::test]
    async fn mail_is_delivered_once_and_never_again() {
        // Read and cleared together. Delivered twice is an instruction the
        // model was given twice, which is how a job ends up doing something
        // once for each time it was mentioned.
        let (job, _heard) = a_job(Gate::Open);
        job.mail.lock().push("use the staging bucket".into());

        let first = hook(&job, &serde_json::json!({ "hook_event_name": "PostToolUse" })).await;
        assert!(first.contains("staging bucket"));
        let second = hook(&job, &serde_json::json!({ "hook_event_name": "PostToolUse" })).await;
        assert_eq!(second, "", "nothing pending is nothing said");
    }

    #[tokio::test]
    async fn a_job_about_to_finish_is_held_open_and_told_in_the_same_breath() {
        // The case the `Stop` hook exists for: a correction typed at minute
        // forty-four. Without this it lands after the job decided it was done.
        let (job, _heard) = a_job(Gate::Open);
        job.mail.lock().push("do not merge it yet".into());

        let said = hook(&job, &serde_json::json!({ "hook_event_name": "Stop" })).await;
        let out: serde_json::Value = serde_json::from_str(&said).unwrap();
        assert_eq!(out["decision"], "block");
        // Blocking and delivering in one call is what makes this terminate. A
        // block that did not deliver would refuse to stop forever and be killed
        // at the ceiling.
        assert!(out["reason"].as_str().unwrap().contains("do not merge it yet"));

        let again = hook(&job, &serde_json::json!({ "hook_event_name": "Stop" })).await;
        assert_eq!(again, "", "a second stop finds nothing pending and lets the run end");
    }

    #[tokio::test]
    async fn an_ordinary_tool_call_costs_nothing_to_say_nothing_about() {
        let (job, _heard) = a_job(Gate::Open);
        assert_eq!(hook(&job, &serde_json::json!({ "hook_event_name": "PostToolUse" })).await, "");
        assert_eq!(hook(&job, &serde_json::json!({ "hook_event_name": "SessionStart" })).await, "");
        assert_eq!(hook(&job, &serde_json::json!({})).await, "");
    }

    #[test]
    fn the_newest_corrections_are_the_ones_kept() {
        let bridge = Bridge::new();
        let (signals, _heard) = mpsc::channel(8);
        bridge.inner.registry.lock().jobs.insert(
            "t".into(),
            Arc::new(Job {
                signals,
                gate: Gate::Open,
                root: PathBuf::from("/nonexistent-work-tree"),
                mail: Mutex::new(Vec::new()),
            }),
        );

        for n in 0..MAX_STAGED + 3 {
            assert!(bridge.post("t", &format!("correction {n}")));
        }
        assert_eq!(bridge.pending("t"), MAX_STAGED);

        // Blank is not a correction, and a job nobody is running cannot be sent
        // one: both are the panel's to report rather than to swallow.
        assert!(!bridge.post("t", "   "));
        assert!(!bridge.post("nobody", "hello"));
    }

    #[test]
    fn a_pasted_document_is_cut_rather_than_delivered_whole() {
        let bridge = Bridge::new();
        let (signals, _heard) = mpsc::channel(8);
        bridge.inner.registry.lock().jobs.insert(
            "t".into(),
            Arc::new(Job {
                signals,
                gate: Gate::Open,
                root: PathBuf::from("/nonexistent-work-tree"),
                mail: Mutex::new(Vec::new()),
            }),
        );
        assert!(bridge.post("t", &"x".repeat(MAX_MESSAGE_LEN * 2)));

        let job = bridge.job("t").unwrap();
        let held = job.mail.lock()[0].clone();
        assert!(held.chars().count() <= MAX_MESSAGE_LEN + 1, "{}", held.chars().count());
        assert!(held.ends_with('…'));
    }

    // ---- the gate --------------------------------------------------------

    #[tokio::test]
    async fn a_push_is_outward_facing_however_it_is_spelled() {
        for line in [
            "git push",
            "git push --force-with-lease origin HEAD",
            "git -C /srv/app push",
            "git -c user.name=bot push origin main",
            "cd /repo && git push",
            "npm test && git push origin feature",
            "sh -c 'git push'",
            "sudo git push",
            // A subshell puts an empty piece in front of the push. The walk
            // used to end on it and answer that nothing here reaches outside.
            "(cd sub); git push",
        ] {
            assert!(reach(line).await.is_some(), "{line}");
        }
    }

    #[tokio::test]
    async fn reading_is_never_outward_facing() {
        // The wrong `no` here is a prompt on the desk. The wrong `yes` is a
        // job parked for reading its own history, which is what teaches an
        // operator to switch the gate off.
        for line in [
            "git status",
            "git log --grep push",
            "git commit -m 'push the button'",
            "git fetch origin",
            "git branch --show-current",
            "gh pr view 12",
            "gh pr list",
            "gh api /repos/o/r",
            "cargo test",
            "echo git push",
            "",
        ] {
            assert_eq!(reach(line).await, None, "{line}");
        }
    }

    #[tokio::test]
    async fn opening_and_merging_reach_outside_and_looking_does_not() {
        let what = |line: &'static str| async move { reach(line).await.map(|r| r.what) };
        assert_eq!(what("gh pr create --fill").await.as_deref(), Some("gh pr create"));
        assert_eq!(what("gh pr merge 12 --squash").await.as_deref(), Some("gh pr merge"));
        assert_eq!(what("gh release create v1.2.0").await.as_deref(), Some("gh release create"));
        assert_eq!(what("glab mr create").await.as_deref(), Some("glab mr create"));
        // `gh api` is every one of those with the topic taken off, so it is
        // read by its method instead.
        assert!(what("gh api -X POST /repos/o/r/issues").await.is_some());
        assert!(what("gh api --method DELETE /repos/o/r").await.is_some());
        assert_eq!(what("gh api /user").await, None, "a read is a read");
    }

    // ---- and what the line runs ------------------------------------------

    /// The gap this was built to close.
    ///
    /// A repository whose release is a script had a gate that was on, said it
    /// was holding, and stopped nothing: `./scripts/ship.sh` is not `git push`
    /// and no amount of reading the words would ever make it one.
    #[tokio::test]
    async fn a_push_kept_in_a_script_is_still_a_push() {
        let root = a_tree(
            "in-a-script",
            &[
                (
                    "scripts/ship.sh",
                    "#!/bin/sh\nset -e\ncargo build --release\ngit push origin main\n",
                ),
                ("package.json", r#"{"scripts":{"release":"pnpm build && ./scripts/ship.sh"}}"#),
            ],
        );

        for (line, through) in [
            ("./scripts/ship.sh", "./scripts/ship.sh"),
            ("scripts/ship.sh --yes", "scripts/ship.sh"),
            ("bash scripts/ship.sh", "scripts/ship.sh"),
            ("cd . && ./scripts/ship.sh", "./scripts/ship.sh"),
            ("npm run release", "package.json \"release\""),
            ("pnpm release", "package.json \"release\""),
            ("yarn run release", "package.json \"release\""),
        ] {
            let found = outward(line, &root).await.unwrap_or_else(|| panic!("{line}"));
            assert_eq!(found.what, "git push", "{line}");
            // Named as the operator would recognize it. Asked about the third
            // file in a chain they would have to work out how the agent got
            // there before they could answer.
            assert_eq!(found.through.as_deref(), Some(through), "{line}");
        }

        let _ = std::fs::remove_dir_all(&root);
    }

    /// And the ordinary script is still ordinary.
    #[tokio::test]
    async fn a_script_that_does_not_reach_outside_asks_nobody() {
        let root = a_tree(
            "plain-script",
            &[
                ("scripts/ci.sh", "#!/bin/sh\ncargo test\ngit status --porcelain\n"),
                ("package.json", r#"{"scripts":{"test":"vitest run","build":"vite build"}}"#),
            ],
        );

        for line in [
            "./scripts/ci.sh",
            "npm test",
            "pnpm build",
            // No entry behind the name is a command about to fail, which is
            // nothing to look into and nothing to ask about.
            "npm run nonexistent",
            // Outside the tree is not this job's tree.
            "/usr/bin/env",
            "../elsewhere/ship.sh",
        ] {
            assert_eq!(outward(line, &root).await, None, "{line}");
        }

        let _ = std::fs::remove_dir_all(&root);
    }

    /// A script that runs a script that pushes.
    ///
    /// And a pair that run each other, which is the reason the walk carries a
    /// set rather than a depth alone: without it this is a question with no
    /// answer rather than a slow one.
    #[tokio::test]
    async fn the_walk_goes_deeper_than_one_and_survives_a_cycle() {
        let root = a_tree(
            "deep",
            &[
                ("a.sh", "./b.sh\n"),
                ("b.sh", "./c.sh\n"),
                ("c.sh", "git push\n"),
                ("loop-one.sh", "./loop-two.sh\n"),
                ("loop-two.sh", "./loop-one.sh\n"),
            ],
        );

        let found = outward("./a.sh", &root).await.unwrap();
        assert_eq!(found.what, "git push");
        assert_eq!(found.through.as_deref(), Some("./a.sh"), "the line's own script is named");

        assert_eq!(outward("./loop-one.sh", &root).await, None);

        let _ = std::fs::remove_dir_all(&root);
    }

    /// What it deliberately does not read.
    ///
    /// A compiled program in the tree is indirection this cannot see through,
    /// and answering "ask" for it would park a turn for every locally built
    /// binary on every line that runs one. That is the wrong yes that teaches
    /// an operator to switch the gate off, after which it holds nothing.
    #[tokio::test]
    async fn something_this_cannot_read_is_left_exactly_as_it_was_found() {
        let root = a_tree("unreadable", &[("scripts/keep", "keep")]);
        std::fs::write(root.join("target-app"), [0xff, 0xfe, 0x00, 0x01]).unwrap();
        std::fs::write(root.join("huge.sh"), "x".repeat(MAX_SCRIPT_BYTES as usize + 1)).unwrap();

        assert_eq!(outward("./target-app --serve", &root).await, None);
        assert_eq!(outward("./huge.sh", &root).await, None);
        assert_eq!(outward("./not-there.sh", &root).await, None);
        // A directory is not a script either.
        assert_eq!(outward("./scripts", &root).await, None);

        let _ = std::fs::remove_dir_all(&root);
    }

    #[tokio::test]
    async fn nothing_stops_on_a_repository_the_operator_did_not_gate() {
        let (job, mut heard) = a_job(Gate::Open);
        let asked = serde_json::json!({
            "hook_event_name": "PreToolUse",
            "tool_name": "Bash",
            "tool_input": { "command": "git push" },
        });
        assert_eq!(hook(&job, &asked).await, "", "the default is what every job did before");
        assert!(heard.try_recv().is_err(), "and it does not reach the desk either");
    }

    #[tokio::test]
    async fn a_gated_push_waits_for_the_operator_and_a_no_is_a_deny() {
        let (job, mut heard) = a_job(Gate::AskBeforePushing);
        let asked = serde_json::json!({
            "hook_event_name": "PreToolUse",
            "tool_name": "Bash",
            "tool_input": { "command": "git push origin main" },
        });

        let answering = tokio::spawn(async move {
            let Some(Signal::Permission { line, reach, reply }) = heard.recv().await else {
                panic!("the gate has to reach the desk");
            };
            assert_eq!(reach.what, "git push");
            assert_eq!(reach.through, None);
            // Both halves reach the desk: the operator is answering about the
            // push, and the flags on it are what decides the answer.
            assert_eq!(line, "git push origin main");
            reply.send(false).unwrap();
        });

        let said = hook(&job, &asked).await;
        answering.await.unwrap();

        let out: serde_json::Value = serde_json::from_str(&said).unwrap();
        assert_eq!(out["hookSpecificOutput"]["permissionDecision"], "deny");
        // A refusal an agent reads mid-run needs a way forward, not just a
        // reason, or it gets reworded and retried.
        let why = out["hookSpecificOutput"]["permissionDecisionReason"].as_str().unwrap();
        assert!(why.contains("waiting on them"), "{why}");
        assert!(why.contains("committed"), "{why}");
    }

    #[tokio::test]
    async fn a_yes_says_nothing_and_lets_the_run_decide() {
        // There is no `allow` branch and there must not be one: answering
        // `allow` would override whatever the operator's own settings had to
        // say, which this file has no business doing in that direction.
        let (job, mut heard) = a_job(Gate::AskBeforePushing);
        let answering = tokio::spawn(async move {
            let Some(Signal::Permission { reply, .. }) = heard.recv().await else { panic!() };
            reply.send(true).unwrap();
        });
        let said = hook(
            &job,
            &serde_json::json!({
                "hook_event_name": "PreToolUse",
                "tool_input": { "command": "git push" },
            }),
        )
        .await;
        answering.await.unwrap();
        assert_eq!(said, "");
    }

    #[tokio::test]
    async fn a_gate_nobody_is_listening_to_says_nothing_rather_than_refusing() {
        // The job's own task has gone, which means the run is being torn down.
        // A refusal here would be about Guaca's plumbing rather than about the
        // operator's answer.
        let (job, heard) = a_job(Gate::AskBeforePushing);
        drop(heard);
        let said = hook(
            &job,
            &serde_json::json!({
                "hook_event_name": "PreToolUse",
                "tool_input": { "command": "git push" },
            }),
        )
        .await;
        assert_eq!(said, "");
    }

    #[tokio::test]
    async fn an_operator_who_never_answers_is_a_deny() {
        // A dropped sender is the shape a bug takes, and it has to fail toward
        // refusing rather than toward granting.
        let (job, mut heard) = a_job(Gate::AskBeforePushing);
        let answering = tokio::spawn(async move {
            let Some(Signal::Permission { reply, .. }) = heard.recv().await else { panic!() };
            drop(reply);
        });
        let said = hook(
            &job,
            &serde_json::json!({
                "hook_event_name": "PreToolUse",
                "tool_input": { "command": "git push" },
            }),
        )
        .await;
        answering.await.unwrap();
        assert!(said.contains("deny"), "{said}");
    }

    // ---- the job's own tools ---------------------------------------------

    #[tokio::test]
    async fn a_reported_pull_request_arrives_as_a_value() {
        let (job, mut heard) = a_job(Gate::Open);
        let answered = rpc(
            &job,
            &serde_json::json!({
                "jsonrpc": "2.0", "id": 4, "method": "tools/call",
                "params": {
                    "name": "report_pull_request",
                    "arguments": { "url": "https://github.com/o/r/pull/12", "branch": "fix/flaky" },
                },
            }),
        )
        .await
        .unwrap();
        assert!(answered["result"]["isError"].is_null());

        match heard.try_recv().unwrap() {
            Signal::PullRequest { url, branch } => {
                assert_eq!(url, "https://github.com/o/r/pull/12");
                assert_eq!(branch, "fix/flaky");
            }
            other => panic!("{other:?}"),
        }
    }

    #[tokio::test]
    async fn a_refusal_says_what_to_do_about_it() {
        // Read by a coding harness mid-run. One that only says no gets reworded
        // and retried, which costs the job a round trip per attempt.
        let (job, _heard) = a_job(Gate::Open);
        let refused = |args: serde_json::Value, name: &str| {
            let job = job.clone();
            let name = name.to_string();
            async move {
                let answered = rpc(
                    &job,
                    &serde_json::json!({
                        "jsonrpc": "2.0", "id": 1, "method": "tools/call",
                        "params": { "name": name, "arguments": args },
                    }),
                )
                .await
                .unwrap();
                assert_eq!(answered["result"]["isError"], true);
                answered["result"]["content"][0]["text"].as_str().unwrap().to_string()
            }
        };

        let no_url = refused(serde_json::json!({ "branch": "x" }), "report_pull_request").await;
        assert!(no_url.contains("https://"), "{no_url}");
        let no_branch =
            refused(serde_json::json!({ "url": "https://x/pull/1" }), "report_pull_request").await;
        assert!(no_branch.contains("git branch --show-current"), "{no_branch}");
        let empty = refused(serde_json::json!({ "note": "  " }), "note_progress").await;
        assert!(empty.contains("just finished"), "{empty}");
        let unknown = refused(serde_json::json!({}), "ask_operator").await;
        assert!(unknown.contains("note_progress"), "{unknown}");
    }

    #[tokio::test]
    async fn there_is_no_way_for_a_job_to_ask_a_question() {
        // The appended prompt tells a job that nobody will answer one, and a
        // tool that contradicted it would invite a run to spend ten of its
        // forty-five minutes waiting on somebody who is not there.
        let names: Vec<String> = tools()
            .as_array()
            .unwrap()
            .iter()
            .map(|tool| tool["name"].as_str().unwrap().to_string())
            .collect();
        assert_eq!(names, vec!["note_progress", "report_pull_request"]);
    }

    #[tokio::test]
    async fn both_protocol_eras_are_answered_and_a_notification_is_not() {
        let (job, _heard) = a_job(Gate::Open);

        let modern = rpc(
            &job,
            &serde_json::json!({ "jsonrpc": "2.0", "id": 1, "method": "server/discover" }),
        )
        .await
        .unwrap();
        assert_eq!(modern["result"]["protocolVersion"], crate::mcp::PROTOCOL_VERSION);

        // The handshake era answers with what was asked for, which is how a
        // client learns its own request was understood.
        let legacy = rpc(
            &job,
            &serde_json::json!({
                "jsonrpc": "2.0", "id": 0, "method": "initialize",
                "params": { "protocolVersion": "2025-11-25" },
            }),
        )
        .await
        .unwrap();
        assert_eq!(legacy["result"]["protocolVersion"], "2025-11-25");

        // No id is a notification, and answering one with a body is a protocol
        // error the client is entitled to complain about.
        assert!(rpc(
            &job,
            &serde_json::json!({ "jsonrpc": "2.0", "method": "notifications/initialized" })
        )
        .await
        .is_none());

        let unknown = rpc(
            &job,
            &serde_json::json!({ "jsonrpc": "2.0", "id": 9, "method": "resources/list" }),
        )
        .await
        .unwrap();
        assert_eq!(unknown["error"]["code"], -32601);
    }

    // ---- the whole thing, over a socket ----------------------------------

    #[tokio::test]
    async fn a_job_ending_takes_its_mailbox_and_its_scratch_with_it() {
        let bridge = Bridge::new();
        let (signals, _heard) = mpsc::channel(8);
        let session = bridge
            .open(signals, Gate::Open, PathBuf::from("/nonexistent-work-tree"))
            .await
            .expect("the bridge has to start");
        let token = session.session_id().to_string();

        assert!(bridge.post(&token, "a correction"));
        let dir = session.wiring().settings.parent().unwrap().to_path_buf();
        assert!(dir.exists());
        // What the operator hands to `claude --resume`, which is only the same
        // work if Guaca chose it rather than reading it back.
        assert_eq!(session.wiring().session_id, token);

        drop(session);
        assert!(!bridge.post(&token, "too late"), "the mailbox goes with the job");
        assert!(!dir.exists(), "and so does the token that reached it");
    }

    #[tokio::test]
    async fn the_server_answers_a_hook_and_refuses_a_token_it_is_not_holding() {
        let bridge = Bridge::new();
        let (signals, _heard) = mpsc::channel(8);
        let session = bridge
            .open(signals, Gate::Open, PathBuf::from("/nonexistent-work-tree"))
            .await
            .unwrap();
        let port = *bridge.inner.port.get().unwrap();
        let token = session.session_id().to_string();
        bridge.post(&token, "switch to the other library");

        let post = |path: String, body: &'static str| async move {
            let mut client = TcpStream::connect(("127.0.0.1", port)).await.unwrap();
            let request = format!(
                "POST /{path} HTTP/1.1\r\nhost: x\r\ncontent-length: {}\r\n\r\n{body}",
                body.len()
            );
            client.write_all(request.as_bytes()).await.unwrap();
            let mut said = String::new();
            client.read_to_string(&mut said).await.unwrap();
            said
        };

        let delivered = post(format!("{token}/hook"), r#"{"hook_event_name":"PostToolUse"}"#).await;
        assert!(delivered.contains("200 OK"), "{delivered}");
        assert!(delivered.contains("switch to the other library"), "{delivered}");

        // An unknown token is a job that ended, which is not an error worth a
        // body: the hook fails open on an empty answer.
        let gone = post(
            "f96abc0a-5404-4e51-b465-b96677118cf9/hook".to_string(),
            r#"{"hook_event_name":"Stop"}"#,
        )
        .await;
        assert!(gone.contains("404"), "{gone}");
    }

    #[tokio::test]
    async fn one_port_serves_every_job_and_each_reaches_only_its_own() {
        let bridge = Bridge::new();
        let (one, _a) = mpsc::channel(8);
        let (two, _b) = mpsc::channel(8);
        let elsewhere = || PathBuf::from("/nonexistent-work-tree");
        let first = bridge.open(one, Gate::Open, elsewhere()).await.unwrap();
        let second = bridge.open(two, Gate::AskBeforePushing, elsewhere()).await.unwrap();

        assert_ne!(first.session_id(), second.session_id());
        bridge.post(first.session_id(), "for the first one only");
        assert_eq!(bridge.pending(first.session_id()), 1);
        assert_eq!(bridge.pending(second.session_id()), 0);

        // Two jobs, two scratch directories, one socket.
        assert_ne!(first.wiring().settings, second.wiring().settings);
    }
}
