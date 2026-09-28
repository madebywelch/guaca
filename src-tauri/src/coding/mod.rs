//! Running a coding harness in an agent's terminal.
//!
//! Guaca does not write the code for a change that takes a few hundred tool
//! calls. It starts something that does, in a directory inside the agent's own
//! terminal, and reads what comes back. Codex, Claude Code and `pi` each
//! own their model settings and credentials. The operator installs and signs in to whichever of
//! them they use.
//!
//! ## Why a harness and not tools
//!
//! A turn and a coding task are different units of work, and that is the whole
//! argument. A guaca turn is one model call plus `max_tool_rounds` rounds,
//! twenty-four by default, inside a conversation bounded at sixty model calls.
//! A real change to a repository is a few hundred tool calls, its own context
//! window and its own compaction. The terminal's own `read`, `write` and `edit`
//! are for the change a turn can finish; reaching past that with them means
//! raising both limits to coding scale, and both are per group, so the guard
//! that keeps a crew of eight from talking forever comes off for every agent in
//! every crew.
//!
//! So the harness keeps its own loop, its own context and its own budget, and
//! Guaca spends one tool round starting it.
//!
//! ## Why each harness is a program rather than a provider setting
//!
//! Because a subscription is spent by the program it was issued to. The
//! argument is in [`Harness`], and it is the reason this module is a dispatch
//! rather than a provider flag on a single command line.
//!
//! What they share is the shape of a job: one process, in one directory,
//! speaking JSON one object per line, with its stdin open for the life of the
//! job. Claude Code's lifecycle is below, because its stdout is a stream that
//! ends at `result`; `pi`'s RPC mode and Codex's app-server hold a
//! conversation on stdin and each drives its own process in [`pi`] and
//! [`codex`]. All three end by having their input closed, never by a kill
//! unless that fails, and return the same [`Outcome`].
//!
//! ## Whose credentials
//!
//! The harnesses read their own auth: `pi` from `~/.pi/agent/auth.json` or the
//! environment, Codex and Claude Code from their own sign-ins, and a plan is
//! spent by the program it was issued to. The one exception is chosen, never
//! inferred: `pi` set to be paid for with Guaca's own API key reaches it
//! through [`relay`], which holds the key in this process and hands the job a
//! loopback token. Only then does a job's spend appear in this app's usage
//! table, because only then did this app spend it. Otherwise what the job
//! reports back is what the harness says it cost.
//!
//! ## Every job is reachable while it runs
//!
//! `code` returns as soon as the process is up, which is what keeps the agent
//! that asked from reading as `Thinking` for the length of a change to a
//! codebase. The cost of that used to be paid at the other end: for up to
//! [`CEILING`] the job was write-only, and an operator watching one go the
//! wrong way at minute three had nothing to do but wait for it to finish.
//!
//! Each program is reached through the interface its own window uses.
//! Claude Code takes a correction through [`bridge`]'s hooks and a stop as its
//! SDK's `interrupt` on stdin; Codex takes `turn/steer` and `turn/interrupt`;
//! `pi` takes RPC `steer` and `abort`. A push gate is a `PreToolUse` hook, an
//! approval callback and an extension's confirm respectively, and all three
//! are decided by [`bridge::outward`] and the operator's desk. What the
//! operator chose inside each program, the model and the effort, is passed as
//! that program's own flag ([`crate::domain::terminal::Tuning`]), and the
//! models offered are asked of the program ([`models`]).
//!
//! ## What is not here
//!
//! Any confinement. The process runs as the operator, in their repository, with
//! their credentials and their network, and it may commit, push and open pull
//! requests. That was asked for explicitly. Neither harness is asked to prompt
//! for permission on the ordinary path, because there is nobody there to
//! answer: `pi` has no permission system of its own and says so in its own
//! documentation, and Claude Code is started in the mode that does not ask. So
//! the boundary is the directory the operator chose and the fact that git can
//! undo what happens inside it. Nothing in this file should ever be described
//! as a sandbox.
//!
//! [`crate::domain::terminal::Gate::AskBeforePushing`] does not change that
//! sentence and must not be read as changing it. It reads a shell line and decides
//! whether it looks like a push, which is a judgment about the ordinary case: a
//! job that wanted to get around it could, and it was already running as the
//! operator with their network before any of this. What it buys is that the
//! ordinary push, made by a job doing what it was asked, is one somebody gets
//! to see first.

pub mod bridge;
pub mod claude_code;
pub mod codex;
pub mod pi;
pub mod relay;

use tokio::io::{AsyncBufReadExt, AsyncReadExt, BufReader};

use crate::domain::terminal::{Gate, Harness};

pub use bridge::{Bridge, Signal, Wiring};

/// How long a job may run before it is killed.
///
/// Generous, because the unit of work is a change to a repository rather than a
/// reply. What it must not be is unbounded: a harness waiting on a prompt
/// nobody will answer holds a process and a tokio task for the life of the app.
const CEILING: std::time::Duration = std::time::Duration::from_secs(45 * 60);

/// Appended to the harness's own system prompt, on every job, in either of
/// them.
///
/// Only things that are true of every piece of work in every repository. What
/// to change belongs in the brief; how to leave the tree behind belongs here,
/// because it is the same answer every time and an agent writing a brief should
/// not have to remember it.
///
/// Commits are the argument. The job runs unattended for many minutes with no
/// one watching, neither harness is asked to stop and check, and there is no
/// undo but git: a run that works for forty minutes and commits once leaves the
/// operator a single enormous diff and nothing to bisect if it went wrong
/// halfway. Small commits are the only checkpoints this arrangement has.
///
/// Appended rather than replacing: each harness's own coding prompt is the
/// thing that makes it good at this, and Guaca has no business rewriting it.
pub const APPENDED_PROMPT: &str = "\
You are running unattended, started by an agent rather than by a person sitting \
in front of you. Nobody will answer a question, so decide and proceed.

