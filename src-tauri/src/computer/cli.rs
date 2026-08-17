//! A local runtime's command-line tool, spoken to by argument vector.
//!
//! Everything a local provider does is one of these: create a network, run a
//! container, exec inside it, inspect it in JSON. None of it goes through a
//! host shell, so nothing on this Mac interprets a string an agent influenced,
//! and a secret never becomes a character in a command line: the argument
//! vector carries `--env NAME` and the value travels in the child's own
//! environment.
//!
//! That environment is built rather than inherited. A CLI needs `PATH` and
//! `HOME` to find its own helpers and config; it has no business seeing the
//! operator's `AWS_SECRET_ACCESS_KEY`, and `SSH_AUTH_SOCK` deliberately does
//! not carry over, because a forwarded agent socket is an authority the guest
//! was never granted.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::time::Duration;

use tokio::io::AsyncReadExt;
use tokio::process::Command;

/// Host variables a local runtime is allowed to see, plus anything named
/// `LC_*` or `XDG_*`. Everything else is cleared: this is an allow-list rather
/// than a deny-list because the operator's shell is full of variables nobody
/// here can enumerate.
pub const HOST_ENV_ALLOWLIST: &[&str] =
    &["PATH", "HOME", "USER", "LOGNAME", "TMPDIR", "LANG", "SHELL"];

/// One local executable, found once and run many times.
#[derive(Debug)]
pub struct Cli {
    path: PathBuf,
}

/// What one run produced. `stdout` stays bytes because a CLI is also how a
/// screenshot comes back; `stderr` is a message for a person either way.
#[derive(Debug)]
pub struct CliOutput {
    pub status: i32,
    pub stdout: Vec<u8>,
    pub stderr: String,
    /// Which binary said this. Carried so a parse failure can name the thing
    /// that answered strangely; an operator reading "could not be read" needs
    /// to know what could not be.
    pub binary: String,
}

/// Why a local CLI produced nothing usable. Each variant is a different next
/// step: install it, look at what is wedged, or read the message.
#[derive(Debug, thiserror::Error)]
pub enum CliError {
    #[error("{0}")]
    Spawn(String),
    #[error("{binary} did not answer within {secs}s")]
    Timeout { binary: String, secs: u64 },
    #[error("{0}")]
    Io(String),
}

impl Cli {
    /// The first existing, executable candidate: each of `well_known` in turn,
    /// then `name` on the inherited `PATH`. Both halves are needed — a Mac app
    /// launched from Finder inherits a `PATH` that holds neither Homebrew nor
    /// `/usr/local/bin`, and an operator who installed the runtime elsewhere is
    /// only on `PATH`.
    pub fn discover(name: &str, well_known: &[&str]) -> Option<Cli> {
        if let Some(path) = well_known.iter().map(Path::new).find(|p| is_executable(p)) {
            return Some(Cli::at(path.to_path_buf()));
        }
        std::env::split_paths(&std::env::var_os("PATH")?)
            .map(|dir| dir.join(name))
            .find(|path| is_executable(path))
            .map(Cli::at)
    }

