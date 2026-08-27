//! The `PATH` this app finds programs on, which is not the one the operator has.
//!
//! Four things here are a process rather than a call: `claude` for a turn paid
//! for by a Claude plan, `claude` or `pi` again for a coding job, and `git` with
//! `gh` beside it for a repository. Every one of them is found by name, for the
//! reason `coding::binary` gives: a second place to say where a program
//! lives is a second place for it to be wrong.
//!
//! What that assumes is that this process has the operator's `PATH`. A
//! double-clicked app does not. macOS starts one from the Dock or Finder under
//! `launchd`, which hands it `PATH=/usr/bin:/bin:/usr/sbin:/sbin` and nothing
//! else: no `/opt/homebrew/bin`, where Homebrew puts `pi` and `gh`, and no
//! `~/.local/bin`, where Anthropic's own installer puts `claude`. The same
//! binary started from a terminal inherits that terminal's `PATH` and finds all
//! of them, which is why every test, every `pnpm app` and every `cargo run`
//! passes and only the built app fails.
//!
//! The symptom is a refusal that is false. *claude is not installed* was said to
//! an operator who had `claude` on their `PATH`, in the shell they had built the
//! app in, one directory away from the source of the message. A refusal nobody
//! can act on is worse than no feature: the operator's next move is to install
//! what is already installed.
//!
//! ## The operator's shell is the only place the answer is
//!
//! `/etc/paths` and `/etc/paths.d` describe the machine rather than the person,
//! and neither mentions a directory under `$HOME`. Guessing a list of likely
//! install directories is worse than asking: it is right about the machine it
//! was written on and silently short everywhere else, and it goes stale the day
//! a vendor moves. So the shell is asked, once, and what it says is what this
//! process uses.
//!
//! ## It has to be a login shell *and* an interactive one
//!
//! Measured on the machine this was found on, with `launchd`'s environment:
//! `zsh -l -c` answers without `~/.local/bin` and `zsh -l -i -c` answers with
//! it, because a zsh user's `PATH` is written in `.zshrc` and only an
//! interactive shell reads that file. A login-only probe is a fix that leaves
//! `claude` exactly as missing as it was, and looks like it worked.
//!
//! ## What is read is `env`, not `echo $PATH`
//!
//! `PATH` is a colon-joined string in `sh`, `bash` and `zsh` and a list in
//! `fish`, so an `echo` has to be written differently per shell and the third
//! one is the one nobody tests. `env` prints what a child of that shell would
//! actually be given, in all of them, which is the question being asked. The
//! marker line in front of it is because an rc file is entitled to print
//! things — a banner, a version notice, a prompt framework's warning — and the
//! answer has to survive that.

use std::process::Stdio;
use std::time::Duration;

/// How long the shell gets. An rc file that loads a version manager can take a
/// second; one that waits for input never answers. This is startup, so the
/// ceiling is what keeps a badly configured shell from being an app that does
/// not open.
const PATIENCE: Duration = Duration::from_secs(5);

/// Written before `env` so the answer can be found under whatever the operator's
/// rc files printed first.
const MARK: &str = "__guaca_path__";

/// Login *and* interactive, which is the whole finding. See the module docs
/// before dropping either.
const FLAGS: [&str; 3] = ["-l", "-i", "-c"];

/// Puts the operator's `PATH` on this process, so the programs it runs are found
/// where they actually are.
///
/// Called once, at the top of [`crate::run`], and it must stay there: this is a
/// process-wide environment write, which is only sound while this is the one
/// thread. The alternative is passing an environment to each of the six spawn
/// sites, which is six places to remember and five places for the next one to be
/// forgotten.
///
/// Silent when it changes nothing, which is every start from a terminal.
pub fn adopt_operator_path() {
    // A Windows shell has no `-l -i -c` and no `env`, and nothing here would
    // answer. The app's own `PATH` is left alone rather than probed at.
    if cfg!(windows) {
        return;
    }

    let inherited = std::env::var("PATH").unwrap_or_default();
    let Some(adopted) = adopted(&inherited, &shell(), PATIENCE) else { return };

    tracing::info!(path = %adopted, "adopted the operator's PATH");
    std::env::set_var("PATH", adopted);
}

/// What this process's `PATH` should become, or `None` for leave it alone.
///
/// Split from the one line that writes it so the whole decision is testable
/// against a shell that is not this machine's. `None` covers both ways there is
/// nothing to do: a shell that would not answer, and an answer that adds
/// nothing, which is every start from a terminal.
fn adopted(inherited: &str, shell: &str, patience: Duration) -> Option<String> {
    let found = probe(shell, patience)?;
    let merged = merged(&found, inherited);
    (merged != inherited).then_some(merged)
}