Commit early and often. Every commit is a checkpoint and it is the only undo \
anyone has here: commit as soon as a piece of work stands on its own, before \
starting the next one, rather than saving it all for the end. A run that works \
for forty minutes and commits once leaves a single enormous diff that cannot be \
bisected or partly reverted. Prefer many small commits with real messages, each \
one leaving the tree in a state that builds.

Say what you actually did at the end, including what you could not do. Your \
last message is the only thing the agent that asked for this will read.";

#[derive(Debug, thiserror::Error)]
pub enum CodingError {
    #[error(
        "the {harness} coding harness is not installed, or is not on this app's PATH. Install it \
         with `{install}`, or choose the other harness in the agent's terminal settings, then \
         try again"
    )]
    NotInstalled { harness: &'static str, install: &'static str },
    #[error("the coding harness could not be started: {0}")]
    Start(String),
    #[error(
        "the coding harness exited without answering ({0}). Its own output is above; run it in \
         this repository yourself to see what it says"
    )]
    NoAnswer(String),
    #[error("the job ran for {0} minutes without finishing and was stopped")]
    TooLong(u64),
    #[error(
        "the {harness} on this machine is older than {}.{}, which is the version Guaca can \
         steer and stop while it works. Update it with `{install}`, or choose another harness in \
         the agent's terminal settings",
        .needs.0,
        .needs.1
    )]
    TooOld { harness: &'static str, needs: (u32, u32), install: &'static str },
    /// A program asked for its models that did not list them. Nothing was
    /// started; the model field still takes a name typed by hand.
    #[error("{0}. The model field still takes a name typed by hand")]
    NoModels(String),
}

/// What one job did, as the agent that started it is told.
#[derive(Debug, Clone, Default, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Outcome {
    /// The harness's own last word. What it says it did.
    pub said: String,
    /// How many tools it ran. The one number that says whether it worked in the
    /// repository or just talked about it.
    pub tool_calls: u32,
    /// What the harness reports the job cost, on the operator's own credentials.
    /// `None` when the harness does not price the call, which is not the same as
    /// free and must not be added up as zero.
    pub cost: Option<f64>,
    /// The model the harness chose. Not Guaca's to pick: each harness resolves
    /// it from its own settings and its own sign-ins.
    pub model: String,
    /// What the harness said went wrong, when it ended a turn on an error.
    ///
    /// Both harnesses report a failed turn *inside* their stream and can still
    /// exit zero about it, with no content and no answer. Read by exit code and
    /// text alone, that is indistinguishable from a job with nothing to do.
    ///
    /// It cost an afternoon to find. An expired Codex token turned every coding
    /// job in a live workspace into a silent no-op, the agents reported that
    /// nothing needed doing, and `pi auth check` called the provider ready
    /// throughout.
    pub failed: Option<String>,
    /// The harness session this job ran as.
    ///
    /// What a follow-up continues, and what the operator hands to `claude
    /// --resume`, `codex resume` or `pi --session` to open the same work in
    /// their own terminal, which is not the same thing as `-c`: that resumes
    /// whatever ran last in the directory, and after two jobs that is the
    /// wrong one.
    pub session_id: String,
    /// Whether the operator or the agent stopped it before it finished.
    ///
    /// Not a failure: the harness ended its turn because it was asked to, the
    /// session is on disk, and whatever it committed is real. The agent is told
    /// the work is partly done, which is a different sentence from both "it
    /// finished" and "it could not".
    pub stopped: bool,
    /// A pull request the job opened and said so about.
    ///
    /// Filled in by the runtime from [`Signal::PullRequest`] rather than by the
    /// fold, because it does not come from the harness's stdout at all: it is
    /// the job deliberately calling a tool. Recorded here so everything one job
    /// produced arrives at `job_finished` as one value.
    pub pull_request: Option<PullRequest>,
}

/// A pull request a job opened, as it reported it.
#[derive(Debug, Clone, Default, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PullRequest {
    pub url: String,
    pub branch: String,
}

/// One line of progress, for whoever is watching the job run.
///
/// Deliberately not the raw event. The stream is tens of thousands of lines of
/// deltas and cumulative usage, and forwarding it into a channel would be a
/// firehose nobody reads at the cost of a re-render per token. This is the
/// shape a person watching over the shoulder would want: what it is doing, and
/// what it says as it goes.
#[derive(Debug, Clone, PartialEq)]
pub enum Progress {
    /// A tool the harness started, with the one argument worth reading.
    Using { tool: String, detail: String },
    /// Something the harness said on its way through.
    Said(String),
}

/// Folds one event from a harness's stream into the outcome.
///
/// Pi and Claude share this reader shape. Codex handles requests and replies
/// as well as events in its own driver. The watcher is `dyn` so the type is
/// nameable.
type Fold = fn(&mut Outcome, &serde_json::Value, &mut dyn FnMut(Progress));

/// The program, by name. Found on `PATH` rather than configured: an operator
/// who has one of these has it on their path, and a second place to say where
/// it lives is a second place for that to be wrong.
fn binary(harness: Harness) -> &'static str {
    match harness {
        Harness::Pi => pi::BINARY,
        Harness::Claude => claude_code::BINARY,
        Harness::Codex => codex::BINARY,
    }
}

/// How to get it, for the operator who does not have it.
///
/// One string, spent twice: in the refusal a job gives an agent, and in the
/// panel that offers the choice. A second copy in the webview would be the same
/// operational fact in two languages, drifting the day a vendor renames a
/// package, and only one of the two is the one a failing job quotes.
pub fn install(harness: Harness) -> &'static str {
    match harness {
        Harness::Pi => pi::INSTALL,
        Harness::Claude => claude_code::INSTALL,
        Harness::Codex => codex::INSTALL,
    }
}

