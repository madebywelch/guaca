//! What the repository subsystem left on disk, put away once it is safe to.
//!
//! Before an agent had a terminal, a workspace kept three things for its
//! repositories, and nothing reads any of them now:
//!
//! - `<data>/worktrees/<repository>/<agent>`, one git work tree per agent, each
//!   linked to a checkout the operator linked or a clone the box made;
//! - `<data>/repos/<clone>`, the clones a box made of a remote;
//! - `<config>/repo-credentials/`, the token each clone pushed with.
//!
//! The tokens go unconditionally: a credential nothing uses is a credential
//! nobody will remember to rotate. Everything else goes only where nothing is
//! lost by it. A work tree with nothing uncommitted, whose commit is still held
//! by the repository it came from, is removed through git, which also takes
//! its entry out of that repository's own list. A work tree holding work that
//! exists nowhere else is moved into the terminal of the agent it belonged to,
//! still linked, so the agent finds it where it now works. A clone is removed
//! once nothing in it is unpushed and no work tree still depends on it.
//! Anything that fits none of those is kept, and said so in the log with the
//! reason, every time the workspace opens, until somebody deals with it.
//!
//! Run at every boot and a no-op once there is nothing left, so a box that was
//! down during the release that removed repositories is tidied when it comes
//! back rather than never.

use std::path::{Path, PathBuf};
use std::time::Duration;

use crate::domain::ids::AgentId;

/// What one pass did, for the log and for the tests.
#[derive(Debug, Default, PartialEq)]
pub struct Report {
    pub removed: Vec<PathBuf>,
    /// Work trees moved into a terminal: from, to.
    pub moved: Vec<(PathBuf, PathBuf)>,
    /// Left where they are, and why.
    pub kept: Vec<(PathBuf, String)>,
}

/// Puts away what is safe to, under `data` and `config`.
///
/// `terminal_of` is where a living agent's terminal is, made if need be, and
/// `None` for an agent that has been deleted: work of an agent that is gone has
/// nowhere of its own to go, and is kept rather than handed to somebody else.
pub async fn put_away(
    data: &Path,
    config: &Path,
    terminal_of: impl Fn(AgentId) -> Option<PathBuf>,
) -> Report {
    let mut report = Report::default();
    // Canonical, because git answers with canonical paths and "is this clone
    // one of ours" is a prefix test against it.
    let data = &tokio::fs::canonicalize(data).await.unwrap_or_else(|_| data.to_path_buf());

    let credentials = config.join("repo-credentials");
    if credentials.exists() {
        match tokio::fs::remove_dir_all(&credentials).await {
            Ok(()) => report.removed.push(credentials),
            Err(err) => report.kept.push((credentials, format!("could not be removed: {err}"))),
        }
    }

    let benches = data.join("worktrees");
    for repository in children(&benches).await {
        for bench in children(&repository).await {
            let agent = bench
                .file_name()
                .and_then(|name| name.to_str())
                .and_then(|name| name.parse::<AgentId>().ok());
            bench_away(&bench, agent.and_then(&terminal_of), data, &mut report).await;
        }
        remove_if_empty(&repository).await;
    }
    remove_if_empty(&benches).await;

    let clones = data.join("repos");
    for clone in children(&clones).await {
        clone_away(&clone, &mut report).await;
    }
    remove_if_empty(&clones).await;

    for path in &report.removed {
        tracing::info!(path = %path.display(), "removed what repositories left behind");
    }
    for (from, to) in &report.moved {
        tracing::info!(from = %from.display(), to = %to.display(), "moved unsaved work into its agent's terminal");
    }
    for (path, why) in &report.kept {
        tracing::warn!(path = %path.display(), why, "kept what repositories left behind");
    }
    report
}