    pub fn at(path: PathBuf) -> Cli {
        Cli { path }
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    /// What to call this in a message to a person: the name they installed,
    /// not the path it happened to be found at.
    fn name(&self) -> String {
        self.path.file_name().unwrap_or(self.path.as_os_str()).to_string_lossy().into_owned()
    }

    /// Spawn `path argv…` with an allow-listed environment plus `secrets`, and
    /// wait up to `timeout`.
    ///
    /// `secrets` is borrowed and never formatted: the values in it exist in
    /// this call and in the child process, and nowhere a log or a panic could
    /// pick them up.
    pub async fn run(
        &self,
        argv: &[String],
        secrets: &BTreeMap<String, String>,
        timeout: Duration,
    ) -> Result<CliOutput, CliError> {
        let binary = self.name();
        // The first three arguments are the verb and its object, which is what
        // a debug trace is for; the rest can hold a label an agent named and is
        // no use anyway once the command is identified.
        tracing::debug!(
            binary = %binary,
            argv = ?&argv[..argv.len().min(3)],
            "running a local runtime command"
        );

        let mut command = Command::new(&self.path);
        command.args(argv);
        command.env_clear();
        for (name, value) in std::env::vars_os() {
            if name.to_str().is_some_and(allowed) {
                command.env(name, value);
            }
        }
        command.envs(secrets);
        // Nothing here has a terminal to answer a prompt with, and a child
        // reading a closed stdin gets EOF instead of waiting out the timeout.
        command.stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::piped());
        command.kill_on_drop(true);

        let mut child = command.spawn().map_err(|err| {
            CliError::Spawn(format!(
                "could not start {}: {err}; is it installed and executable?",
                self.path.display()
            ))
        })?;

        // Both pipes are drained while the child runs. A command that fills a
        // pipe buffer and blocks on the write never exits, so waiting first and
        // reading after would turn any chatty command into a timeout.
        let mut stdout_pipe = child.stdout.take().expect("stdout was piped above");
        let mut stderr_pipe = child.stderr.take().expect("stderr was piped above");
        let mut stdout = Vec::new();
        let mut stderr = Vec::new();
        let finished = tokio::time::timeout(timeout, async {
            let (status, out, err) = tokio::join!(
                child.wait(),
                stdout_pipe.read_to_end(&mut stdout),
                stderr_pipe.read_to_end(&mut stderr),
            );
            out.and(err).and(status)
        })
        .await;

        match finished {
            Ok(Ok(status)) => Ok(CliOutput {
                // A child killed by a signal has no code of its own. -1 is not
                // a status any process exits with, so a caller reading it as
                // failure is reading it correctly.
                status: status.code().unwrap_or(-1),
                stdout,
                stderr: String::from_utf8_lossy(&stderr).into_owned(),
                binary,
            }),
            Ok(Err(err)) => Err(CliError::Io(format!("{binary} could not be read: {err}"))),
            Err(_) => {
                // `kill_on_drop` covers the drop at the end of this scope; the
                // signal is asked for here so the child is already on its way
                // out while the caller decides what to do about the error.
                let _ = child.start_kill();
                Err(CliError::Timeout { binary, secs: timeout.as_secs() })
            }
        }
    }
}

/// Whether a host variable carries over into a local runtime's environment.
fn allowed(name: &str) -> bool {
    HOST_ENV_ALLOWLIST.contains(&name) || name.starts_with("LC_") || name.starts_with("XDG_")
}

/// A candidate is the runtime only if it is a file somebody can run: the
/// documented path also matches a directory and a half-finished download, and
/// taking either would report the runtime as installed and fail every command.
#[cfg(unix)]
fn is_executable(path: &Path) -> bool {
    use std::os::unix::fs::PermissionsExt;
    std::fs::metadata(path)
        .is_ok_and(|meta| meta.is_file() && meta.permissions().mode() & 0o111 != 0)
}

/// No local container runtime ships for Windows, so nothing found there is one.
/// The crate is still built for it, as `config.rs` is.
#[cfg(not(unix))]
fn is_executable(_path: &Path) -> bool {
    false
}

impl CliOutput {
    pub fn ok(&self) -> bool {
        self.status == 0
    }

    pub fn stdout_str(&self) -> String {
        String::from_utf8_lossy(&self.stdout).into_owned()
    }

    /// Machine-readable output only: a local runtime's tables are presentation
    /// and change between releases.
    pub fn json(&self) -> Result<serde_json::Value, CliError> {
        serde_json::from_slice(&self.stdout).map_err(|err| {
            CliError::Io(format!(
                "{} could not be read: it answered with something other than JSON ({err}); \
                 the message it printed was: {}",
                self.binary,
                first_line(&self.stdout_str(), &self.stderr)
            ))
        })
    }
}