/// The oldest Claude Code this app will wire a [`bridge`] into.
///
/// Not a floor on running a job. A job on an older program runs exactly as it
/// did before the bridge existed, which is the only honest thing to do with a
/// version nothing here has ever been measured against: refusing it would take
/// away a harness that works to protect a feature that is an addition to it.
///
/// The number is the line the contract was measured on. Every part of the
/// bridge is a promise about how the program behaves rather than about a flag
/// it accepts, and those cannot be checked offline: that a `PreToolUse` hook's
/// `deny` overrides `--permission-mode bypassPermissions`, that a `Stop` hook's
/// `reason` reaches the model, that `additionalContext` from `PostToolUse` is
/// put in front of it. All three were verified against 2.1.247, and the live
/// half of `tests/coding.rs` is what catches them moving.
const BRIDGE_FLOOR: (u32, u32) = (2, 1);

/// What this machine has of one harness.
#[derive(Debug, Clone, PartialEq)]
pub enum Presence {
    /// Not on this app's `PATH`.
    Missing,
    /// There, and what it says its version is.
    ///
    /// The string is the program's own answer rather than a parse of it, so a
    /// panel shows what `--version` prints and an operator comparing it with
    /// their terminal sees the same thing.
    Installed { version: String, bridged: bool },
}

impl Presence {
    pub fn installed(&self) -> bool {
        matches!(self, Presence::Installed { .. })
    }
}

/// What this machine has of a harness, and whether a job on it gets a bridge.
///
/// Asked by the panel that offers the choice, so an operator picking one they
/// do not have is told at the moment they pick rather than forty minutes later
/// inside a job that never started. Asked again by [`crate::runtime`] when a
/// job starts, which is one process spawn against a job that runs for minutes
/// and is the only way to know whether the bridge is worth wiring: an operator
/// upgrades between the panel and the job.
///
/// It is still not a pre-flight on whether the job can run. Spawning is the
/// check for that, it cannot go stale between the question and the answer, and
/// a refusal built on this one would refuse jobs that work.
pub async fn presence(harness: Harness) -> Presence {
    let mut command = tokio::process::Command::new(binary(harness));
    command
        .arg("--version")
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::null())
        .kill_on_drop(true);
    let Ok(Ok(asked)) =
        tokio::time::timeout(std::time::Duration::from_secs(5), command.output()).await
    else {
        return Presence::Missing;
    };
    if !asked.status.success() {
        return Presence::Missing;
    }

    let version = String::from_utf8_lossy(&asked.stdout).trim().to_string();
    let bridged = at_least(&version, floor(harness));
    Presence::Installed { version, bridged }
}

/// The oldest version of each program whose interface this build was measured
/// against.
fn floor(harness: Harness) -> (u32, u32) {
    match harness {
        Harness::Claude => BRIDGE_FLOOR,
        Harness::Codex => (0, 153),
        Harness::Pi => pi::RPC_FLOOR,
    }
}

/// The refusal for a program too old to be driven at all, with the floor it
/// is under spelled from the same constant [`presence`] compares against.
pub fn too_old(harness: Harness) -> CodingError {
    CodingError::TooOld {
        harness: harness.label(),
        needs: floor(harness),
        install: install(harness),
    }
}

/// Commands are run by the operator on the backend, under the daemon's user.
/// Guaca never opens a consumer OAuth flow or reads a CLI credential file.
pub fn sign_in(harness: Harness) -> &'static str {
    match harness {
        Harness::Codex => "codex login --device-auth",
        Harness::Claude => "claude auth login",
        Harness::Pi => "pi",
    }
}

/// A CLI's own local status check, never a model call. Only the boolean crosses
/// IPC: even status output may contain an account name or an API key prefix.
pub async fn signed_in(harness: Harness) -> Option<bool> {
    let args: &[&str] = match harness {
        Harness::Codex => &["login", "status"],
        Harness::Claude => &["auth", "status", "--json"],
        Harness::Pi => return None,
    };
    let mut command = tokio::process::Command::new(binary(harness));
    command.args(args).kill_on_drop(true).stderr(std::process::Stdio::null());
    let asked = tokio::time::timeout(std::time::Duration::from_secs(5), command.output())
        .await
        .ok()?
        .ok()?;
    if harness == Harness::Claude {
        serde_json::from_slice::<serde_json::Value>(&asked.stdout).ok()?["loggedIn"].as_bool()
    } else {
        Some(asked.status.success())
    }
}

/// Whether a `--version` line names a release at or past a floor.
///
/// The programs spell it differently and both spellings are moving targets:
/// `2.1.247 (Claude Code)` today, a bare number yesterday. So the first
/// dotted number in the line is what is read, and a line with none in it is
/// treated as too old rather than as new enough. That direction matters: an
/// unreadable version turns the bridge off and leaves a working job, where the
/// other way round would wire a job to a contract nothing has checked.
fn at_least(version: &str, floor: (u32, u32)) -> bool {
    let digits = |word: &str| {
        word.split('.').all(|part| !part.is_empty() && part.bytes().all(|b| b.is_ascii_digit()))
    };
    let Some(found) = version.split_whitespace().find(|word| word.contains('.') && digits(word))
    else {
        return false;
    };
    let mut parts = found.split('.').map(|part| part.parse::<u32>().unwrap_or(0));
    let (major, minor) = (parts.next().unwrap_or(0), parts.next().unwrap_or(0));
    (major, minor) >= floor
}

/// One model a harness offers, as its own model picker shows it.
///
/// Asked of the program rather than listed here, because the list is the
/// program's and moves with its releases and its sign-in: Claude Code's
/// `initialize` answer, Codex's `model/list`, `pi`'s `get_available_models`.
#[derive(Debug, Clone, PartialEq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ModelOffer {
    /// What `--model` (or Codex's `model`) takes.
    pub id: String,
    /// What the program calls it.
    pub label: String,
    /// The program's own line about it, or which provider serves it.
    pub detail: String,
    /// Whether it is what the program runs when nothing is chosen.
    pub default: bool,
    /// The effort words this model takes. Empty when it takes none.
    pub efforts: Vec<String>,
}

