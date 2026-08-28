//! One shell line, in a directory on this machine, with two bounds on it.
//!
//! This is the second way into a repository and the small one. [`crate::coding`]
//! is the other: a whole harness, in its own process, on its own budget, for
//! minutes at a time. That is the right unit for a change to a codebase and the
//! wrong unit for every question an agent has about the tree it is standing in.
//! Before this existed there was only the big one, and the shape of the failure
//! was consistent: an agent asked to merge a pull request either spent a coding
//! job on `gh pr merge`, or, when the harness would not start — a spent plan, a
//! program not installed, another job already in the work tree — reported to the
//! operator that it had no shell and no way to reach GitHub. Both are true
//! sentences about a design with one door in it.
//!
//! ## It is not a sandbox, and must never be described as one
//!
//! The line runs as the operator, in their directory, with their credentials
//! and their network. That is the same sentence `docs/CODING.md` writes under
//! *What is not here*, and it is not a widening: a coding job in that directory
//! already ran arbitrary commands as the operator, under
//! `--permission-mode bypassPermissions`, with the operator's own MCP servers
//! loaded. What this adds is directness, not reach. What confines it is what
//! confined that: the directory the operator chose, and the fact that git can
//! undo what happens inside it.
//!
//! The one control that does apply is [`crate::domain::repository::Gate`], and
//! it applies here for the same reason it applies to a job. A push, a merge or
//! a release leaves the work tree under the operator's own name and git cannot
//! take it back. `Runtime::run_in_repository` asks `coding::bridge::outward`
//! about the line before running it, from the same function the `PreToolUse`
//! hook asks, so the two doors give the same answer to the same command. A gate
//! that read only one of them would be a gate an agent could walk around by
//! choosing the other tool.
//!
//! ## The two bounds
//!
//! A job has forty-five minutes because it is its own unit of work. This
//! happens *inside a turn*, so it has [`PATIENCE`], which is what an operator
//! will watch a reply take. And its output is what a model is about to be
//! charged for reading, so both ends are kept and the middle is counted and
//! dropped. Neither bound is confinement and neither is a security control;
//! they are what keeps one call from taking a turn or a context window with it.

use std::collections::VecDeque;
use std::path::Path;
use std::process::Stdio;
use std::time::Duration;

use tokio::io::{AsyncRead, AsyncReadExt};

/// How long one line gets before it is killed.
///
/// Bounded by what an operator will watch a single message take, because this
/// runs inside a turn. Anything slower than this is a piece of work rather than
/// a question, and `code` is the tool with the long ceiling on it. The refusal
/// says exactly that, because a timeout an agent cannot act on gets retried.
pub const PATIENCE: Duration = Duration::from_secs(120);

/// How much of one stream is kept, in bytes, split between its two ends.
///
/// Both ends rather than one, because which end carries the answer depends on
/// the command and nothing here knows which was run: `git log` puts it at the
/// top and a failing test run puts it at the bottom. Keeping the head alone
/// sends a model back to re-run a slow command through `tail`; keeping the tail
/// alone silently drops the newest commits off a log. What fell out of the
/// middle is counted and said, so the model narrows the command rather than
/// concluding that was all the output there was.
const KEPT: usize = 12_000;

/// The shell a line is handed to.
///
/// `bash` rather than `sh`, because a model writes bash: `[[ ]]`, `pipefail`
/// and `$(...)` inside a conditional are what turn up, and under a POSIX `sh`
/// each is a syntax error about a token the model does not believe it wrote.
///
/// Not a login shell. [`crate::programs::adopt_operator_path`] already put the
/// operator's `PATH` on this process at startup, so `-l` would re-read their rc
/// files on every call to arrive at the same answer, slowly, and would run a
/// version manager's startup hook in front of every `git status`.
const SHELL: &str = "bash";

/// What one line did.
#[derive(Debug, Clone, PartialEq)]
pub struct Ran {
    pub stdout: String,
    pub stderr: String,
    /// Absent when the line was killed at [`PATIENCE`], which is the one way it
    /// ends without reporting one. Absent is not zero: a killed command has not
    /// succeeded, and a `0` here would say it had.
    pub exit_code: Option<i32>,
    /// Bytes that fell out of the middle of either stream.
    pub dropped: usize,
}