/// What the runtime actually said, for an error about it not saying JSON.
/// Whichever stream carried it, trimmed to the line that names the problem.
fn first_line(stdout: &str, stderr: &str) -> String {
    let spoken = if stdout.trim().is_empty() { stderr } else { stdout };
    match spoken.lines().find(|line| !line.trim().is_empty()) {
        Some(line) => line.trim().chars().take(200).collect(),
        None => "(nothing)".to_string(),
    }
}

// Every fixture below is a `/bin/sh` script with an executable bit, which is
// the only shape this module is ever asked about.
#[cfg(all(test, unix))]
mod tests {
    use std::os::unix::fs::PermissionsExt;
    use std::time::Instant;

    use super::*;

    /// Prints its arguments one per line, a marker, then its whole environment.
    /// The two halves are read separately because that is exactly the claim
    /// under test: a secret is in the second and never the first.
    const REPORTER: &str =
        "#!/bin/sh\nfor a in \"$@\"; do printf '%s\\n' \"$a\"; done\necho ---ENV---\nenv | sort\n";

    fn fake(dir: &Path, name: &str, script: &str) -> PathBuf {
        let path = dir.join(name);
        std::fs::write(&path, script).unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
        path
    }

    fn no_secrets() -> BTreeMap<String, String> {
        BTreeMap::new()
    }

    fn argv(args: &[&str]) -> Vec<String> {
        args.iter().map(|a| a.to_string()).collect()
    }

    /// Splits the reporter's output into (arguments, environment lines).
    fn halves(out: &CliOutput) -> (Vec<String>, Vec<String>) {
        let text = out.stdout_str();
        let (args, env) = text.split_once("---ENV---\n").expect("the reporter prints the marker");
        (
            args.lines().map(|l| l.to_string()).collect(),
            env.lines().map(|l| l.to_string()).collect(),
        )
    }

    #[tokio::test]
    async fn arguments_reach_the_child_exactly_as_given() {
        // No shell is involved, so an argument with spaces stays one argument
        // and `$HOME` stays four characters. A provider builds argv from an
        // agent's names and paths; the moment either of these is untrue, so is
        // the promise that nothing on the host interprets them.
        let dir = tempfile::tempdir().unwrap();
        let cli = Cli::at(fake(dir.path(), "guac-reporter", REPORTER));

        let given = argv(&["list", "--filter", "label=guac.agent=a b", "$HOME", ";rm -rf /"]);
        let out = cli.run(&given, &no_secrets(), Duration::from_secs(10)).await.unwrap();

        assert!(out.ok(), "{}", out.stderr);
        let (seen, _) = halves(&out);
        assert_eq!(seen, given);
    }

    #[tokio::test]
    async fn a_secret_reaches_the_environment_and_never_the_arguments() {
        // The release-blocking one. `--env NAME` names a variable the child
        // already holds; a build that ever wrote `--env NAME=value` would put
        // the value in `ps`, in a crash report, and in any log that prints an
        // argv.
        std::env::set_var("GUAC_TEST_LEAK", "x");
        let dir = tempfile::tempdir().unwrap();
        let cli = Cli::at(fake(dir.path(), "guac-reporter", REPORTER));

        let secrets =
            BTreeMap::from([("GUAC_TEST_TOKEN".to_string(), "ghp_sentinel_9x".to_string())]);
        let out = cli
            .run(&argv(&["exec", "--env", "GUAC_TEST_TOKEN"]), &secrets, Duration::from_secs(10))
            .await
            .unwrap();

        let (seen, env) = halves(&out);
        assert!(
            seen.iter().all(|a| !a.contains("ghp_sentinel_9x")),
            "a secret value appeared in the argument vector: {seen:?}"
        );
        assert!(
            env.iter().any(|line| line == "GUAC_TEST_TOKEN=ghp_sentinel_9x"),
            "the child never received the secret it was told to pass on"
        );

        assert!(
            env.iter().all(|line| !line.starts_with("GUAC_TEST_LEAK=")),
            "a host variable off the allow-list reached the child: {env:?}"
        );
        assert!(env.iter().any(|line| line.starts_with("PATH=")), "{env:?}");
        assert!(env.iter().any(|line| line.starts_with("HOME=")), "{env:?}");
    }