/// How long a program has to answer a question about itself.
const ASKING: std::time::Duration = std::time::Duration::from_secs(20);

/// The models a harness offers on this machine, asked of the program.
///
/// `lent` is for `pi` on Guaca's key: the listing loads the same provider
/// override a job would, so what is offered is what that job can run. Against
/// an endpoint other than OpenRouter there is no catalog to ask, and the list
/// is empty rather than pi's own sign-ins, which would offer models the job
/// cannot reach.
pub async fn models(
    harness: Harness,
    lent: Option<&relay::Upstream>,
) -> Result<Vec<ModelOffer>, CodingError> {
    match harness {
        Harness::Claude => {
            let args = [
                "-p",
                "--input-format",
                "stream-json",
                "--output-format",
                "stream-json",
                "--verbose",
            ];
            let asked = [claude_code::initialize()];
            let answer = ask(harness, &args, &asked, claude_code::initialized).await?;
            Ok(claude_code::offers(&answer))
        }
        Harness::Codex => {
            let asked = codex::listing();
            let answer =
                ask(harness, &["app-server", "--listen", "stdio://"], &asked, codex::listed)
                    .await?;
            Ok(codex::offers(&answer))
        }
        Harness::Pi => {
            if lent.is_some_and(|upstream| !upstream.is_openrouter()) {
                return Ok(Vec::new());
            }
            let mut scratch =
                pi::Scratch::new().map_err(|err| CodingError::Start(err.to_string()))?;
            let mut args = vec!["--mode".to_string(), "rpc".into(), "--no-session".into()];
            if lent.is_some() {
                // Never called: the address is a port nothing listens on, and a
                // listing makes no model call. It only has to make pi count the
                // provider as signed in.
                let source = pi::provider_source("http://127.0.0.1:9/v1", "listing", true, "");
                scratch
                    .add("provider.ts", &source)
                    .map_err(|err| CodingError::Start(err.to_string()))?;
                for extension in scratch.extensions() {
                    args.push("-e".into());
                    args.push(extension.to_string_lossy().into_owned());
                }
            }
            let args: Vec<&str> = args.iter().map(String::as_str).collect();
            let answer = ask(harness, &args, &[pi::listing()], pi::listed).await?;
            Ok(pi::offers(&answer, lent.is_some()))
        }
    }
}

/// Starts a program, says a few lines to it, and returns the first line of
/// its output `answered` accepts. The process is ended either way.
async fn ask(
    harness: Harness,
    args: &[&str],
    lines: &[serde_json::Value],
    answered: fn(&serde_json::Value) -> bool,
) -> Result<serde_json::Value, CodingError> {
    // A directory of its own, so a program that writes where it starts leaves
    // nothing behind, and two listings never share one.
    let here = std::env::temp_dir().join(format!("guaca-models-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(&here).map_err(|err| CodingError::Start(err.to_string()))?;
    let answer = ask_in(&here, harness, args, lines, answered).await;
    let _ = std::fs::remove_dir_all(&here);
    answer
}

async fn ask_in(
    here: &std::path::Path,
    harness: Harness,
    args: &[&str],
    lines: &[serde_json::Value],
    answered: fn(&serde_json::Value) -> bool,
) -> Result<serde_json::Value, CodingError> {
    use tokio::io::AsyncWriteExt;

    let mut child = tokio::process::Command::new(binary(harness))
        .args(args)
        .current_dir(here)
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::null())
        .kill_on_drop(true)
        .spawn()
        .map_err(|err| match err.kind() {
            std::io::ErrorKind::NotFound => {
                CodingError::NotInstalled { harness: harness.label(), install: install(harness) }
            }
            _ => CodingError::Start(err.to_string()),
        })?;
    let mut stdin = child.stdin.take().ok_or_else(|| CodingError::Start("no input".into()))?;
    let stdout = child.stdout.take().ok_or_else(|| CodingError::Start("no output".into()))?;
    for line in lines {
        let text = format!("{line}\n");
        stdin
            .write_all(text.as_bytes())
            .await
            .map_err(|_| CodingError::NoModels(format!("{} closed its input", harness.label())))?;
    }
    let mut output = BufReader::new(stdout).lines();
    let reading = async {
        while let Ok(Some(line)) = output.next_line().await {
            if let Ok(value) = serde_json::from_str::<serde_json::Value>(&line) {
                if answered(&value) {
                    return Some(value);
                }
            }
        }
        None
    };
    let found = tokio::time::timeout(ASKING, reading).await;
    close(&mut child, stdin).await;
    match found {
        Ok(Some(value)) => Ok(value),
        Ok(None) => Err(CodingError::NoModels(format!(
            "{} exited without listing its models. Run it on the backend to see why",
            harness.label()
        ))),
        Err(_) => Err(CodingError::NoModels(format!(
            "{} did not list its models within {} seconds",
            harness.label(),
            ASKING.as_secs()
        ))),
    }
}

/// Ends a program by closing its input, and kills it only if that fails.
///
/// All three exit on their own when stdin closes, in milliseconds. A kill is
/// not the same thing to them: a `pi` that is killed leaves the next `pi` to
/// start waiting about thirty seconds before it answers anything, measured
/// against 0.84.4, and a model listing that killed pi made every other panel
/// open half a minute late.
async fn close(child: &mut tokio::process::Child, stdin: tokio::process::ChildStdin) {
    drop(stdin);
    if tokio::time::timeout(std::time::Duration::from_secs(5), child.wait()).await.is_err() {
        let _ = child.kill().await;
        let _ = child.wait().await;
    }
}

/// One piece of work handed to a harness.
///
/// Everything the three drivers need, gathered once by the runtime so none of
/// them reads the store. `session` is the harness's own name for the
/// conversation: `claude` and `pi` are started with one Guaca chose, which is
/// what makes a follow-up and `claude --resume` find it; Codex names its thread
/// itself, so a new Codex job carries an empty one and reports the thread with
/// [`Signal::Session`].
pub struct Job<'a> {
    pub harness: Harness,
    /// Absolute, and inside the agent's terminal.
    pub directory: &'a str,
    pub brief: &'a str,
    pub session: &'a str,
    /// Whether `session` already exists and this job carries it on, with
    /// everything it had already read and decided, rather than starting over.
    pub resume: bool,
    pub gate: Gate,
    pub env: &'a crate::secrets::Environment,
    /// Claude Code's end of the [`bridge`], when this job has one. The other two
    /// are reached through their own protocols and never have one.
    pub bridge: Option<&'a bridge::Session>,
    /// The model and effort the operator chose for this agent in this harness,
    /// passed as the program's own flags. Default is the program's setting.
    pub tuning: &'a crate::domain::terminal::Tuning,
    /// Guaca's key, lent to a `pi` job through the [`relay`], when the
    /// operator chose [`crate::domain::terminal::Payer::GuacaKey`] for it.
    pub lent: Option<&'a relay::Lease>,
}