/// The operator's shell, as their login session records it.
///
/// `launchd` passes `SHELL` to a double-clicked app, so this is answered from
/// the environment rather than from the password database. `/bin/sh` is the
/// fallback and is expected to fail on some machines: a probe that answers
/// nothing leaves the `PATH` alone, which is where this started.
fn shell() -> String {
    std::env::var("SHELL").ok().filter(|s| !s.trim().is_empty()).unwrap_or("/bin/sh".to_string())
}

/// What the shell is asked to print.
fn command() -> String {
    format!("printf '%s\\n' '{MARK}'; env")
}

/// Runs one shell and reads the `PATH` out of what it printed.
///
/// `None` for every way this can go wrong, and that is the whole error handling:
/// a shell that is not there, one that fails, one that prints nothing usable and
/// one that never returns all mean the same thing here, which is that this
/// process keeps the `PATH` it was started with and the refusals downstream say
/// so.
///
/// The wait is a thread rather than a timeout on the child because there is no
/// async runtime yet. A shell that outlives its patience is left running and its
/// answer is dropped: nothing is written to the environment on that path, which
/// is what keeps the abandoned thread's own `spawn` from racing a `set_var`
/// here.
fn probe(shell: &str, patience: Duration) -> Option<String> {
    let (answer, wait) = std::sync::mpsc::channel();
    let running = shell.to_string();
    std::thread::spawn(move || {
        let out = std::process::Command::new(&running)
            .args(FLAGS)
            .arg(command())
            // No terminal to read from, and an rc file that asks for input
            // would otherwise hold the app shut until the patience runs out.
            .stdin(Stdio::null())
            .stderr(Stdio::null())
            .output();
        let _ = answer.send(out);
    });

    match wait.recv_timeout(patience) {
        Ok(Ok(out)) => {
            let printed = String::from_utf8_lossy(&out.stdout);
            let found = path_in(&printed).map(str::to_string);
            if found.is_none() {
                tracing::debug!(%shell, "the shell printed no PATH; keeping this app's own");
            }
            found
        }
        Ok(Err(err)) => {
            tracing::debug!(%shell, %err, "could not run the shell; keeping this app's own PATH");
            None
        }
        Err(_) => {
            tracing::warn!(
                %shell,
                secs = patience.as_secs(),
                "the shell did not answer in time; programs will be looked for on this app's \
                 own PATH, which is not the one your terminal has"
            );
            None
        }
    }
}

/// Finds the `PATH` in a shell's output.
///
/// From after the last marker, because everything before it is whatever the rc
/// files had to say and one of those lines is allowed to look like anything.
fn path_in(printed: &str) -> Option<&str> {
    let after = printed.rsplit_once(&format!("{MARK}\n")).map(|(_, rest)| rest)?;
    after
        .lines()
        .find_map(|line| line.strip_prefix("PATH="))
        .map(str::trim)
        .filter(|path| !path.is_empty())
}

/// The operator's entries first, then anything this process had that they do
/// not.
///
/// Their order is the one they chose and the one their shell resolves a name in,
/// so it wins. Nothing inherited is dropped: `launchd`'s four are already in
/// every real shell's `PATH`, and a directory this app was handed deliberately —
/// by a wrapper, by a test, by a launcher — is not this function's to discard.
fn merged(operator: &str, inherited: &str) -> String {
    let mut kept: Vec<&str> = Vec::new();
    for entry in operator.split(':').chain(inherited.split(':')) {
        if !entry.is_empty() && !kept.contains(&entry) {
            kept.push(entry);
        }
    }
    kept.join(":")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_shell_is_asked_as_a_login_shell_and_an_interactive_one() {
        // Both, and the second is the one that looks droppable: a zsh user's
        // PATH is in `.zshrc`, which a login shell alone never reads.
        assert!(FLAGS.contains(&"-l"), "a login shell reads the profile: {FLAGS:?}");
        assert!(FLAGS.contains(&"-i"), "an interactive one reads the rc file: {FLAGS:?}");
    }

    #[test]
    fn the_path_is_read_from_under_whatever_the_rc_files_printed() {
        let printed = format!(
            "welcome to your shell\nPATH=/decoy\n{MARK}\nHOME=/Users/someone\n\
             PATH=/opt/homebrew/bin:/usr/bin\nSHELL=/bin/zsh\n"
        );
        assert_eq!(path_in(&printed), Some("/opt/homebrew/bin:/usr/bin"));
    }

    #[test]
    fn output_with_no_marker_is_no_answer() {
        // A shell that failed before it got to the command prints its own
        // diagnostics, and a PATH line in those is not an answer to anything.
        assert_eq!(path_in("PATH=/usr/bin\n"), None);
        assert_eq!(path_in(""), None);
    }

    #[test]
    fn an_empty_path_is_no_answer() {
        assert_eq!(path_in(&format!("{MARK}\nPATH=\n")), None);
    }

    #[test]
    fn the_operators_entries_come_first_and_nothing_is_dropped() {
        assert_eq!(
            merged("/opt/homebrew/bin:/usr/bin:/bin", "/usr/bin:/bin:/usr/sbin:/sbin"),
            "/opt/homebrew/bin:/usr/bin:/bin:/usr/sbin:/sbin"
        );
    }

    #[test]
    fn a_repeated_entry_is_kept_once() {
        assert_eq!(merged("/a:/a:/b", "/b:/c"), "/a:/b:/c");
    }

    #[test]
    fn an_empty_entry_is_dropped() {
        // A trailing colon means "the current directory" to some shells, which
        // is not a place this app looks a program up in.
        assert_eq!(merged("/a:", ":/b"), "/a:/b");
    }
}

