//! Rebuilding this app from the checkout it was built from.
//!
//! A source build has no release to download. Its latest is the tip of the
//! branch its checkout follows, and `scripts/install.sh` is already the one
//! sequence that fetches that, builds it, swaps the bundle and opens the new
//! one. This starts the script and reports on it, and repeats none of it.
//!
//! The script quits this app once the new bundle is built, so it runs in a
//! process group of its own, which launchd does not end with the app.
//! The only outcome reported back is failure: success is this app closing and
//! a newer one opening in its place.
//!
//! On a box, the host is updated before the app, from the channel's own
//! build; `docs/UPDATES.md`, *The main channel*, says why that order.

use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::time::Duration;

use parking_lot::Mutex;
use serde::Serialize;

/// Where this app came from and where it lives, when both are a place a
/// rebuild can start from and land in.
#[derive(Clone, Debug, Default)]
pub struct Origin {
    /// The repository `install.sh` is run in.
    pub checkout: Option<PathBuf>,
    /// The directory holding the running `Guaca.app`, which `install.sh`
    /// replaces as `GUACA_DEST`.
    pub dest: Option<PathBuf>,
}

impl Origin {
    /// This process. A release is updated by downloading a release, so it
    /// has no checkout to rebuild from even on the machine that built it.
    pub fn this_app() -> Self {
        if option_env!("GUACA_RELEASE") == Some("1") {
            return Self::default();
        }
        let checkout = Path::new(env!("CARGO_MANIFEST_DIR")).parent().map(Path::to_path_buf);
        let dest = std::env::current_exe().ok().as_deref().and_then(installed_in);
        Self { checkout, dest }
    }
}

/// The directory an installed `Guaca.app` is in, from the path of its
/// executable. Anything else, such as `target/debug/guac` under `pnpm app`,
/// is not a bundle `install.sh` can replace.
fn installed_in(executable: &Path) -> Option<PathBuf> {
    let macos = executable.parent()?;
    let contents = macos.parent()?;
    let bundle = contents.parent()?;
    let named = |path: &Path, name: &str| path.file_name().is_some_and(|found| found == name);
    if named(macos, "MacOS") && named(contents, "Contents") && named(bundle, "Guaca.app") {
        bundle.parent().map(Path::to_path_buf)
    } else {
        None
    }
}

#[derive(Serialize, Debug, Default)]
#[serde(rename_all = "camelCase")]
pub struct Status {
    /// The checkout a rebuild runs in, when there is one it can run in.
    pub checkout: Option<String>,
    /// Why this app cannot rebuild itself, when it cannot.
    pub unavailable: Option<String>,
    /// The branch the checkout follows.
    pub branch: Option<String>,
    /// The commit that branch is at on `origin`, when it could be read.
    pub upstream: Option<String>,
    pub running: bool,
    /// The end of the log of the rebuild that last failed.
    pub failure: Option<String>,
    /// Where the whole log is.
    pub log: String,
}

pub struct Rebuild {
    origin: Origin,
    log: PathBuf,
    child: Mutex<Option<Child>>,
    failure: Mutex<Option<String>>,
}

/// How much of a failed log is shown. The error is in the last few lines;
/// the whole log is one click further.
const TAIL: usize = 12;

impl Rebuild {
    pub fn new(origin: Origin, log: PathBuf) -> Self {
        Self { origin, log, child: Mutex::new(None), failure: Mutex::new(None) }
    }

    /// The checkout and the destination, or what is missing, said so the
    /// operator knows what to run instead.
    fn usable(&self) -> Result<(&Path, &Path), String> {
        let Some(checkout) = self.origin.checkout.as_deref() else {
            return Err(
                "This is a release of Guaca. Download the next release to update it.".into()
            );
        };
        let Some(dest) = self.origin.dest.as_deref() else {
            return Err("This app is not running from an installed Guaca.app, so there is nothing for a rebuild to replace. Run scripts/install.sh to install it.".into());
        };
        if !checkout.join("scripts/install.sh").is_file() || !checkout.join(".git").exists() {
            return Err(format!(
                "The checkout this app was built from is no longer at {}. Run scripts/install.sh from wherever the source is now.",
                checkout.display()
            ));
        }
        Ok((checkout, dest))
    }

    /// Whether a rebuild is running, and how the last one ended if it failed.
    fn settle(&self) -> bool {
        let mut child = self.child.lock();
        let Some(running) = child.as_mut() else { return false };
        match running.try_wait() {
            Ok(None) => true,
            Ok(Some(status)) => {
                *child = None;
                if !status.success() {
                    *self.failure.lock() = Some(tail(&self.log));
                }
                false
            }
            Err(error) => {
                *child = None;
                *self.failure.lock() = Some(format!("The rebuild could not be followed: {error}"));
                false
            }
        }
    }