/// What the runtime can ask of a job while it runs.
///
/// The same two things an operator can do in each program's own window, and
/// every harness answers both: a message that reaches the model at its next
/// step, and a stop that ends the turn and keeps the session.
pub enum Control {
    /// Delivered at the job's next tool boundary. `reply` says whether the
    /// harness took it, which for Codex is its own acknowledgment.
    Steer { message: String, reply: tokio::sync::oneshot::Sender<Result<(), String>> },
    /// End the current turn. The session stays on disk, so the work can be
    /// continued from where it stopped.
    Stop,
}

/// How long a stopped job is given to end its turn before it is killed.
///
/// Long enough for a harness to write the interrupted turn into its session,
/// which is what makes a stopped job one a follow-up can continue, and short
/// enough that a harness ignoring the request does not hold the agent's one
/// job slot for the length of a test run.
const STOP_GRACE: std::time::Duration = std::time::Duration::from_secs(10);

/// What a Claude job with no bridge says to a correction.
const CLAUDE_UNREACHABLE: &str = "this Claude Code job is running without a bridge, so nothing \
     can reach it until it finishes. Claude Code has to be new enough for one";

/// Runs one job to its end, answering controls as it goes.
///
/// Returns when the harness has finished its turn, however that happened: an
/// answer, a failure it reported, a stop, or the [`CEILING`]. Whatever the
/// outcome carries has had every granted secret's value taken out of it.
pub async fn run(
    job: Job<'_>,
    controls: tokio::sync::mpsc::Receiver<Control>,
    signals: tokio::sync::mpsc::Sender<Signal>,
    mut watching: impl FnMut(Progress) + Send,
) -> Result<Outcome, CodingError> {
    let brief = format!("{}{}", job.brief, job.env.description());
    let job = Job { brief: &brief, ..job };
    let values = job.env.values.clone();
    let mut sanitized = |progress| {
        watching(match progress {
            Progress::Using { tool, detail } => Progress::Using {
                tool: crate::secrets::redact(&tool, &values),
                detail: crate::secrets::redact(&detail, &values),
            },
            Progress::Said(said) => Progress::Said(crate::secrets::redact(&said, &values)),
        })
    };
    let result = match job.harness {
        Harness::Codex => codex::run(&job, controls, signals, &mut sanitized).await,
        Harness::Pi => pi::run(&job, controls, signals, &mut sanitized).await,
        Harness::Claude => run_stream(&job, controls, &mut sanitized).await,
    };
    let values = &job.env.values;
    result
        .map(|mut outcome| {
            outcome.said = crate::secrets::redact(&outcome.said, values);
            outcome.failed = outcome.failed.map(|text| crate::secrets::redact(&text, values));
            outcome.model = crate::secrets::redact(&outcome.model, values);
            outcome.session_id = crate::secrets::redact(&outcome.session_id, values);
            if let Some(pr) = &mut outcome.pull_request {
                pr.url = crate::secrets::redact(&pr.url, values);
                pr.branch = crate::secrets::redact(&pr.branch, values);
            }
            outcome
        })
        .map_err(|error| match error {
            CodingError::NoAnswer(text) => {
                CodingError::NoAnswer(crate::secrets::redact(&text, values))
            }
            CodingError::Start(text) => CodingError::Start(crate::secrets::redact(&text, values)),
            other => other,
        })
}