async fn bench_away(bench: &Path, terminal: Option<PathBuf>, data: &Path, report: &mut Report) {
    let Some(common) = git(bench, &["rev-parse", "--path-format=absolute", "--git-common-dir"])
        .await
        .map(PathBuf::from)
    else {
        // Not a work tree any more: the repository it was linked to is gone.
        // Its files are still somebody's.
        match terminal {
            Some(terminal) => {
                let to = free_name(&terminal, "recovered").await;
                match tokio::fs::rename(bench, &to).await {
                    Ok(()) => report.moved.push((bench.to_path_buf(), to)),
                    Err(err) => report
                        .kept
                        .push((bench.to_path_buf(), format!("could not be moved: {err}"))),
                }
            }
            None => report.kept.push((
                bench.to_path_buf(),
                "the repository it was a work tree of is gone, and so is its agent".into(),
            )),
        }
        return;
    };

    let dirty = git(bench, &["status", "--porcelain"]).await.is_none_or(|out| !out.is_empty());
    // The commit is safe if the repository it came from keeps it: an
    // operator's own checkout keeps every branch a work tree made, and a clone
    // this workspace made keeps it only until the clone goes, so there it has
    // to be on a remote.
    let theirs = !common.starts_with(data);
    let pushed = git(bench, &["branch", "-r", "--contains", "HEAD"])
        .await
        .is_some_and(|out| !out.is_empty());
    let git_dir = common.to_string_lossy().into_owned();
    let path = bench.to_string_lossy().into_owned();

    if !dirty && (theirs || pushed) {
        match git_at(&git_dir, &["worktree", "remove", &path]).await {
            Some(_) => report.removed.push(bench.to_path_buf()),
            None => report.kept.push((bench.to_path_buf(), "git would not remove it".into())),
        }
        return;
    }
    let Some(terminal) = terminal else {
        report.kept.push((
            bench.to_path_buf(),
            "it holds work nothing else has, and the agent it belonged to is gone".into(),
        ));
        return;
    };
    let name = common
        .parent()
        .and_then(|repository| repository.file_name())
        .and_then(|name| name.to_str())
        .unwrap_or("recovered")
        .to_string();
    let to = free_name(&terminal, &name).await;
    let target = to.to_string_lossy().into_owned();
    match git_at(&git_dir, &["worktree", "move", &path, &target]).await {
        Some(_) => report.moved.push((bench.to_path_buf(), to)),
        None => report.kept.push((bench.to_path_buf(), "git would not move it".into())),
    }
}

async fn clone_away(clone: &Path, report: &mut Report) {
    let Some(trees) = git(clone, &["worktree", "list", "--porcelain"]).await else {
        report.kept.push((clone.to_path_buf(), "it is not a git repository".into()));
        return;
    };
    let linked =
        trees.lines().filter(|line| line.starts_with("worktree ")).count().saturating_sub(1);
    if linked > 0 {
        report.kept.push((
            clone.to_path_buf(),
            format!("{linked} work tree(s) moved into agents' terminals still use it"),
        ));
        return;
    }
    if git(clone, &["status", "--porcelain"]).await.is_none_or(|out| !out.is_empty()) {
        report.kept.push((clone.to_path_buf(), "it has uncommitted changes".into()));
        return;
    }
    let unpushed = git(clone, &["log", "--branches", "--not", "--remotes", "--oneline", "-1"])
        .await
        .is_none_or(|out| !out.is_empty());
    if unpushed {
        report.kept.push((clone.to_path_buf(), "it has commits no remote has".into()));
        return;
    }
    match tokio::fs::remove_dir_all(clone).await {
        Ok(()) => report.removed.push(clone.to_path_buf()),
        Err(err) => report.kept.push((clone.to_path_buf(), format!("could not be removed: {err}"))),
    }
}

/// A name under `terminal` nothing is using yet: `name`, then `name-2`.
async fn free_name(terminal: &Path, name: &str) -> PathBuf {
    let mut at = terminal.join(name);
    let mut n = 2;
    while tokio::fs::try_exists(&at).await.unwrap_or(true) {
        at = terminal.join(format!("{name}-{n}"));
        n += 1;
    }
    at
}

async fn children(dir: &Path) -> Vec<PathBuf> {
    let mut found = Vec::new();
    let Ok(mut entries) = tokio::fs::read_dir(dir).await else {
        return found;
    };
    while let Ok(Some(entry)) = entries.next_entry().await {
        if entry.file_type().await.is_ok_and(|kind| kind.is_dir()) {
            found.push(entry.path());
        }
    }
    found.sort();
    found
}

async fn remove_if_empty(dir: &Path) {
    // `remove_dir` refuses a directory with anything in it, which is the test.
    let _ = tokio::fs::remove_dir(dir).await;
}

/// `git -C <dir> ...`, its trimmed stdout, or `None` when it failed.
async fn git(dir: &Path, args: &[&str]) -> Option<String> {
    run(std::iter::once("-C").chain(std::iter::once(dir.to_str()?)).chain(args.iter().copied()))
        .await
}

/// `git --git-dir=<dir> ...`, for the commands that act on a work tree from
/// the repository that owns it.
async fn git_at(git_dir: &str, args: &[&str]) -> Option<String> {
    let flag = format!("--git-dir={git_dir}");
    run(std::iter::once(flag.as_str()).chain(args.iter().copied())).await
}

