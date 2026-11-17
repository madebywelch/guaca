use std::process::Stdio;
use std::time::Duration;

const PATIENCE: Duration = Duration::from_secs(5);

const MARK: &str = "__guaca_path__";

const FLAGS: [&str; 3] = ["-l", "-i", "-c"];

pub fn adopt_operator_path() {
    if cfg!(windows) {
        return;
    }

    let inherited = std::env::var("PATH").unwrap_or_default();
    let Some(adopted) = adopted(&inherited, &shell(), PATIENCE) else { return };

    tracing::info!(path = %adopted, "adopted the operator's PATH");
    std::env::set_var("PATH", adopted);
}

fn adopted(inherited: &str, shell: &str, patience: Duration) -> Option<String> {
    let found = probe(shell, patience)?;
    let merged = merged(&found, inherited);
    (merged != inherited).then_some(merged)
}

fn shell() -> String {
    std::env::var("SHELL").ok().filter(|s| !s.trim().is_empty()).unwrap_or("/bin/sh".to_string())
}

fn command() -> String {
    format!("printf '%s\\n' '{MARK}'; env")
}

fn probe(shell: &str, patience: Duration) -> Option<String> {
    let (answer, wait) = std::sync::mpsc::channel();
    let running = shell.to_string();
    std::thread::spawn(move || {
        let out = std::process::Command::new(&running)
            .args(FLAGS)
            .arg(command())
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

fn path_in(printed: &str) -> Option<&str> {
    let after = printed.rsplit_once(&format!("{MARK}\n")).map(|(_, rest)| rest)?;
    after
        .lines()
        .find_map(|line| line.strip_prefix("PATH="))
        .map(str::trim)
        .filter(|path| !path.is_empty())
}

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
        assert_eq!(merged("/a:", ":/b"), "/a:/b");
    }
}

#[cfg(all(test, unix))]
mod shells {
    use std::path::PathBuf;

    use super::*;

    const LAUNCHD: &str = "/usr/bin:/bin:/usr/sbin:/sbin";

    fn stand_in(name: &str, script: &str) -> PathBuf {
        let path = std::env::temp_dir().join(format!("guac-shell-{name}-{}", std::process::id()));
        crate::testing::write_executable(&path, script);
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

        let found = probe(shell.to_str().unwrap(), Duration::from_secs(60));
        assert_eq!(found.as_deref(), Some("/Users/someone/.local/bin:/usr/bin"));

        let argv = std::fs::read_to_string(format!("{}.argv", shell.display())).unwrap();
        assert!(argv.contains("-l"), "not a login shell: {argv}");
        assert!(argv.contains("-i"), "not an interactive shell: {argv}");
    }

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