    #[test]
    fn the_allow_list_admits_locale_and_desktop_variables_and_no_agent_socket() {
        assert!(allowed("PATH") && allowed("HOME") && allowed("TMPDIR"));
        assert!(allowed("LC_ALL") && allowed("XDG_RUNTIME_DIR"), "prefixes carry over");
        assert!(!allowed("SSH_AUTH_SOCK"), "a forwarded agent socket is authority, not config");
        assert!(!allowed("AWS_SECRET_ACCESS_KEY") && !allowed("OPENAI_API_KEY"));
    }

    #[tokio::test]
    async fn a_command_that_never_answers_is_reported_and_killed() {
        // A wedged runtime must not hold the turn that asked. The kill matters
        // as much as the error: a provider retrying an operation ten times
        // would otherwise leave ten of these behind.
        let dir = tempfile::tempdir().unwrap();
        let pidfile = dir.path().join("pid");
        let script = format!("#!/bin/sh\necho $$ > {}\nsleep 20\n", pidfile.display());
        let cli = Cli::at(fake(dir.path(), "guac-sleeper", &script));

        let started = Instant::now();
        // Two seconds against a twenty-second sleep: macOS takes its time over
        // the first execution of a file it has never seen, and this has to be
        // long enough that the child is certainly past the line recording its
        // pid, or the kill below is asserted against a process that never ran.
        let err = cli
            .run(&argv(&["wait"]), &no_secrets(), Duration::from_secs(2))
            .await
            .expect_err("a command that outlives its timeout is not a result");

        assert!(started.elapsed() < Duration::from_secs(10), "it waited for the child anyway");
        assert!(matches!(err, CliError::Timeout { .. }), "{err:?}");
        assert!(err.to_string().contains("guac-sleeper"), "{err}");

        let pid = std::fs::read_to_string(&pidfile)
            .expect("the child records its pid before it sleeps")
            .trim()
            .to_string();
        assert!(dead(&pid), "the child was left running after the timeout (pid {pid})");
    }

    /// True once the process is gone or a zombie awaiting its reaper. Polled:
    /// the signal is delivered by the OS, not by the call that returned.
    fn dead(pid: &str) -> bool {
        for _ in 0..40 {
            let ps = std::process::Command::new("/bin/ps")
                .args(["-o", "state=", "-p", pid])
                .output()
                .expect("/bin/ps says whether a pid is still there");
            let state = String::from_utf8_lossy(&ps.stdout).trim().to_string();
            if state.is_empty() || state.starts_with('Z') {
                return true;
            }
            std::thread::sleep(Duration::from_millis(50));
        }
        false
    }

    #[tokio::test]
    async fn the_guest_exit_code_is_the_result() {
        // A provider decides whether a resource exists from this number.
        let dir = tempfile::tempdir().unwrap();
        let cli =
            Cli::at(fake(dir.path(), "guac-exit", "#!/bin/sh\necho no such thing >&2\nexit 3\n"));

        let out = cli.run(&[], &no_secrets(), Duration::from_secs(10)).await.unwrap();

        assert_eq!(out.status, 3);
        assert!(!out.ok());
        assert_eq!(out.stderr.trim(), "no such thing");
    }