async fn run<'a>(args: impl Iterator<Item = &'a str>) -> Option<String> {
    let mut command = tokio::process::Command::new("git");
    command
        .args(args)
        .env("GIT_TERMINAL_PROMPT", "0")
        .stdin(std::process::Stdio::null())
        .kill_on_drop(true);
    let done = tokio::time::timeout(Duration::from_secs(30), command.output()).await.ok()?.ok()?;
    done.status.success().then(|| String::from_utf8_lossy(&done.stdout).trim().to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sh(dir: &Path, args: &[&str]) {
        let done = std::process::Command::new("git")
            .arg("-C")
            .arg(dir)
            .args(["-c", "user.email=t@t", "-c", "user.name=t", "-c", "commit.gpgsign=false"])
            .args(args)
            .output()
            .unwrap();
        assert!(done.status.success(), "git {args:?}: {}", String::from_utf8_lossy(&done.stderr));
    }

    struct Scene {
        root: PathBuf,
        data: PathBuf,
        config: PathBuf,
        terminals: PathBuf,
    }

    fn scene(name: &str) -> Scene {
        let root =
            std::env::temp_dir().join(format!("guac-leftovers-{name}-{}", uuid::Uuid::new_v4()));
        let data = root.join("data");
        let config = root.join("config");
        let terminals = data.join("terminals");
        for dir in [&data, &config, &terminals] {
            std::fs::create_dir_all(dir).unwrap();
        }
        let root = std::fs::canonicalize(&root).unwrap();
        Scene {
            data: root.join("data"),
            config: root.join("config"),
            terminals: root.join("data/terminals"),
            root,
        }
    }

    /// A repository with one commit, where `at` says.
    fn repository(at: &Path) {
        std::fs::create_dir_all(at).unwrap();
        sh(at, &["init", "-q", "-b", "main"]);
        sh(at, &["commit", "-q", "--allow-empty", "-m", "one"]);
    }

    fn bench(scene: &Scene, main: &Path, agent: AgentId, branch: &str) -> PathBuf {
        let bench = scene.data.join("worktrees").join("r1").join(agent.to_string());
        std::fs::create_dir_all(bench.parent().unwrap()).unwrap();
        sh(main, &["worktree", "add", "-q", "-b", branch, bench.to_str().unwrap()]);
        bench
    }

    fn living(scene: &Scene, agents: &[AgentId]) -> impl Fn(AgentId) -> Option<PathBuf> {
        let terminals = scene.terminals.clone();
        let agents = agents.to_vec();
        move |agent| {
            agents.contains(&agent).then(|| {
                let dir = terminals.join(agent.to_string());
                std::fs::create_dir_all(&dir).unwrap();
                dir
            })
        }
    }

    #[tokio::test]
    async fn a_clean_work_tree_of_the_operators_own_checkout_goes_and_its_branch_stays() {
        let scene = scene("clean");
        let theirs = scene.root.join("operator/site");
        repository(&theirs);
        let agent = AgentId::new();
        let bench = bench(&scene, &theirs, agent, "publish/friday");

        let report = put_away(&scene.data, &scene.config, living(&scene, &[agent])).await;
        assert_eq!(report.removed, vec![bench.clone()]);
        assert!(!bench.exists());
        assert!(!scene.data.join("worktrees").exists(), "the empty directories go too");
        // The branch the work tree was on is the operator's, and stays theirs.
        let branches = git(&theirs, &["branch", "--list", "publish/friday"]).await.unwrap();
        assert!(branches.contains("publish/friday"), "{branches}");
        // And their repository no longer lists a work tree that is gone.
        let trees = git(&theirs, &["worktree", "list"]).await.unwrap();
        assert_eq!(trees.lines().count(), 1, "{trees}");
        let _ = std::fs::remove_dir_all(&scene.root);
    }

    #[tokio::test]
    async fn uncommitted_work_moves_into_its_agents_terminal_still_linked() {
        let scene = scene("dirty");
        let theirs = scene.root.join("operator/site");
        repository(&theirs);
        let agent = AgentId::new();
        let bench = bench(&scene, &theirs, agent, "wip");
        std::fs::write(bench.join("half.txt"), "a half-finished thought").unwrap();

        let report = put_away(&scene.data, &scene.config, living(&scene, &[agent])).await;
        let to = scene.terminals.join(agent.to_string()).join("site");
        assert_eq!(report.moved, vec![(bench.clone(), to.clone())]);
        assert_eq!(
            std::fs::read_to_string(to.join("half.txt")).unwrap(),
            "a half-finished thought"
        );
        let status = git(&to, &["status", "--short", "--branch"]).await.unwrap();
        assert!(status.contains("## wip"), "still the work tree it was: {status}");
        let _ = std::fs::remove_dir_all(&scene.root);
    }

    #[tokio::test]
    async fn work_nothing_else_has_is_kept_when_its_agent_is_gone() {
        let scene = scene("orphan");
        let theirs = scene.root.join("operator/site");
        repository(&theirs);
        let gone = AgentId::new();
        let bench = bench(&scene, &theirs, gone, "wip");
        std::fs::write(bench.join("half.txt"), "mine").unwrap();

        let report = put_away(&scene.data, &scene.config, living(&scene, &[])).await;
        assert!(bench.join("half.txt").exists());
        assert_eq!(report.kept.len(), 1);
        assert!(report.kept[0].1.contains("agent it belonged to is gone"), "{:?}", report.kept);
        let _ = std::fs::remove_dir_all(&scene.root);
    }

    #[tokio::test]
    async fn a_clone_goes_once_everything_in_it_is_pushed_and_stays_until_then() {
        let scene = scene("clones");
        let remote = scene.root.join("remote.git");
        std::fs::create_dir_all(&remote).unwrap();
        sh(&remote, &["init", "-q", "--bare", "-b", "main"]);
        let seed = scene.root.join("seed");
        repository(&seed);
        sh(&seed, &["push", "-q", remote.to_str().unwrap(), "main"]);

        let pushed = scene.data.join("repos/pushed");
        let ahead = scene.data.join("repos/ahead");
        for clone in [&pushed, &ahead] {
            std::fs::create_dir_all(clone.parent().unwrap()).unwrap();
            sh(&scene.root, &["clone", "-q", remote.to_str().unwrap(), clone.to_str().unwrap()]);
        }
        sh(&ahead, &["commit", "-q", "--allow-empty", "-m", "only here"]);

        let report = put_away(&scene.data, &scene.config, living(&scene, &[])).await;
        assert_eq!(report.removed, vec![pushed.clone()]);
        assert!(ahead.exists());
        assert!(report
            .kept
            .iter()
            .any(|(path, why)| path == &ahead && why.contains("no remote has")));
        let _ = std::fs::remove_dir_all(&scene.root);
    }

    #[tokio::test]
    async fn a_clones_work_tree_is_removed_only_when_its_commit_is_on_a_remote() {
        let scene = scene("clone-bench");
        let remote = scene.root.join("remote.git");
        std::fs::create_dir_all(&remote).unwrap();
        sh(&remote, &["init", "-q", "--bare", "-b", "main"]);
        let seed = scene.root.join("seed");
        repository(&seed);
        sh(&seed, &["push", "-q", remote.to_str().unwrap(), "main"]);
        let clone = scene.data.join("repos/site");
        std::fs::create_dir_all(clone.parent().unwrap()).unwrap();
        sh(&scene.root, &["clone", "-q", remote.to_str().unwrap(), clone.to_str().unwrap()]);

        let agent = AgentId::new();
        let bench = bench(&scene, &clone, agent, "feature");
        sh(&bench, &["commit", "-q", "--allow-empty", "-m", "unpushed"]);

        // Committed but on no remote, in a clone that would take it with it:
        // moved rather than removed, and the clone stays because it backs it.
        let report = put_away(&scene.data, &scene.config, living(&scene, &[agent])).await;
        let to = scene.terminals.join(agent.to_string()).join("site");
        assert_eq!(report.moved, vec![(bench, to.clone())]);
        assert!(clone.exists());
        assert!(report
            .kept
            .iter()
            .any(|(path, why)| path == &clone && why.contains("still use it")));
        let _ = std::fs::remove_dir_all(&scene.root);
    }

    #[tokio::test]
    async fn stored_tokens_always_go_and_a_second_pass_does_nothing() {
        let scene = scene("tokens");
        let tokens = scene.config.join("repo-credentials");
        std::fs::create_dir_all(&tokens).unwrap();
        std::fs::write(tokens.join("clone-1"), "https://x-access-token:ghs_secret@github.com")
            .unwrap();

        let first = put_away(&scene.data, &scene.config, living(&scene, &[])).await;
        assert_eq!(first.removed, vec![tokens.clone()]);
        assert!(!tokens.exists());
        let second = put_away(&scene.data, &scene.config, living(&scene, &[])).await;
        assert_eq!(second, Report::default());
        let _ = std::fs::remove_dir_all(&scene.root);
    }
}
