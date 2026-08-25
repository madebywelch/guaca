//! Running a coding harness against a linked repository.
//!
//! Guaca does not write code. It starts something that does, in a directory the
//! operator linked, and reads what comes back. There are two of those things,
//! `pi` and Claude Code, and the operator installs and signs in to whichever of
//! them they use.
//!
//! ## Why a harness and not tools
//!
//! A turn and a coding task are different units of work, and that is the whole
//! argument. A guaca turn is one model call plus `max_tool_rounds` rounds,
//! twenty-four by default, inside a conversation bounded at sixty model calls.
//! A real change to a repository is a few hundred tool calls, its own context
//! window and its own compaction. Reaching that with `read` and `edit` tools in
//! this runtime means raising both limits to coding scale, and both are per
//! group, so the guard that keeps a crew of eight from talking forever comes
//! off for every agent in every crew.
//!
//! So the harness keeps its own loop, its own context and its own budget, and
//! Guaca spends one tool round starting it.
//!
//! ## Why there are two, and why they are not one with a setting
//!
//! Because a subscription is spent by the program it was issued to. The
//! argument is in [`Harness`], and it is the reason this module is a dispatch
//! rather than a provider flag on a single command line.
//!
//! What they share is the shape of a job: one process, in one directory, whose
//! stdout is a stream of JSON objects, one per line, that ends. So there is one
//! process lifecycle here, with two of everything that genuinely differs, which
//! is the argument vector and the fold from an event to an [`Outcome`]. The two
//! submodules are those two things and nothing else.
//!
//! ## Why the credentials are not ours
//!
//! Both harnesses read their own auth: `pi` from `~/.pi/agent/auth.json` or the
//! environment, Claude Code from its own sign-in. Each is already signed in or
//! it is not, and Guaca passing a key would put the operator's Guaca key on a
//! second bill under a second provider for work they are already paying for.
//! The consequence is stated rather than hidden: a job's spend does not appear
//! in this app's usage table, because this app did not spend it. What the job
//! reports back is what the harness says it cost.
//!
//! ## What is not here
//!
//! Any confinement. The process runs as the operator, in their repository, with
//! their credentials and their network, and it may commit, push and open pull
//! requests. That was asked for explicitly. Neither harness is asked to prompt
//! for permission, because there is nobody there to answer: `pi` has no
//! permission system of its own and says so in its own documentation, and
//! Claude Code is started in the mode that does not ask. So the boundary is the
//! directory the operator chose and the fact that git can undo what happens
//! inside it. Nothing in this file should ever be described as a sandbox.

pub mod claude_code;
pub mod pi;

use tokio::io::{AsyncBufReadExt, BufReader};

use crate::domain::repository::Harness;

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
         with `{install}`, or choose the other harness in the repository's settings, then try \
         again"
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
/// A function pointer rather than a trait, because a harness is exactly two
/// functions and a trait for two would be a vocabulary nobody needs. The
/// watcher is `dyn` so the type is nameable.
type Fold = fn(&mut Outcome, &serde_json::Value, &mut dyn FnMut(Progress));

/// The program, by name. Found on `PATH` rather than configured: an operator
/// who has one of these has it on their path, and a second place to say where
/// it lives is a second place for that to be wrong.
fn binary(harness: Harness) -> &'static str {
    match harness {
        Harness::Pi => pi::BINARY,
        Harness::Claude => claude_code::BINARY,
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
    }
}

/// Whether a harness is installed at all.
///
/// Asked by the panel that offers the choice, so an operator picking one they
/// do not have is told at the moment they pick rather than forty minutes later
/// inside a job that never started. It is deliberately not a pre-flight in
/// [`run`]: spawning is already the check there, it cannot go stale between the
/// question and the answer, and a second process per job buys nothing.
pub async fn installed(harness: Harness) -> bool {
    tokio::process::Command::new(binary(harness))
        .arg("--version")
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .await
        .map(|code| code.success())
        .unwrap_or(false)
}