impl Ran {
    /// What the model is shown.
    ///
    /// Both streams, labeled, with the exit code only when it is not zero: a
    /// command that worked should read as its output and nothing else. The two
    /// unusual endings each say what to do next, because a tool result an agent
    /// cannot act on is one it reruns unchanged.
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
        match self.exit_code {
            None => {
                if !out.is_empty() {
                    out.push('\n');
                }
                out.push_str(&format!(
                    "(killed after {} seconds, so it did not finish and whatever it was doing may \
                     be half done. This tool is for commands that answer quickly. If the work \
                     genuinely takes minutes, hand it to `code` instead.)",
                    PATIENCE.as_secs()
                ));
            }
            Some(code) if code != 0 => {
                if !out.is_empty() {
                    out.push('\n');
                }
                out.push_str(&format!("(exit code {code})"));
            }
            Some(_) => {}
        }
        if out.is_empty() {
            out.push_str("(no output)");
        }
        out
    }
}

#[derive(Debug, thiserror::Error, PartialEq)]
pub enum ShellError {
    #[error(
        "`{0}` is not a directory on this machine any more. The repository was linked when it \
         was there, so it has been moved, renamed or deleted since. Tell the operator, and say \
         they can point the repository at where it is now from its panel"
    )]
    Gone(String),
    #[error(
        "the shell could not be started ({0}). Nothing ran. This is the app's own plumbing \
         rather than anything about your command, so tell the operator what you were trying to \
         do rather than rewriting it"
    )]
    Unstartable(String),
}

/// Runs one line in one directory and answers with what it did.
///
/// `stdin` is closed rather than inherited, and it is load-bearing twice over.
/// There is no terminal to read from, so a command that waits for input would
/// hold the turn until [`PATIENCE`] and then be killed with nothing to show for
/// it. `GIT_TERMINAL_PROMPT=0` is the same hazard from git's own end: a push to
/// a remote this machine is not signed in to prompts for a username, and a
/// prompt nobody can answer is two minutes of a turn spent on a question.
///
/// A killed line kills its own process and no further. A command that
/// backgrounded something outlives this, exactly as a coding job's does, and
/// for the same reason: a process group would also take out whatever the
/// operator's own tooling started in there.
pub async fn run(directory: &str, line: &str, patience: Duration) -> Result<Ran, ShellError> {
    // Checked here rather than left to the spawn, because the spawn's own error
    // for a missing directory is an errno the operator cannot act on and this
    // is the one failure that happens to people: a repository linked last month
    // and a directory renamed last week.
    if !Path::new(directory).is_dir() {
        return Err(ShellError::Gone(directory.to_string()));
    }

    let mut child = tokio::process::Command::new(SHELL)
        .arg("-c")
        .arg(line)
        .current_dir(directory)
        .env("GIT_TERMINAL_PROMPT", "0")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        // So a dropped future is a killed process rather than a shell left
        // running in somebody's repository with nothing holding a handle to it.
        .kill_on_drop(true)
        .spawn()
        .map_err(|err| ShellError::Unstartable(err.to_string()))?;

    // Taken before the wait, because both pipes have to be drained while the
    // child runs. A child that fills one and blocks on it, while this waits for
    // the child, is a deadlock that only shows up on commands that print more
    // than a pipe buffer holds.
    let mut out = child.stdout.take();
    let mut err = child.stderr.take();

    let reading = async {
        let (stdout, stderr) = tokio::join!(drain(&mut out), drain(&mut err));
        let status = child.wait().await;
        (stdout, stderr, status)
    };

    match tokio::time::timeout(patience, reading).await {
        Ok(((stdout, out_dropped), (stderr, err_dropped), status)) => Ok(Ran {
            stdout,
            stderr,
            // A status that could not be read is reported as killed rather than
            // as a success: the one thing this must never do is say a command
            // worked when nothing here knows whether it did.
            exit_code: status.ok().and_then(|status| status.code()),
            dropped: out_dropped + err_dropped,
        }),
        Err(_) => {
            // `kill_on_drop` would do this when the future is dropped, but the
            // kill is awaited here so the process is gone before the turn
            // carries on rather than at some point after it.
            let _ = child.kill().await;
            Ok(Ran { stdout: String::new(), stderr: String::new(), exit_code: None, dropped: 0 })
        }
    }
}