/// The probe against a real process, driven by stand-in shells on disk.
///
/// A stand-in rather than a mock for the reason `tests/coding.rs` uses them: the
/// thing being tested is a process, and what it is handed and what it prints are
/// only observable from outside it.
#[cfg(all(test, unix))]
mod shells {
    use std::io::Write;
    use std::os::unix::fs::PermissionsExt;
    use std::path::PathBuf;

    use super::*;

    /// What `launchd` hands an app started from the Dock or the Finder. Not a
    /// simplification of the real thing: it is the whole of it.
    const LAUNCHD: &str = "/usr/bin:/bin:/usr/sbin:/sbin";

    /// Writes an executable stand-in shell and answers with its path.
    fn stand_in(name: &str, script: &str) -> PathBuf {
        let path = std::env::temp_dir().join(format!("guac-shell-{name}-{}", std::process::id()));
        let mut file = std::fs::File::create(&path).unwrap();
        file.write_all(script.as_bytes()).unwrap();
        file.flush().unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
        path
    }

    #[test]
    fn the_shells_own_path_is_what_comes_back() {
        let shell = stand_in(
            "answers",
            &format!(
                "#!/bin/sh\n\
                 echo \"$@\" > \"$0.argv\"\n\
                 echo 'a banner nobody asked for'\n\
                 printf '%s\\n' '{MARK}'\n\
                 PATH=/Users/someone/.local/bin:/usr/bin env\n"
            ),
        );

        // Patience well past the real one, because what is being asserted is
        // the answer rather than the deadline: a file written a moment ago is
        // being executed for the first time, on a machine that is also running
        // the rest of this suite, and it once took five seconds to get going.
        // The deadline has a test of its own below.
        let found = probe(shell.to_str().unwrap(), Duration::from_secs(60));
        assert_eq!(found.as_deref(), Some("/Users/someone/.local/bin:/usr/bin"));

        // And it was asked the way the module docs say it has to be.
        let argv = std::fs::read_to_string(format!("{}.argv", shell.display())).unwrap();
        assert!(argv.contains("-l"), "not a login shell: {argv}");
        assert!(argv.contains("-i"), "not an interactive shell: {argv}");
    }

    /// The bug this module exists for, in one assertion: the `PATH` `launchd`
    /// hands a double-clicked app, and the `PATH` it should be working with.
    #[test]
    fn a_launchd_path_becomes_the_operators_path() {
        let shell = stand_in(
            "adopts",
            &format!(
                "#!/bin/sh\n\
                 printf '%s\\n' '{MARK}'\n\
                 PATH=/Users/someone/.local/bin:/opt/homebrew/bin:/usr/bin env\n"
            ),
        );

        assert_eq!(
            adopted(LAUNCHD, shell.to_str().unwrap(), Duration::from_secs(60)).as_deref(),
            Some("/Users/someone/.local/bin:/opt/homebrew/bin:/usr/bin:/bin:/usr/sbin:/sbin")
        );
    }

    /// Which is every start from a terminal, and it must not be reported as a
    /// change.
    #[test]
    fn a_path_that_already_has_the_shells_own_is_left_alone() {
        let shell = stand_in(
            "agrees",
            &format!(
                "#!/bin/sh\n\
                 printf '%s\\n' '{MARK}'\n\
                 PATH={LAUNCHD} env\n"
            ),
        );

        assert_eq!(adopted(LAUNCHD, shell.to_str().unwrap(), Duration::from_secs(60)), None);
    }

    #[test]
    fn a_shell_that_never_answers_leaves_the_path_alone() {
        let shell = stand_in("hangs", "#!/bin/sh\nsleep 30\n");
        assert_eq!(probe(shell.to_str().unwrap(), Duration::from_millis(200)), None);
    }

    #[test]
    fn a_shell_that_is_not_there_leaves_the_path_alone() {
        assert_eq!(probe("/nowhere/no-such-shell", PATIENCE), None);
    }
}