/// Runs one task to completion in one repository.
///
/// Neither harness is asked for a session-less run. A session on disk is what
/// lets the operator open the same work in their own terminal (`pi -c`,
/// `claude -c`), which is the difference between a harness the app runs and a
/// black box.
pub async fn run(
    harness: Harness,
    repository: &str,
    task: &str,
    mut watching: impl FnMut(Progress),
) -> Result<Outcome, CodingError> {
    let (args, fold): (Vec<String>, Fold) = match harness {
        Harness::Pi => (pi::argv(task), pi::absorb),
        Harness::Claude => (claude_code::argv(task), claude_code::absorb),
    };

    let mut child = tokio::process::Command::new(binary(harness))
        .current_dir(repository)
        .args(&args)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        // Killed with this handle rather than left behind. A job whose task is
        // dropped mid-await otherwise leaves a coding agent running in somebody
        // else's repository with nothing holding a reference to it.
        .kill_on_drop(true)
        .spawn()
        .map_err(|err| match err.kind() {
            std::io::ErrorKind::NotFound => {
                CodingError::NotInstalled { harness: harness.label(), install: install(harness) }
            }
            _ => CodingError::Start(err.to_string()),
        })?;

    let stdout = child.stdout.take().ok_or_else(|| CodingError::Start("no output".into()))?;
    let stderr = child.stderr.take();
    let mut lines = BufReader::new(stdout).lines();
    let mut outcome = Outcome::default();

    let reading = async {
        // Split on `\n` and nothing else: pi's own protocol note, and the
        // reason is that `U+2028` and `U+2029` are legal inside JSON strings.
        // Rust's `lines` is compliant where several line readers are not.
        while let Ok(Some(line)) = lines.next_line().await {
            let Ok(event) = serde_json::from_str::<serde_json::Value>(&line) else {
                continue;
            };
            fold(&mut outcome, &event, &mut watching);
        }
    };

    // Stopping is the caller aborting the task this runs in, which drops the
    // child, which `kill_on_drop` turns into a killed process. A cancellation
    // token beside that would be a second way to stop one thing, and the two
    // would have to agree about which had happened.
    tokio::select! {
        _ = reading => {}
        _ = tokio::time::sleep(CEILING) => {
            let _ = child.kill().await;
            return Err(CodingError::TooLong(CEILING.as_secs() / 60));
        }
    }

    let status = child.wait().await.map_err(|err| CodingError::Start(err.to_string()))?;
    if !status.success() && outcome.said.trim().is_empty() && outcome.failed.is_none() {
        // Only when there is nothing to report. A harness that answered and
        // then exited non-zero has still done the work, and throwing its answer
        // away over the exit code is how an agent reports a finished change as
        // a failure.
        //
        // A harness that said inside its own stream *why* it failed has also
        // reported something, and it is the more specific of the two. That
        // clause is not defensive: Claude Code exits non-zero on exactly the
        // failure this feature exists for, so without it a spent plan is
        // reported to the operator as `exit 1` with the sentence naming the
        // plan thrown away.
        let mut why = format!("exit {}", status.code().unwrap_or(-1));
        if let Some(stderr) = stderr {
            let mut text = String::new();
            let mut reader = BufReader::new(stderr).lines();
            while let Ok(Some(line)) = reader.next_line().await {
                text.push_str(&line);
                text.push('\n');
                if text.len() > 2_000 {
                    break;
                }
            }
            if !text.trim().is_empty() {
                why = format!("{why}: {}", text.trim());
            }
        }
        return Err(CodingError::NoAnswer(why));
    }

    Ok(outcome)
}

/// One line of a tool call, cut to fit.
///
/// A heredoc in a shell command is a screen of text that would push everything
/// else out of the panel, and a `write` carries an entire file in it.
pub(crate) fn one_line(raw: &str) -> String {
    let first = raw.trim().lines().next().unwrap_or_default();
    if first.chars().count() > 120 {
        format!("{}…", first.chars().take(120).collect::<String>())
    } else {
        first.to_string()
    }
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
    }
}