/// Claude Code: one process, its brief and its stop on stdin, and a stream of
/// events on stdout that ends at `result`.
///
/// A correction is posted to the job's mailbox on the bridge and read by a
/// hook, which is measured and is the one path an older program without a
/// bridge refuses. A stop is the SDK's own `interrupt` on stdin, which ends the
/// turn and leaves the session resumable; the process is killed only if it has
/// not ended the turn within [`STOP_GRACE`].
async fn run_stream(
    job: &Job<'_>,
    mut controls: tokio::sync::mpsc::Receiver<Control>,
    mut watching: impl FnMut(Progress) + Send,
) -> Result<Outcome, CodingError> {
    use tokio::io::AsyncWriteExt;

    let harness = job.harness;
    let args =
        claude_code::argv(job.session, job.resume, job.bridge.map(|b| b.wiring()), job.tuning);
    let fold: Fold = claude_code::absorb;
    let directory = job.directory;

    let mut command = tokio::process::Command::new(binary(harness));
    job.env.apply(&mut command);
    let mut child = command
        .current_dir(directory)
        .args(&args)
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        // Killed with this handle rather than left behind. A job whose task is
        // dropped mid-await otherwise leaves a coding agent running with
        // nothing holding a reference to it.
        .kill_on_drop(true)
        .spawn()
        .map_err(|err| match err.kind() {
            std::io::ErrorKind::NotFound => {
                CodingError::NotInstalled { harness: harness.label(), install: install(harness) }
            }
            _ => CodingError::Start(err.to_string()),
        })?;

    let stdout = child.stdout.take().ok_or_else(|| CodingError::Start("no output".into()))?;
    let stderr = child.stderr.take().ok_or_else(|| CodingError::Start("no stderr".into()))?;
    let mut stdin = child.stdin.take();
    let mut lines = BufReader::new(stdout).lines();
    // The session is set from what was asked for rather than read back off the
    // stream, which is the point of choosing it: a job killed at the ceiling
    // still has a session the operator can open, and a job that died before its
    // first event still says which one it was.
    let mut outcome = Outcome { session_id: job.session.to_string(), ..Outcome::default() };

    // Every tool call the panel draws goes through here rather than through the
    // parser, because this is the level that knows where the job is standing
    // and a `cd` is only redundant against that.
    let mut drawing = |progress: Progress| {
        watching(match progress {
            Progress::Using { tool, detail } => {
                Progress::Using { tool, detail: shown(directory, &detail) }
            }
            said => said,
        });
    };

    let draining = tokio::spawn(drain_stderr(stderr));
    // A program that cannot take its brief has already exited, and what it
    // said about why is on stderr, which the exit handling below reports.
    if let Some(pipe) = stdin.as_mut() {
        let line = format!("{}\n", claude_code::prompt(job.brief));
        if pipe.write_all(line.as_bytes()).await.is_err() {
            stdin = None;
        }
    }

    // When the stop was asked for, so the grace can run out; and whether the
    // turn had already ended by then, in which case there was nothing to stop.
    let mut stopping: Option<tokio::time::Instant> = None;
    let mut ended = false;
    let reading = async {
        loop {
            let grace = async {
                match stopping {
                    Some(at) => tokio::time::sleep_until(at).await,
                    None => std::future::pending().await,
                }
            };
            tokio::select! {
                line = lines.next_line() => {
                    // Split on `\n` and nothing else: `U+2028` and `U+2029` are
                    // legal inside JSON strings, and Rust's `lines` is compliant
                    // where several line readers are not.
                    let Ok(Some(line)) = line else { break };
                    let Ok(event) = serde_json::from_str::<serde_json::Value>(&line) else {
                        continue;
                    };
                    fold(&mut outcome, &event, &mut drawing);
                    // In this mode the program waits for another message after
                    // its answer. There is none, and closing stdin is how it is
                    // told so.
                    if event["type"] == "result" {
                        ended = true;
                        stdin = None;
                    }
                }
                Some(control) = controls.recv() => match control {
                    Control::Steer { message, reply } => {
                        let answer = match job.bridge {
                            Some(session) if session.post(&message) => Ok(()),
                            Some(_) => Err("the job has already finished".to_string()),
                            None => Err(CLAUDE_UNREACHABLE.to_string()),
                        };
                        let _ = reply.send(answer);
                    }
                    Control::Stop if stopping.is_none() && !ended => {
                        stopping = Some(tokio::time::Instant::now() + STOP_GRACE);
                        let sent = match stdin.as_mut() {
                            Some(pipe) => {
                                let line = format!("{}\n", claude_code::interrupt());
                                pipe.write_all(line.as_bytes()).await.is_ok()
                            }
                            None => false,
                        };
                        // Nobody to ask: the grace is spent at once.
                        if !sent {
                            break;
                        }
                    }
                    Control::Stop => {}
                },
                _ = grace => break,
            }
        }
    };
    if tokio::time::timeout(CEILING, reading).await.is_err() {
        let _ = child.kill().await;
        return Err(CodingError::TooLong(CEILING.as_secs() / 60));
    }
    if stopping.is_some() {
        // The interrupt ends the turn as `error_during_execution`, which is the
        // program's word for a turn that did not finish and not a failure: it
        // was asked to stop, and did. Given its own exit if it answered, and
        // killed if the grace ran out.
        // Its input was closed at `result`, so it is already on its way out.
        drop(stdin);
        let exited = ended
            && tokio::time::timeout(std::time::Duration::from_secs(5), child.wait()).await.is_ok();
        if !exited {
            let _ = child.kill().await;
            let _ = child.wait().await;
        }
        outcome.stopped = true;
        outcome.failed = None;
        return Ok(outcome);
    }
    drop(stdin);
    let status = child.wait().await.map_err(|err| CodingError::Start(err.to_string()))?;
    let stderr_text = draining.await.unwrap_or_default();
    if !status.success() && outcome.said.trim().is_empty() && outcome.failed.is_none() {
        // Only when there is nothing to report. A harness that answered and
        // then exited non-zero has still done the work, and throwing its answer
        // away over the exit code is how an agent reports a finished change as
        // a failure.
        //
        // A harness that said inside its own stream *why* it failed has also
        // reported something, and it is the more specific of the two. Claude
        // Code exits non-zero on exactly the failure this feature exists for,
        // so without that clause a spent plan is reported as `exit 1` with the
        // sentence naming the plan thrown away.
        let mut why = format!("exit {}", status.code().unwrap_or(-1));
        if !stderr_text.trim().is_empty() {
            why = format!("{why}: {}", stderr_text.trim());
        }
        return Err(CodingError::NoAnswer(why));
    }

    Ok(outcome)
}