    pub async fn status(&self) -> Status {
        let running = self.settle();
        let mut status = Status {
            running,
            failure: self.failure.lock().clone(),
            log: self.log.display().to_string(),
            ..Status::default()
        };
        let checkout = match self.usable() {
            Ok((checkout, _)) => checkout,
            Err(reason) => {
                status.unavailable = Some(reason);
                return status;
            }
        };
        status.checkout = Some(checkout.display().to_string());
        status.branch = git(checkout, &["rev-parse", "--abbrev-ref", "HEAD"]).await.ok();
        if let Some(branch) = status.branch.as_deref() {
            status.upstream =
                git(checkout, &["ls-remote", "origin", &format!("refs/heads/{branch}")])
                    .await
                    .ok()
                    .and_then(|line| line.split_whitespace().next().map(str::to_string))
                    .filter(|commit| {
                        commit.len() == 40 && commit.bytes().all(|c| c.is_ascii_hexdigit())
                    });
        }
        status
    }

    /// Starts `install.sh` and returns. It builds while this app keeps
    /// running, then quits it and opens the new one.
    pub fn start(&self) -> Result<(), String> {
        let mut child = self.child.lock();
        if let Some(running) = child.as_mut() {
            if matches!(running.try_wait(), Ok(None)) {
                return Err(
                    "Guaca is already being rebuilt. It restarts when the build finishes.".into()
                );
            }
        }
        let (checkout, dest) = self.usable()?;
        if let Some(parent) = self.log.parent() {
            std::fs::create_dir_all(parent).map_err(|e| {
                format!("Could not keep the rebuild's log at {}: {e}", self.log.display())
            })?;
        }
        let log = std::fs::File::create(&self.log).map_err(|e| {
            format!("Could not keep the rebuild's log at {}: {e}", self.log.display())
        })?;
        let errors =
            log.try_clone().map_err(|e| format!("Could not keep the rebuild's log: {e}"))?;
        let mut command = Command::new("/bin/bash");
        command
            .arg(checkout.join("scripts/install.sh"))
            .arg("--launch")
            .current_dir(checkout)
            .env("GUACA_DEST", dest)
            .stdin(Stdio::null())
            .stdout(log)
            .stderr(errors);
        #[cfg(unix)]
        {
            use std::os::unix::process::CommandExt;
            // Its own group: launchd ends what is left of an app's process
            // group when the app exits, and the script is what quits it.
            command.process_group(0);
        }
        let started =
            command.spawn().map_err(|e| format!("Could not start scripts/install.sh: {e}"))?;
        tracing::info!(pid = started.id(), checkout = %checkout.display(), "rebuilding this app");
        *self.failure.lock() = None;
        *child = Some(started);
        Ok(())
    }
}

/// The last lines of the log, which is where `install.sh` says what stopped it.
fn tail(log: &Path) -> String {
    let text = std::fs::read_to_string(log).unwrap_or_default();
    let lines: Vec<&str> = text.lines().filter(|line| !line.trim().is_empty()).collect();
    let said = lines[lines.len().saturating_sub(TAIL)..].join("\n");
    if said.is_empty() {
        "The rebuild stopped without saying why.".into()
    } else {
        said
    }
}