/// Reads one stream to its end, keeping both of its ends.
///
/// Reads past the cap rather than stopping at it, because a reader that stops
/// leaves the pipe to fill and the child to block on a write nobody will take:
/// the command would never finish and the only thing that ended the call would
/// be [`PATIENCE`]. So everything is read and the middle is thrown away as it
/// goes, which is what makes the memory this holds bounded by [`KEPT`] rather
/// than by what the command decided to print.
async fn drain<R: AsyncRead + Unpin>(reader: &mut Option<R>) -> (String, usize) {
    let Some(reader) = reader.as_mut() else { return (String::new(), 0) };

    let ends = KEPT / 2;
    let mut head: Vec<u8> = Vec::new();
    let mut tail: VecDeque<u8> = VecDeque::new();
    let mut dropped = 0usize;
    let mut chunk = [0u8; 8192];

    loop {
        let read = match reader.read(&mut chunk).await {
            Ok(0) | Err(_) => break,
            Ok(read) => read,
        };
        for &byte in &chunk[..read] {
            if head.len() < ends {
                head.push(byte);
                continue;
            }
            tail.push_back(byte);
            if tail.len() > ends {
                tail.pop_front();
                dropped += 1;
            }
        }
    }

    if dropped == 0 {
        head.extend(tail);
        // Lossy because a stream is bytes and a command is entitled to print
        // any of them. A model reading one replacement character in a binary
        // file is a better outcome than a tool that fails on it.
        return (String::from_utf8_lossy(&head).into_owned(), 0);
    }

    // The gap says what is missing and how to go and get it. Without the
    // second half a model reads the two ends as one continuous output, which
    // is how a summary comes back describing a file that is not there.
    let gap = format!(
        "\n\n… {dropped} bytes from the middle were dropped. Narrow the command — `head`, \
         `tail`, `grep`, a smaller range — if you need what was in there. …\n\n"
    );
    let mut whole = String::from_utf8_lossy(&head).into_owned();
    whole.push_str(&gap);
    whole.push_str(&String::from_utf8_lossy(tail.make_contiguous()));
    (whole, dropped)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ran(stdout: &str, stderr: &str, exit_code: Option<i32>) -> Ran {
        Ran { stdout: stdout.to_string(), stderr: stderr.to_string(), exit_code, dropped: 0 }
    }

    #[test]
    fn a_command_that_worked_reads_as_its_output_and_nothing_else() {
        assert_eq!(ran("on branch main\n", "", Some(0)).rendered(), "on branch main");
    }

    #[test]
    fn a_failure_carries_its_code_and_its_stderr() {
        let rendered = ran("", "fatal: not a git repository\n", Some(128)).rendered();
        assert!(rendered.contains("stderr: fatal: not a git repository"), "{rendered}");
        assert!(rendered.contains("(exit code 128)"), "{rendered}");
    }

    /// The one ending an agent has to be told what to do about, because the
    /// obvious move — run it again — is the one that spends another two
    /// minutes arriving in the same place.
    #[test]
    fn a_killed_command_says_it_did_not_finish_and_names_the_other_tool() {
        let rendered = ran("half of a build\n", "", None).rendered();
        assert!(rendered.contains("did not finish"), "{rendered}");
        assert!(rendered.contains("`code`"), "{rendered}");
        assert!(!rendered.contains("exit code"), "a kill is not an exit: {rendered}");
    }

    #[test]
    fn a_command_that_said_nothing_says_so() {
        assert_eq!(ran("", "", Some(0)).rendered(), "(no output)");
    }
}