    #[tokio::test]
    async fn a_binary_that_is_not_there_says_so_and_says_what_to_do() {
        let dir = tempfile::tempdir().unwrap();
        let missing = dir.path().join("guac-absent");
        let cli = Cli::at(missing.clone());

        let err = cli
            .run(&[], &no_secrets(), Duration::from_secs(10))
            .await
            .expect_err("there is nothing to run");

        let message = err.to_string();
        assert!(matches!(err, CliError::Spawn(_)), "{err:?}");
        assert!(message.contains(&missing.display().to_string()), "{message}");
        assert!(message.contains("installed"), "a spawn failure has to say what to try: {message}");
    }

    #[tokio::test]
    async fn json_is_parsed_and_a_reply_that_is_not_json_names_the_binary() {
        // Presentation tables change between releases; JSON is the contract.
        // When a runtime prints a warning instead, the error has to say which
        // runtime, because the operator has two installed.
        let dir = tempfile::tempdir().unwrap();
        let cli = Cli::at(fake(dir.path(), "guac-speaker", "#!/bin/sh\nprintf '%s' \"$1\"\n"));

        let good = cli
            .run(
                &argv(&[r#"{"networks":[{"address":"192.168.64.3/24"}]}"#]),
                &no_secrets(),
                Duration::from_secs(10),
            )
            .await
            .unwrap();
        let value = good.json().unwrap();
        assert_eq!(value["networks"][0]["address"], "192.168.64.3/24");

        let bad = cli
            .run(&argv(&["Warning: kernel not installed"]), &no_secrets(), Duration::from_secs(10))
            .await
            .unwrap();
        let err = bad.json().expect_err("a warning is not JSON");
        assert!(err.to_string().contains("guac-speaker"), "{err}");
    }

    #[test]
    fn nothing_is_discovered_when_nothing_is_installed() {
        assert!(Cli::discover("definitely-not-a-binary-xyz", &[]).is_none());
    }

    #[test]
    fn a_well_known_location_wins_over_the_search_path() {
        // The documented install location is the signed one. An operator with
        // a wrapper script earlier on `PATH` should still get the real thing.
        let dir = tempfile::tempdir().unwrap();
        let on_path_dir = dir.path().join("bin");
        std::fs::create_dir(&on_path_dir).unwrap();
        let on_path = fake(&on_path_dir, "guac-discoverable", "#!/bin/sh\n");
        let well_known = fake(dir.path(), "guac-discoverable", "#!/bin/sh\n");

        let restore = std::env::var_os("PATH");
        // Prepended rather than replaced: other tests in this binary are
        // spawning children that need the real one.
        let mut search = vec![on_path_dir.clone()];
        search.extend(std::env::split_paths(restore.as_deref().unwrap_or_default()));
        std::env::set_var("PATH", std::env::join_paths(search).unwrap());

        let from_path = Cli::discover("guac-discoverable", &[]).expect("it is on PATH");
        assert_eq!(from_path.path(), on_path);

        let preferred =
            Cli::discover("guac-discoverable", &[&well_known.display().to_string()]).unwrap();
        assert_eq!(preferred.path(), well_known);

        if let Some(path) = restore {
            std::env::set_var("PATH", path);
        }
    }

    #[test]
    fn a_file_that_cannot_be_executed_is_not_the_runtime() {
        // A half-finished download is a file at the documented path. Taking it
        // would report the runtime as installed and fail on every command.
        let dir = tempfile::tempdir().unwrap();
        let unreadable = dir.path().join("guac-partial");
        std::fs::write(&unreadable, "#!/bin/sh\n").unwrap();
        std::fs::set_permissions(&unreadable, std::fs::Permissions::from_mode(0o644)).unwrap();
        let real = fake(dir.path(), "guac-real", "#!/bin/sh\n");

        let found = Cli::discover(
            "guac-nothing-by-this-name",
            &[
                &unreadable.display().to_string(),
                &dir.path().display().to_string(),
                &real.display().to_string(),
            ],
        );

        assert_eq!(
            found.expect("the executable one is there").path(),
            real,
            "a directory is not one either"
        );
    }
}