async fn git(dir: &Path, args: &[&str]) -> Result<String, String> {
    let output = tokio::time::timeout(
        Duration::from_secs(15),
        tokio::process::Command::new("git")
            .arg("-C")
            .arg(dir)
            .args(args)
            .kill_on_drop(true)
            .output(),
    )
    .await
    .map_err(|_| "git took too long".to_string())?
    .map_err(|e| e.to_string())?;
    if !output.status.success() {
        return Err(String::from_utf8_lossy(&output.stderr).trim().to_string());
    }
    Ok(String::from_utf8_lossy(&output.stdout).trim().to_string())
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;

    /// A checkout whose `install.sh` is `script`, with an origin of its own.
    fn checkout(script: &str) -> (tempfile::TempDir, Origin) {
        let dir = tempfile::tempdir().unwrap();
        let repo = dir.path().join("guaca");
        let origin = dir.path().join("origin.git");
        let run = |args: &[&str], at: &Path| {
            let ok = std::process::Command::new("git").args(args).current_dir(at).output().unwrap();
            assert!(ok.status.success(), "{}", String::from_utf8_lossy(&ok.stderr));
        };
        std::fs::create_dir_all(repo.join("scripts")).unwrap();
        std::fs::write(repo.join("scripts/install.sh"), script).unwrap();
        run(&["init", "--quiet", "--initial-branch", "main"], &repo);
        run(
            &[
                "-c",
                "user.name=t",
                "-c",
                "user.email=t@t",
                "commit",
                "--quiet",
                "--allow-empty",
                "-m",
                "one",
            ],
            &repo,
        );
        run(
            &["clone", "--quiet", "--bare", repo.to_str().unwrap(), origin.to_str().unwrap()],
            dir.path(),
        );
        run(&["remote", "add", "origin", origin.to_str().unwrap()], &repo);
        let dest = dir.path().join("Applications");
        std::fs::create_dir_all(&dest).unwrap();
        (dir, Origin { checkout: Some(repo), dest: Some(dest) })
    }

    async fn settled(rebuild: &Rebuild) -> Status {
        for _ in 0..200 {
            let status = rebuild.status().await;
            if !status.running {
                return status;
            }
            tokio::time::sleep(Duration::from_millis(25)).await;
        }
        panic!("the rebuild did not finish");
    }

    #[tokio::test]
    async fn a_release_a_loose_binary_and_a_moved_checkout_each_say_what_to_do_instead() {
        let (dir, origin) = checkout("exit 0\n");
        let log = dir.path().join("rebuild.log");
        for (origin, said) in [
            (Origin::default(), "Download the next release"),
            (Origin { dest: None, ..origin.clone() }, "scripts/install.sh to install it"),
            (Origin { checkout: Some(dir.path().join("gone")), ..origin.clone() }, "no longer at"),
        ] {
            let rebuild = Rebuild::new(origin, log.clone());
            let refused = rebuild.start().unwrap_err();
            assert!(refused.contains(said), "{refused}");
            let status = rebuild.status().await;
            assert!(status.unavailable.unwrap().contains(said));
            assert!(status.checkout.is_none() && status.upstream.is_none());
        }
        assert!(!log.exists(), "nothing was started");
    }

    #[tokio::test]
    async fn a_failed_rebuild_reports_the_end_of_its_log_and_can_be_started_again() {
        let script = "for i in $(seq 1 30); do echo line $i; done\necho \"$GUACA_DEST\" >&2\necho 'cargo is not on PATH' >&2\nexit 1\n";
        let (dir, origin) = checkout(script);
        let dest = origin.dest.clone().unwrap();
        let rebuild = Rebuild::new(origin, dir.path().join("logs/rebuild.log"));
        rebuild.start().unwrap();
        let status = settled(&rebuild).await;
        let failure = status.failure.unwrap();
        assert!(failure.ends_with("cargo is not on PATH"), "{failure}");
        assert!(failure.contains(&dest.display().to_string()), "installs where this app is");
        assert!(!failure.contains("line 1\n"), "only the end: {failure}");
        rebuild.start().unwrap();
        // Read without settling: the script may already have failed again.
        assert!(rebuild.failure.lock().is_none(), "a new attempt clears the last failure");
        settled(&rebuild).await;
    }

    #[tokio::test]
    async fn a_second_rebuild_is_refused_while_one_runs() {
        // `exec`, so the process killed below is the whole of it.
        let (dir, origin) = checkout("exec sleep 30\n");
        let rebuild = Rebuild::new(origin, dir.path().join("rebuild.log"));
        rebuild.start().unwrap();
        let refused = rebuild.start().unwrap_err();
        assert!(refused.contains("already being rebuilt"), "{refused}");
        assert!(rebuild.status().await.running);
        rebuild.child.lock().as_mut().unwrap().kill().unwrap();
    }

    #[tokio::test]
    async fn the_status_names_the_branch_and_where_origin_has_it() {
        let (dir, origin) = checkout("exit 0\n");
        let repo = origin.checkout.clone().unwrap();
        let head = std::process::Command::new("git")
            .args(["rev-parse", "HEAD"])
            .current_dir(&repo)
            .output()
            .unwrap();
        let rebuild = Rebuild::new(origin, dir.path().join("rebuild.log"));
        let status = rebuild.status().await;
        assert_eq!(status.branch.as_deref(), Some("main"));
        assert_eq!(status.upstream.as_deref(), Some(String::from_utf8_lossy(&head.stdout).trim()));
        assert!(status.unavailable.is_none() && !status.running);
    }

    #[test]
    fn only_an_installed_bundle_has_somewhere_to_land() {
        assert_eq!(
            installed_in(Path::new("/Applications/Guaca.app/Contents/MacOS/guac")),
            Some(PathBuf::from("/Applications"))
        );
        assert_eq!(
            installed_in(Path::new("/Users/r/Applications/Guaca.app/Contents/MacOS/guac")),
            Some(PathBuf::from("/Users/r/Applications"))
        );
        for loose in [
            "/Users/r/guaca/src-tauri/target/debug/guac",
            "/Applications/Other.app/Contents/MacOS/guac",
            "/tmp/Guaca.app/Contents/Resources/guac",
        ] {
            assert_eq!(installed_in(Path::new(loose)), None, "{loose}");
        }
    }

    /// A stand-in for the installed app, ended when the test is.
    #[cfg(target_os = "macos")]
    struct Running(std::process::Child);

    #[cfg(target_os = "macos")]
    impl Drop for Running {
        fn drop(&mut self) {
            let _ = self.0.kill();
            let _ = self.0.wait();
        }
    }

    /// The real `install.sh`, started by the installed app the way `start`
    /// starts it: whether the app was still running when the script reached
    /// the copy, and the script's log. The app is `bash` reached through a
    /// symlink at the installed executable's path, so `pgrep -f` sees what it
    /// sees for Guaca, and it exits when asked to quit unless `stubborn`.
    /// Every tool that builds, signs or quits is a stub; the one that copies
    /// records what it found and fails, so nothing is replaced.
    #[cfg(target_os = "macos")]
    fn install_from_the_app(stubborn: bool, within: Duration) -> (String, String) {
        use std::os::unix::fs::{symlink, PermissionsExt};

        let dir = tempfile::tempdir().unwrap();
        let repo = dir.path().join("guaca");
        let dest = dir.path().join("Applications");
        let stubs = dir.path().join("bin");
        let built = repo.join("src-tauri/target/release/bundle/macos/Guaca.app/Contents");
        let exec = dest.join("Guaca.app/Contents/MacOS/guac");
        let (log, quit, seen) =
            (dir.path().join("install.log"), dir.path().join("quit"), dir.path().join("seen"));
        for made in
            [repo.join("scripts"), built.clone(), exec.parent().unwrap().into(), stubs.clone()]
        {
            std::fs::create_dir_all(made).unwrap();
        }
        std::fs::write(repo.join("scripts/install.sh"), include_str!("../../scripts/install.sh"))
            .unwrap();
        std::fs::write(
            built.join("Info.plist"),
            "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<plist version=\"1.0\"><dict>\
             <key>CFBundleIdentifier</key><string>com.madebywelch.guac.test</string>\
             <key>CFBundleShortVersionString</key><string>0.0.0</string>\
             </dict></plist>\n",
        )
        .unwrap();
        symlink("/bin/bash", &exec).unwrap();
        let (exec, quit, seen) = (exec.display(), quit.display(), seen.display());
        for (tool, body) in [
            ("git", "echo 0000000".to_string()),
            ("pnpm", "exit 0".into()),
            ("cargo", "exit 0".into()),
            ("codesign", "exit 0".into()),
            ("osascript", format!("touch '{quit}'")),
            (
                "ditto",
                format!(
                    "if pgrep -a -f '^{exec}' >/dev/null; then echo running; else echo gone; fi > '{seen}'\nexit 1"
                ),
            ),
        ] {
            let stub = stubs.join(tool);
            std::fs::write(&stub, format!("#!/bin/sh\n{body}\n")).unwrap();
            std::fs::set_permissions(&stub, std::fs::Permissions::from_mode(0o755)).unwrap();
        }

        let until = if stubborn { ":".to_string() } else { format!("[ ! -e '{quit}' ]") };
        let _app = Running(
            std::process::Command::new(exec.to_string())
                .arg("-c")
                .arg(format!(
                    "/bin/bash '{}' --no-pull >'{}' 2>&1 &\nwhile {until}; do sleep 0.05; done",
                    repo.join("scripts/install.sh").display(),
                    log.display()
                ))
                .env("PATH", format!("{}:/usr/bin:/bin:/usr/sbin:/sbin", stubs.display()))
                .env("GUACA_DEST", &dest)
                .env_remove("GUACA_SIGN_IDENTITY")
                .spawn()
                .unwrap(),
        );
        let started = std::time::Instant::now();
        let seen = loop {
            let said = std::fs::read_to_string(seen.to_string()).unwrap_or_default();
            if !said.trim().is_empty() {
                break said.trim().to_string();
            }
            assert!(
                started.elapsed() < within,
                "the script never reached the copy:\n{}",
                std::fs::read_to_string(&log).unwrap_or_default()
            );
            std::thread::sleep(Duration::from_millis(50));
        };
        (seen, std::fs::read_to_string(&log).unwrap())
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn the_app_that_starts_the_install_is_quit_before_its_bundle_is_replaced() {
        let (app, log) = install_from_the_app(false, Duration::from_secs(10));
        assert_eq!(app, "gone", "{log}");
        assert!(log.contains("Quitting the running Guaca"), "{log}");
        assert!(!log.contains("did not quit on its own"), "{log}");
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn the_app_that_starts_the_install_and_ignores_the_quit_is_ended_before_its_bundle_is_replaced()
    {
        let (app, log) = install_from_the_app(true, Duration::from_secs(30));
        assert_eq!(app, "gone", "{log}");
        assert!(log.contains("it did not quit on its own; ending it"), "{log}");
    }
}