/// Against a real shell, because the thing being tested is a process.
///
/// Same argument `tests/coding.rs` makes for its stand-ins and `programs.rs`
/// makes for its: what a child is handed, what it prints and whether it is
/// still running are only observable from outside it.
#[cfg(all(test, unix))]
mod running {
    use super::*;

    /// Generous, because these are real processes on a machine also running the
    /// rest of the suite. What is being asserted is the answer, not the
    /// deadline; the deadline has a test of its own.
    const ENOUGH: Duration = Duration::from_secs(60);

    #[tokio::test]
    async fn a_line_runs_in_the_directory_it_was_given() {
        let dir = std::env::temp_dir();
        let ran = run(dir.to_str().unwrap(), "pwd", ENOUGH).await.unwrap();
        assert_eq!(ran.exit_code, Some(0));
        // Compared by canonical path: macOS puts the temp directory under
        // `/var`, which is a symlink to `/private/var`, and `pwd` answers with
        // the resolved one.
        let said = std::fs::canonicalize(ran.stdout.trim()).unwrap();
        assert_eq!(said, std::fs::canonicalize(&dir).unwrap());
    }

    #[tokio::test]
    async fn a_failing_line_answers_rather_than_erroring() {
        let ran = run(".", "exit 3", ENOUGH).await.unwrap();
        assert_eq!(ran.exit_code, Some(3));
    }

    #[tokio::test]
    async fn both_streams_come_back() {
        let ran = run(".", "echo out; echo err >&2", ENOUGH).await.unwrap();
        assert_eq!(ran.stdout.trim(), "out");
        assert_eq!(ran.stderr.trim(), "err");
    }

    /// The bound that keeps one call from taking a context window with it, and
    /// the reason both ends are kept rather than one.
    #[tokio::test]
    async fn a_command_that_prints_too_much_keeps_both_ends_and_counts_the_middle() {
        // Numbered lines well past the cap, so the first and the last are far
        // enough apart that keeping both is the only way to have both.
        let ran = run(".", "seq 1 200000", ENOUGH).await.unwrap();
        assert!(ran.dropped > 0, "nothing was dropped, so the cap did not apply");
        assert!(ran.stdout.starts_with("1\n2\n3\n"), "the head is not the head");
        assert!(ran.stdout.trim_end().ends_with("200000"), "the tail is not the tail");
        assert!(ran.stdout.contains("bytes from the middle were dropped"), "the gap is silent");
        assert!(ran.stdout.len() < KEPT + 500, "kept {} bytes", ran.stdout.len());
        assert_eq!(ran.exit_code, Some(0), "the command still finished");
    }

    /// The deadlock this would have without the two pipes being drained while
    /// the child runs: more output than a pipe buffer holds, from a command
    /// that then has to exit.
    #[tokio::test]
    async fn a_command_that_fills_the_pipe_still_finishes() {
        let ran = run(".", "seq 1 100000 >&2; echo done", ENOUGH).await.unwrap();
        assert_eq!(ran.exit_code, Some(0));
        assert_eq!(ran.stdout.trim(), "done");
    }

    #[tokio::test]
    async fn a_line_that_will_not_end_is_killed_and_says_so() {
        let ran = run(".", "sleep 30", Duration::from_millis(300)).await.unwrap();
        assert_eq!(ran.exit_code, None);
        assert!(ran.rendered().contains("did not finish"), "{}", ran.rendered());
    }

    /// Not a nicety: with stdin inherited there is no terminal to read from,
    /// and a command that asks for input holds the turn until the ceiling.
    #[tokio::test]
    async fn a_command_that_waits_for_input_gets_nothing_and_ends() {
        let ran = run(".", "read line; echo \"got [$line]\"", ENOUGH).await.unwrap();
        assert_eq!(ran.stdout.trim(), "got []");
    }

    #[tokio::test]
    async fn a_directory_that_is_no_longer_there_says_who_can_fix_it() {
        let err = run("/no/such/directory/here", "pwd", ENOUGH).await.unwrap_err();
        assert_eq!(err, ShellError::Gone("/no/such/directory/here".to_string()));
        assert!(err.to_string().contains("operator"), "{err}");
    }
}