/// Drain to EOF while keeping only a diagnostic prefix. Both protocols must
/// read stderr beside stdout or a full pipe can deadlock the coding process.
pub(super) async fn drain_stderr(mut stderr: tokio::process::ChildStderr) -> String {
    let mut kept = Vec::new();
    let mut chunk = [0u8; 4096];
    while let Ok(n) = stderr.read(&mut chunk).await {
        if n == 0 {
            break;
        }
        let take = n.min(2000usize.saturating_sub(kept.len()));
        kept.extend_from_slice(&chunk[..take]);
    }
    String::from_utf8_lossy(&kept).into_owned()
}

/// The first line of something a harness wrote, and nothing after it.
///
/// A heredoc in a shell command is a screen of text that would push everything
/// else out of the panel, and a `write` carries an entire file in it.
pub(crate) fn first_line(raw: &str) -> &str {
    raw.trim().lines().next().unwrap_or_default()
}

/// One line of what a harness said, cut to fit.
///
/// What a hook hands the bridge goes through here on its way to being stored.
/// A tool call does not: it is cut by [`shown`] instead, once the part of it
/// that says nothing has come off, because cutting it here would cut it before
/// anything worth reading had arrived.
pub(crate) fn one_line(raw: &str) -> String {
    clip(first_line(raw))
}

/// How much of a line the panel drawing it can hold.
const DETAIL: usize = 120;

/// Cut to that, saying that it was cut.
fn clip(line: &str) -> String {
    if line.chars().count() > DETAIL {
        format!("{}…", line.chars().take(DETAIL).collect::<String>())
    } else {
        line.to_string()
    }
}

/// What the panel draws for one tool call.
///
/// The command that ran, with the `cd` that only says where it already was
/// taken off the front, cut to fit whatever is left.
fn shown(here: &str, detail: &str) -> String {
    clip(without_cd(here, detail))
}

/// A command with a `cd` back to the directory it is already in taken off it.
///
/// Every harness is started in its directory with `current_dir`, and the brief
/// names that directory by its absolute path, which a model reads as somewhere
/// to go: it writes `cd "/Users/…/terminals/<agent>/<repo>" && pnpm test` in
/// front of every command it runs. A work tree path runs to
/// around 110 characters and a line has [`DETAIL`] of them to spend, so the cut
/// landed inside the path and nine commands drew nine copies of it and none of
/// what ran. The prefix is worth nothing even when it fits.
///
/// Only a `cd` to exactly this directory, and only where something follows it
/// unconditionally. `cd frontend && pnpm test` says where the tests ran, and
/// `cd somewhere || echo no` runs the rest *because* the `cd` failed: both are
/// part of what happened, and a panel that quietly dropped either would be
/// wrong about a command it is drawing as the thing that ran. Anything this
/// cannot read confidently is left exactly as it was written, for the same
/// reason.
fn without_cd<'a>(here: &str, line: &'a str) -> &'a str {
    let mut rest = line.trim_start();
    loop {
        let Some(after) = rest.strip_prefix("cd") else { return rest };
        if !(after.starts_with(' ') || after.starts_with('\t')) {
            return rest;
        }
        let Some((where_to, after)) = one_word(after.trim_start()) else { return rest };
        if !same_place(here, &where_to) {
            return rest;
        }
        let after = after.trim_start();
        let Some(next) = after.strip_prefix("&&").or_else(|| after.strip_prefix(';')) else {
            return rest;
        };
        let next = next.trim_start();
        // A `cd` with nothing after it is the whole of what the harness ran,
        // which is a step like any other and the only thing on the line.
        if next.is_empty() {
            return rest;
        }
        rest = next;
    }
}

/// One shell word off the front, unquoted, and what follows it.
///
/// Unquoted, a word ends at whitespace or at the operator that follows it:
/// `cd /w/tree; git status` has no space in front of the `;`, and a word read
/// up to the space instead is `/w/tree;`, which matches no directory and left
/// the prefix on the line it was written to take off.
///
/// `None` where reading it would be a guess: a quote that never closes, or a
/// space escaped outside quotes, which means the word did not end where the
/// space is. Neither is a shape a model writes a `cd` in, and the caller leaves
/// the line alone rather than cutting it somewhere arbitrary.
fn one_word(text: &str) -> Option<(String, &str)> {
    let quote = text.chars().next()?;
    if quote != '"' && quote != '\'' {
        let ends = |c: char| c.is_whitespace() || c == ';' || c == '&' || c == '|';
        let end = text.find(ends).unwrap_or(text.len());
        if text[..end].ends_with('\\') {
            return None;
        }
        return Some((text[..end].to_string(), &text[end..]));
    }

    let body = &text[quote.len_utf8()..];
    let mut word = String::new();
    let mut chars = body.char_indices();
    while let Some((at, ch)) = chars.next() {
        if ch == quote {
            return Some((word, &body[at + ch.len_utf8()..]));
        }
        // Only inside double quotes: a backslash in a single-quoted string is a
        // backslash, which is what the shell does with it.
        if ch == '\\' && quote == '"' {
            let (_, escaped) = chars.next()?;
            word.push(escaped);
            continue;
        }
        word.push(ch);
    }
    None
}

/// Whether a `cd` names the directory the job is already standing in.
///
/// Compared as written, plus the two spellings of the same place a model
/// actually produces. Nothing is resolved against the filesystem: this runs on
/// every tool call of every job, for a line that is only being drawn, and a
/// symlink answered wrong here leaves a `cd` on the front of a command rather
/// than doing anything worse.
fn same_place(here: &str, where_to: &str) -> bool {
    where_to == "." || where_to.trim_end_matches('/') == here.trim_end_matches('/')
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_appended_prompt_is_about_how_to_work_and_never_about_what_to_build() {
        // Anything here is true of every job in every repository, in either
        // harness. What to change is the brief's, and a sentence in here about
        // it would quietly apply to work it was never written for.
        assert!(APPENDED_PROMPT.contains("Commit early and often"));
        assert!(APPENDED_PROMPT.contains("checkpoint"));
        // Unattended is the fact everything else follows from: nobody will
        // answer, so it decides, and git is the only undo.
        assert!(APPENDED_PROMPT.contains("unattended"));
        assert!(APPENDED_PROMPT.contains("last message"));
    }

    #[test]
    fn a_program_too_old_to_drive_is_told_the_floor_it_is_under_and_how_to_clear_it() {
        let why = too_old(Harness::Pi).to_string();
        assert!(why.contains("older than 0.84"), "{why}");
        assert!(why.contains(pi::INSTALL), "{why}");
        assert!(at_least("0.84.4", floor(Harness::Pi)));
        assert!(!at_least("0.83.9", floor(Harness::Pi)));
    }

    #[test]
    fn every_refusal_says_what_to_do_about_it() {
        // Read by a model mid-turn and by a person under pressure. A refusal
        // that only says no gets reworded and retried.
        for harness in Harness::ALL {
            let missing =
                CodingError::NotInstalled { harness: harness.label(), install: install(harness) }
                    .to_string();
            assert!(missing.contains(harness.label()), "{missing}");
            assert!(missing.contains("npm install"), "{missing}");
            // The other harness is a way out that does not need an install at
            // all, and the operator cannot guess it from a message about PATH.
            assert!(missing.contains("the other harness"), "{missing}");
        }
        assert!(CodingError::TooLong(45).to_string().contains("45 minutes"));
    }

    #[test]
    fn every_harness_has_a_binary_and_a_way_to_install_it() {
        // The two matches in this file are exhaustive by the compiler; this is
        // about neither arm being blank, which the compiler cannot see.
        for harness in Harness::ALL {
            assert!(!binary(harness).is_empty());
            assert!(install(harness).starts_with("npm install"), "{}", install(harness));
        }
    }

    #[test]
    fn a_long_command_is_cut_rather_than_drawn_whole() {
        let long = one_line(&"x".repeat(400));
        assert!(long.chars().count() <= 121, "{}", long.chars().count());
        assert!(long.ends_with('…'));
        // A heredoc is a screen of text and only its first line says what ran.
        assert_eq!(one_line("cat <<'EOF' > a\nline\nEOF"), "cat <<'EOF' > a");

        // A tool call is cut by the other one, which has taken the part that
        // says nothing off the front first.
        let call = shown("/w/tree", &"x".repeat(400));
        assert!(call.chars().count() <= 121, "{}", call.chars().count());
        assert!(call.ends_with('…'));
    }

    #[test]
    fn a_cd_back_into_the_work_tree_is_not_what_the_command_did() {
        // A real bench path. The harness is already standing in it, so a model
        // that read it in the brief and wrote it in front of every command has
        // spent the whole line saying where it was.
        let here = concat!(
            "/Users/robert/Library/Application Support/com.madebywelch.guac",
            "/worktrees/5f2c2995-c0fb-42a2-8a29-49ca803fdae3/b2baae"
        );
        assert!(
            format!("cd \"{here}\" && ").chars().count() > DETAIL,
            "the prefix alone is more than a line holds, which is why the cut landed inside it"
        );

        assert_eq!(shown(here, &format!("cd \"{here}\" && pnpm test")), "pnpm test");
        assert_eq!(shown(here, &format!("cd '{here}' && pnpm build")), "pnpm build");
        // No space in front of the operator, which is where a word read up to
        // the next space swallows the `;` and matches nothing.
        assert_eq!(shown(here, &format!("cd \"{here}\"; git status")), "git status");
        assert_eq!(shown(here, &format!("cd '{here}'&& ls")), "ls");
        assert_eq!(shown(here, &format!("cd \"{here}/\" && ls")), "ls");
        assert_eq!(shown(here, &format!("cd \"{here}\" && cd \"{here}\" && ls")), "ls");
        assert_eq!(shown(here, &format!("cd \"{here}\" && cd . && ls")), "ls");

        // The failure as the operator met it: nine commands in a row, drawn
        // identically, because everything that differed was past the cut.
        assert_ne!(
            shown(here, &format!("cd \"{here}\" && pnpm test")),
            shown(here, &format!("cd \"{here}\" && pnpm build")),
        );
        // And what is left is cut from its own start, not from the path's.
        let long = shown(here, &format!("cd \"{here}\" && {}", "y".repeat(400)));
        assert!(long.starts_with("yyy"), "{long}");
    }

    #[test]
    fn a_cd_that_says_where_the_work_happened_stays_on_the_line() {
        let here = "/w/tree";
        // Somewhere below, and somewhere else: both are part of what ran.
        assert_eq!(shown(here, "cd frontend && pnpm test"), "cd frontend && pnpm test");
        assert_eq!(shown(here, "cd /other && ls"), "cd /other && ls");
        // `||` runs the rest *because* the `cd` failed, which is not a prefix.
        assert_eq!(shown(here, "cd /w/tree || echo no"), "cd /w/tree || echo no");
        // Nothing follows it, so it is the whole of what the harness ran.
        assert_eq!(shown(here, "cd /w/tree"), "cd /w/tree");
        assert_eq!(shown(here, "cdefg /w/tree && ls"), "cdefg /w/tree && ls");
        // Unquoted, and the operator is where the word ends.
        assert_eq!(shown(here, "cd /w/tree;ls"), "ls");
        assert_eq!(shown(here, "cd /w/tree&&ls"), "ls");
        // Unquoted with a space in it is a `cd` to `/w`, whatever was meant.
        assert_eq!(shown("/w/t wo", "cd /w/t wo && ls"), "cd /w/t wo && ls");
        // Quoting this cannot read is left exactly as it was written, rather
        // than cut somewhere guessed at.
        assert_eq!(shown(here, "cd \"/w/tree && ls"), "cd \"/w/tree && ls");
        assert_eq!(shown(here, "cd /w/tree\\ two && ls"), "cd /w/tree\\ two && ls");
    }
}
