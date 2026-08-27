//! Whether a directory is one an agent may be given.
//!
//! One question, asked by running git rather than by looking for a `.git`
//! entry. A work tree that is a linked worktree, a submodule, or a checkout
//! whose `.git` is a file rather than a directory all answer correctly to git
//! and all fool a directory check, and each of those is somebody's ordinary
//! setup rather than an exotic case. Asking git also answers a second question
//! for free: whether git is installed at all, which whatever works in the
//! directory later is going to need.
//!
//! This is where a repository stops being a string. Everything above it in
//! [`crate::domain::repository`] is the shape and the rules that need no disk;
//! this is the disk.
//!
//! ## The directory is on this machine
//!
//! Not on an agent's sandbox. The operator picks a directory they already have,
//! with work already in it, and that is the whole point of the feature: an
//! agent that can only see a fresh clone cannot see the branch you are on, the
//! change you have not committed, or the toolchain your repository actually
//! builds with.
//!
//! What makes that defensible is not this check. It is the three things around
//! it: the operator picks the path and no model ever supplies one, git is the
//! undo, and the process that works in there is confined to it. This function
//! is the first of the three and the cheapest, and it must never be described
//! as the boundary.

use std::path::Path;

#[derive(Debug, thiserror::Error, PartialEq)]
pub enum RepoError {
    #[error("`{0}` is not a directory on this machine; pick one that exists")]
    NotADirectory(String),
    #[error(
        "git is not installed, or is not on this app's PATH. Guaca needs it to work in a \
         repository at all: install the Xcode command line tools with `xcode-select --install`, \
         or install git, and link the directory again"
    )]
    GitMissing,
    #[error(
        "`{0}` is not a git repository. An agent works there only because git can undo it, so \
         run `git init` in it first, or pick a directory that already has a repository"
    )]
    NotARepository(String),
    #[error(
        "`{linked}` is inside the repository at `{root}` rather than being its root. Link \
         `{root}` instead: what an agent may write to and what git can undo have to be the same \
         directory. To keep an agent to part of it, say so in the repository's note"
    )]
    NotTheRoot { linked: String, root: String },
    #[error("`{path}` could not be read ({reason})")]
    Unreadable { path: String, reason: String },
}

/// The canonical path of the work tree this directory is the root of.
///
/// Canonical because the answer is stored and compared: a path reached through
/// a symlink and the same path reached directly are one directory, and two rows
/// for it would be two reaches over one tree with nothing saying which applied.
pub async fn verify(path: &str) -> Result<String, RepoError> {
    let given = Path::new(path);

    let canonical = tokio::fs::canonicalize(given).await.map_err(|err| match err.kind() {
        std::io::ErrorKind::NotFound => RepoError::NotADirectory(path.to_string()),
        _ => RepoError::Unreadable { path: path.to_string(), reason: err.to_string() },
    })?;
    if !tokio::fs::metadata(&canonical).await.map(|meta| meta.is_dir()).unwrap_or(false) {
        return Err(RepoError::NotADirectory(path.to_string()));
    }

    let output = tokio::process::Command::new("git")
        .arg("-C")
        .arg(&canonical)
        .args(["rev-parse", "--show-toplevel"])
        .output()
        .await
        .map_err(|err| match err.kind() {
            std::io::ErrorKind::NotFound => RepoError::GitMissing,
            _ => RepoError::Unreadable { path: path.to_string(), reason: err.to_string() },
        })?;

    let shown = String::from_utf8_lossy(&output.stdout).trim().to_string();
    // A bare repository exits zero and prints nothing: there is no work tree,
    // so there is nothing to open and nothing to edit. Reported as not a
    // repository rather than as an empty answer, because that is what it is
    // from where the operator is standing.
    if !output.status.success() || shown.is_empty() {
        return Err(RepoError::NotARepository(display(&canonical)));
    }

    // Canonicalized on both sides before they are compared. On macOS a
    // directory under `/tmp` or `/var` is reached through a symlink, and git
    // resolves it while the operator's own path does not: comparing the two as
    // typed reports the root of a repository as being inside itself.
    let root = tokio::fs::canonicalize(&shown)
        .await
        .map_err(|err| RepoError::Unreadable { path: shown.clone(), reason: err.to_string() })?;

    if root != canonical {
        return Err(RepoError::NotTheRoot { linked: display(&canonical), root: display(&root) });
    }

    Ok(display(&canonical))
}

/// A path as the operator would read it back.
///
/// Lossy on purpose: a path this app cannot spell is one the operator cannot
/// act on either, and refusing to draw it would leave them with an error naming
/// nothing.
fn display(path: &Path) -> String {
    path.to_string_lossy().to_string()
}

/// What a repository is doing right now, as the rail draws it.
///
/// Read rather than remembered. Nothing here is stored: the answer changes when
/// the operator commits, pulls or opens a pull request, and none of those go
/// through Guaca. A cached copy would be wrong exactly when somebody looked.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RepoStatus {
    /// The branch name, or `HEAD` detached at a commit.
    pub branch: String,
    /// Whether the work tree is at a commit rather than on a branch. Drawn
    /// differently because it is a state somebody has to get out of, not a
    /// place to work from.
    pub detached: bool,
    /// Paths that differ from HEAD: modified, staged, untracked, unmerged.
    /// One number rather than four, because the rail has room for one and the
    /// question it answers is "is there uncommitted work here".
    pub dirty: u32,
    /// Commits on this branch the upstream does not have, and the reverse.
    /// Both zero when there is no upstream, which is not the same as being in
    /// sync and is why `upstream` is separate.
    pub ahead: u32,
    pub behind: u32,
    pub upstream: bool,
    /// Open pull requests, when `gh` is installed and signed in.
    ///
    /// `None` is not zero and must never be drawn as one. It means the question
    /// could not be asked: no `gh`, not authenticated, no remote, or the
    /// network is down. Zero means somebody asked and there are none.
    pub pull_requests: Option<u32>,
}

/// Everything the rail says about one repository, in two calls.
///
/// The git half is local and costs nothing. The `gh` half is a network round
/// trip, and it is allowed to fail without taking the rest with it: a
/// repository with no remote is still a repository somebody is working in, and
/// reporting the branch as unknown because GitHub was unreachable would be the
/// tail wagging the dog.
pub async fn status(path: &str) -> Option<RepoStatus> {
    let (git, prs) = tokio::join!(work_tree_status(path), open_pull_requests(path));
    let mut status = git?;
    status.pull_requests = prs;
    Some(status)
}

async fn work_tree_status(path: &str) -> Option<RepoStatus> {
    // One invocation for the branch, the tracking counts and the changed
    // paths. `--porcelain=v2` is the only status format documented as stable
    // for machines; v1 has to be parsed by column position and says nothing
    // about how far ahead the branch is.
    let out = tokio::process::Command::new("git")
        .arg("-C")
        .arg(path)
        .args(["status", "--porcelain=v2", "--branch"])
        .output()
        .await
        .ok()?;
    if !out.status.success() {
        return None;
    }

    let text = String::from_utf8_lossy(&out.stdout);
    let mut status = RepoStatus {
        branch: "HEAD".to_string(),
        detached: false,
        dirty: 0,
        ahead: 0,
        behind: 0,
        upstream: false,
        pull_requests: None,
    };

    for line in text.lines() {
        let Some(header) = line.strip_prefix("# ") else {
            // Everything that is not a header is a path that differs from
            // HEAD: `1` and `2` are tracked changes, `u` unmerged, `?`
            // untracked, `!` ignored and only ever present when asked for.
            if line.starts_with(['1', '2', 'u', '?']) {
                status.dirty += 1;
            }
            continue;
        };
        if let Some(head) = header.strip_prefix("branch.head ") {
            // Git spells a detached HEAD as the literal `(detached)` here, so
            // the parenthesis is the signal rather than a missing value.
            status.detached = head == "(detached)";
            if !status.detached {
                status.branch = head.to_string();
            }
        } else if header.starts_with("branch.upstream ") {
            status.upstream = true;
        } else if let Some(ab) = header.strip_prefix("branch.ab ") {
            // `+2 -1`. Absent entirely when there is no upstream, which is why
            // zero and zero cannot be read as "in sync" on its own.
            for part in ab.split_whitespace() {
                let (sign, count) = part.split_at(1);
                let Ok(count) = count.parse::<u32>() else { continue };
                match sign {
                    "+" => status.ahead = count,
                    "-" => status.behind = count,
                    _ => {}
                }
            }
        } else if header.starts_with("branch.oid ") && status.branch == "HEAD" {
            // Only reached on a detached HEAD, where the short sha is the only
            // name the state has.
            if let Some(sha) = header.split_whitespace().nth(1) {
                status.branch = sha.chars().take(7).collect();
            }
        }
    }

    Some(status)
}

/// How many pull requests are open, asked of `gh`.
///
/// Every failure is the same answer, and it is `None` rather than zero: `gh`
/// not installed, not signed in, no remote, a repository GitHub has never heard
/// of, and a network that is down all mean the question could not be asked.
/// Drawn as zero, each of them would say there is nothing waiting for review,
/// which is a claim this app has no basis for.
async fn open_pull_requests(path: &str) -> Option<u32> {
    let out = tokio::process::Command::new("gh")
        .current_dir(path)
        .args(["pr", "list", "--state", "open", "--limit", "100", "--json", "number"])
        .output()
        .await
        .ok()?;
    if !out.status.success() {
        return None;
    }
    // The count rather than `gh`'s own `--jq`, which needs its embedded jq and
    // fails differently when the expression is wrong than when the query is.
    let rows: Vec<serde_json::Value> = serde_json::from_slice(&out.stdout).ok()?;
    Some(rows.len() as u32)
}

/// Where a coding job is about to start, and what that means for the branch.
///
/// Not [`RepoStatus`], and deliberately not folded into it. That one is drawn
/// on the rail every thirty seconds and carries what fits on one line. This is
/// read once, by a program that is about to write to the tree, and it exists to
/// settle one question before the first edit: which branch this work starts on.
///
/// It has to be settled *before* the brief, because a harness handed a brief
/// starts editing where it is standing. Nothing prompts it to look at the
/// branch, and nothing else in the app will: a job that ends on the branch it
/// made leaves the next job starting there, which is how a work tree ends up
/// sitting on a feature branch that landed a month ago.
///
/// The alternative was a standing rule about what a job leaves behind, and it
/// does not hold. A job killed at the ceiling never runs its cleanup, and a job
/// that opened a pull request should still be on its branch. Cleanup at the end
/// is a step that sometimes does not happen; the footing at the start always
/// does.
#[derive(Debug, Clone, PartialEq)]
pub struct Footing {
    /// What git says about the tree. [`RepoStatus::pull_requests`] on it is not
    /// read here: the count belongs to the rail, and what a job needs is the
    /// one pull request whose head is this branch.
    pub tree: RepoStatus,
    /// The branch new work starts from, as this repository names it.
    ///
    /// `None` is a repository with no `origin/HEAD`, no `main` and no `master`,
    /// which is somebody's own convention rather than a broken repository. The
    /// brief says so and asks the harness to decide, because a name invented
    /// here is a branch that does not exist.
    pub default_branch: Option<String>,
    /// Whether HEAD is already contained in the default branch.
    ///
    /// True on the default branch itself whenever it has nothing unpushed,
    /// which is why [`Footing::rule`] asks where you are standing before it
    /// asks this.
    pub merged: bool,
    /// An open pull request whose head is this branch.
    pub pull_request: Option<u32>,
    /// Whether nothing has been committed here yet.
    ///
    /// A fresh `git init` is an ordinary thing to link, and it is the one state
    /// where the branch the tree reports is a branch that does not exist: git
    /// names an unborn HEAD after the branch the first commit will create. Left
    /// unsaid, the preamble reads as a contradiction, on branch `main` and
    /// there is no `main`, and a model handed a contradiction resolves it by
    /// guessing.
    pub unborn: bool,
}

impl Footing {
    /// What the harness reads in front of the brief.
    ///
    /// Facts, then the one rule they resolve to, and both halves are
    /// load-bearing. Facts alone are not enough: a model handed a branch name
    /// and a count decides for itself what to do with them, and the decision it
    /// makes silently is to carry on where it is standing. A rule alone cannot
    /// be written safely, which is the sharper half: *start from the default
    /// branch* over uncommitted work destroys it, and this is the operator's
    /// own machine rather than a sandbox. The facts are what make the rule
    /// conditional, and the rule is what makes the facts act on anything.
    pub fn brief(&self) -> String {
        let mut out =
            String::from("Where you are starting from, read from this work tree just now:\n");

        if self.tree.detached {
            out.push_str(&format!(
                "- HEAD is detached at `{}`, not on a branch.\n",
                self.tree.branch
            ));
        } else {
            out.push_str(&format!("- On branch `{}`.\n", self.tree.branch));
        }
        if self.unborn {
            out.push_str(&format!(
                "- No commits yet: `{}` starts existing when something is committed to it.\n",
                self.tree.branch
            ));
        }

        out.push_str(&match self.tree.dirty {
            0 => "- The tree is clean.\n".to_string(),
            1 => "- 1 file is changed or untracked and not committed.\n".to_string(),
            n => format!("- {n} files are changed or untracked and not committed.\n"),
        });

        if !self.tree.upstream {
            out.push_str("- This branch tracks nothing on a remote.\n");
        } else if self.tree.ahead == 0 && self.tree.behind == 0 {
            out.push_str("- Level with its upstream.\n");
        } else {
            out.push_str(&format!(
                "- {} not pushed, {} not pulled.\n",
                commits(self.tree.ahead),
                commits(self.tree.behind)
            ));
        }

        if let Some(default) = &self.default_branch {
            out.push_str(&format!("- The default branch here is `{default}`.\n"));
        }
        if let Some(number) = self.pull_request {
            out.push_str(&format!("- Pull request #{number} is open for this branch.\n"));
        }

        // Said every time, including when the counts are zero, because zero is
        // where it misleads: every number above and the merge test under it are
        // measured against the last fetch, and a branch that landed upstream an
        // hour ago reads here as work in flight. Fetching on the operator's
        // behalf is not this app's call. The harness is the thing standing in
        // the directory with a shell.
        out.push_str(
            "\nThese are measured against the last fetch rather than against the remote as it \
             is now, so fetch before you rely on them.\n\n",
        );

        out.push_str(&self.rule());
        // A brief and a preamble are both prose, and a harness given the two
        // run together answers the preamble. One sentence is cheaper than
        // finding out which it did.
        out.push_str("\n\nWhat you have been asked to do follows.");
        out
    }

    /// The one thing to do about all of that.
    fn rule(&self) -> String {
        // Uncommitted work is checked first and overrides every other case.
        // The operator works in this directory too, and a rule that switched
        // branches to be tidy would take an afternoon of theirs with it. This
        // is the reason the facts are here at all: the same rule written
        // unconditionally is the one that does the damage.
        if self.tree.dirty > 0 {
            return "There is uncommitted work in this tree and it may not be yours: this is the \
                    operator's own machine and they work here too. Do not switch branches, \
                    stash, reset or clean. Work from where you are, keep anything you did not \
                    change out of your commits, and say what you found already changed. If this \
                    brief cannot be done from here at all, stop and say so rather than clearing \
                    the tree."
                .to_string();
        }

        // After the dirty case, because a fresh `git init` with the operator's
        // files already in it is both, and the rule that must not be lost is
        // the one about their work. Before everything else, because there is no
        // branch to go back to and no history to measure against.
        if self.unborn {
            return format!(
                "This repository has no commits yet, so `{}` is not a branch you can leave and \
                 come back to, and there is no history here to build on. Commit your work on it \
                 rather than looking for another branch.",
                self.tree.branch
            );
        }

        // Before the default branch is looked at, because a detached HEAD is a
        // state to get out of whether or not this repository publishes one, and
        // the way out is the same either way.
        if self.tree.detached {
            let onto = match self.default_branch.as_deref() {
                Some(default) => format!("a new one off `{default}` for new work"),
                None => "a new one for new work".to_string(),
            };
            return format!(
                "HEAD is detached, so a commit made here belongs to no branch and is easy to \
                 lose. Put yourself on a branch before you change anything: {onto}, or the \
                 branch this commit already belongs to if you are continuing something."
            );
        }

        let Some(default) = self.default_branch.as_deref() else {
            return "Nothing here names a default branch: there is no `origin/HEAD`, no `main` \
                    and no `master`. Decide where this work belongs yourself, and say which \
                    branch you put it on."
                .to_string();
        };

        if self.tree.branch == default {
            return format!(
                "You are on `{default}`, the default branch. Bring it up to date before you \
                 start. Where the work should land is the brief's to say; if it does not say, \
                 put it on a branch of its own rather than committing here."
            );
        }

        if self.merged {
            return format!(
                "`{}` is already contained in `{default}`: its work has landed and this is not \
                 where new work goes. Unless the brief is explicitly about this branch, start \
                 from `{default}` brought up to date and branch from there.",
                self.tree.branch
            );
        }

        let mut rule = format!(
            "`{}` has commits `{default}` does not, so it is work in flight. Continue on it if \
             this brief is about that work. If it is not, start from `{default}` brought up to \
             date instead, so two unrelated changes do not arrive on one branch.",
            self.tree.branch
        );
        if let Some(number) = self.pull_request {
            rule.push_str(&format!(
                " Pull request #{number} is open for it: push to this branch rather than opening \
                 a second one."
            ));
        }
        rule
    }
}

/// A count of commits that reads as English at one.
fn commits(count: u32) -> String {
    match count {
        1 => "1 commit".to_string(),
        n => format!("{n} commits"),
    }
}

/// Everything a job is told about the tree it is starting in.
///
/// `None` is the answer [`status`] gives for the same reason: this is not a
/// directory git will talk about, which by the time a job starts means one that
/// has been moved or deleted since it was linked. The job goes ahead without
/// the preamble rather than being refused for it. The harness is standing in
/// the directory and will find that out faster and say it better.
pub async fn footing(path: &str) -> Option<Footing> {
    let (tree, default) = tokio::join!(work_tree_status(path), default_branch(path));
    let tree = tree?;

    let (merged, pull_request, unborn) = tokio::join!(
        async {
            match &default {
                Some((_, reference)) => contained_in(path, reference).await,
                None => false,
            }
        },
        async {
            // Per branch rather than per repository, and only when there is a
            // branch to ask about. The count the rail draws answers "is
            // anything waiting for review"; what a job needs is whether the
            // branch it is standing on already has one, which is the difference
            // between pushing and opening a second.
            if tree.detached {
                None
            } else {
                open_pull_request(path, &tree.branch).await
            }
        },
        // An unborn HEAD is the one thing `git status` reports by naming a
        // branch that does not exist yet, and this is the cheapest question
        // that tells the two apart.
        async { !exists(path, "HEAD").await }
    );

    Some(Footing {
        tree,
        default_branch: default.map(|(name, _)| name),
        merged,
        pull_request,
        unborn,
    })
}

/// The branch new work starts from, by name and by the ref to measure against.
///
/// Two answers because they are two different things. The name is prose the
/// harness reads and types. The ref is what the merge test runs against, and it
/// is `origin/main` rather than `main`: a branch merged upstream has landed
/// whether or not the local copy was ever pulled, and a local `main` nobody has
/// updated in a month calls every landed branch work in flight.
///
/// `origin/HEAD` first, because it is the repository's own answer. The two
/// names after it are a guess, reached only when nothing published one.
async fn default_branch(path: &str) -> Option<(String, String)> {
    let published = tokio::process::Command::new("git")
        .arg("-C")
        .arg(path)
        .args(["symbolic-ref", "--quiet", "--short", "refs/remotes/origin/HEAD"])
        .output()
        .await
        .ok();
    if let Some(out) = published {
        if out.status.success() {
            let shown = String::from_utf8_lossy(&out.stdout).trim().to_string();
            // Always under `origin/`, because that is the ref that was asked
            // for. Stripped by prefix rather than by the last separator, so a
            // default branch called `release/next` keeps its own name.
            if let Some(name) = shown.strip_prefix("origin/") {
                if !name.is_empty() {
                    return Some((name.to_string(), format!("refs/remotes/origin/{name}")));
                }
            }
        }
    }

    for name in ["main", "master"] {
        for reference in [format!("refs/remotes/origin/{name}"), format!("refs/heads/{name}")] {
            if exists(path, &reference).await {
                return Some((name.to_string(), reference));
            }
        }
    }
    None
}

async fn exists(path: &str, reference: &str) -> bool {
    tokio::process::Command::new("git")
        .arg("-C")
        .arg(path)
        .args(["rev-parse", "--verify", "--quiet", reference])
        .output()
        .await
        .map(|out| out.status.success())
        .unwrap_or(false)
}

/// Whether HEAD is already contained in this ref.
///
/// `merge-base --is-ancestor` rather than `branch --merged`, which answers with
/// a list of names that would then have to be matched against the one this tree
/// is on. Anything that is not a clean yes is a no: an unborn HEAD, a ref that
/// has gone and a git that could not be run all mean this build cannot claim
/// the work has landed, and claiming it wrongly sends a job away from the
/// branch its work is on.
async fn contained_in(path: &str, reference: &str) -> bool {
    tokio::process::Command::new("git")
        .arg("-C")
        .arg(path)
        .args(["merge-base", "--is-ancestor", "HEAD", reference])
        .output()
        .await
        .map(|out| out.status.success())
        .unwrap_or(false)
}

/// The open pull request whose head is this branch, asked of `gh`.
///
/// The same `None` discipline [`open_pull_requests`] has, for the same reason:
/// every way of not knowing is one answer and it is not zero. The first row is
/// taken because a branch carrying two open pull requests is somebody's own
/// arrangement rather than something a job should reason about.
async fn open_pull_request(path: &str, branch: &str) -> Option<u32> {
    let out = tokio::process::Command::new("gh")
        .current_dir(path)
        .args([
            "pr", "list", "--state", "open", "--head", branch, "--limit", "1", "--json", "number",
        ])
        .output()
        .await
        .ok()?;
    if !out.status.success() {
        return None;
    }
    let rows: Vec<serde_json::Value> = serde_json::from_slice(&out.stdout).ok()?;
    rows.first()?.get("number")?.as_u64().map(|number| number as u32)
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::*;

    /// A real repository, made here rather than mocked. The whole value of this
    /// function is that it agrees with git, and a stub agrees with itself.
    async fn a_repository(name: &str) -> PathBuf {
        let root = std::env::temp_dir().join(format!("guac-repo-{name}-{}", std::process::id()));
        let _ = tokio::fs::remove_dir_all(&root).await;
        tokio::fs::create_dir_all(&root).await.unwrap();
        let done = tokio::process::Command::new("git")
            .arg("-C")
            .arg(&root)
            .arg("init")
            .output()
            .await
            .expect("git has to be installed to run this suite");
        assert!(done.status.success(), "git init failed: {done:?}");
        tokio::fs::canonicalize(&root).await.unwrap()
    }

    #[tokio::test]
    async fn a_work_tree_root_is_accepted_and_comes_back_canonical() {
        let root = a_repository("root").await;
        let verified = verify(root.to_str().unwrap()).await.unwrap();
        assert_eq!(verified, root.to_string_lossy());
        let _ = tokio::fs::remove_dir_all(&root).await;
    }

    #[tokio::test]
    async fn a_subdirectory_is_refused_and_names_the_root() {
        let root = a_repository("sub").await;
        let inner = root.join("packages").join("api");
        tokio::fs::create_dir_all(&inner).await.unwrap();

        let err = verify(inner.to_str().unwrap()).await.unwrap_err();
        match &err {
            RepoError::NotTheRoot { root: named, .. } => {
                assert_eq!(named, &root.to_string_lossy().to_string())
            }
            other => panic!("expected the root to be named, got {other:?}"),
        }
        // The refusal is the fix: an operator reading it knows which directory
        // to link without going and looking.
        assert!(err.to_string().contains(&root.to_string_lossy().to_string()));
        let _ = tokio::fs::remove_dir_all(&root).await;
    }

    #[tokio::test]
    async fn a_directory_with_no_repository_is_refused_with_the_command_that_fixes_it() {
        let plain = std::env::temp_dir().join(format!("guac-plain-{}", std::process::id()));
        let _ = tokio::fs::remove_dir_all(&plain).await;
        tokio::fs::create_dir_all(&plain).await.unwrap();

        let err = verify(plain.to_str().unwrap()).await.unwrap_err();
        assert!(matches!(err, RepoError::NotARepository(_)), "got {err:?}");
        assert!(err.to_string().contains("git init"), "the refusal has to say what to do");
        let _ = tokio::fs::remove_dir_all(&plain).await;
    }

    #[tokio::test]
    async fn a_path_that_is_not_there_is_refused_before_git_is_asked() {
        let err = verify("/no/such/directory/anywhere").await.unwrap_err();
        assert!(matches!(err, RepoError::NotADirectory(_)), "got {err:?}");
    }

    #[tokio::test]
    async fn a_file_is_not_a_directory() {
        let file = std::env::temp_dir().join(format!("guac-file-{}", std::process::id()));
        tokio::fs::write(&file, b"not a directory").await.unwrap();
        let err = verify(file.to_str().unwrap()).await.unwrap_err();
        assert!(matches!(err, RepoError::NotADirectory(_)), "got {err:?}");
        let _ = tokio::fs::remove_file(&file).await;
    }

    // ---- the footing a job starts from -----------------------------------

    /// A clean tree on a branch of its own, which every case below bends.
    fn standing(branch: &str) -> Footing {
        Footing {
            tree: RepoStatus {
                branch: branch.to_string(),
                detached: false,
                dirty: 0,
                ahead: 0,
                behind: 0,
                upstream: true,
                pull_requests: None,
            },
            default_branch: Some("main".to_string()),
            merged: false,
            pull_request: None,
            unborn: false,
        }
    }

    #[test]
    fn a_branch_that_has_already_landed_sends_the_job_back_to_the_default() {
        // The state this exists for: the last job opened a pull request, it was
        // merged, and nobody put the tree back. Left alone, the next brief is
        // built on top of a branch whose work is already in `main`.
        let landed = Footing { merged: true, ..standing("feature/rail-badges") };
        let brief = landed.brief();

        assert!(brief.contains("already contained in `main`"), "{brief}");
        assert!(brief.contains("start from `main`"), "{brief}");
        assert!(brief.contains("branch from there"), "{brief}");
    }

    #[test]
    fn uncommitted_work_is_never_cleared_to_get_to_a_branch() {
        // The reason the state is read at all rather than the rule being
        // written unconditionally. This is the operator's own machine: a job
        // that checks out `main` to be tidy takes their afternoon with it.
        let mut dirty = Footing { merged: true, ..standing("feature/x") };
        dirty.tree.dirty = 4;
        let brief = dirty.brief();

        assert!(brief.contains("4 files are changed or untracked"), "{brief}");
        assert!(brief.contains("Do not switch branches"), "{brief}");
        assert!(brief.contains("stash, reset or clean"), "{brief}");
        assert!(
            !brief.contains("start from `main`"),
            "a dirty tree is never sent to another branch: {brief}"
        );
    }

    #[test]
    fn a_branch_with_work_on_it_is_continued_and_its_pull_request_named() {
        let flight = Footing { pull_request: Some(41), ..standing("feature/plugins") };
        let brief = flight.brief();

        assert!(brief.contains("work in flight"), "{brief}");
        assert!(brief.contains("Continue on it"), "{brief}");
        // Both halves matter: the fact, so it knows one exists, and the
        // instruction, because the alternative it reaches for is a second one.
        assert!(brief.contains("Pull request #41 is open for this branch"), "{brief}");
        assert!(brief.contains("rather than opening a second one"), "{brief}");
    }

    #[test]
    fn on_the_default_branch_the_brief_decides_where_the_work_lands() {
        // Not an opinion about trunk-based development. Where the change goes
        // is already the brief's to say, and a standing rule that overrode it
        // would be a second answer to a question that has one.
        let brief = standing("main").brief();

        assert!(brief.contains("the default branch"), "{brief}");
        assert!(brief.contains("brief's to say"), "{brief}");
        assert!(brief.contains("branch of its own"), "{brief}");
    }

    #[test]
    fn a_detached_head_is_put_on_a_branch_before_anything_is_written() {
        let mut adrift = standing("x");
        adrift.tree.branch = "9f3c1ab".into();
        adrift.tree.detached = true;
        let brief = adrift.brief();

        assert!(brief.contains("HEAD is detached at `9f3c1ab`"), "{brief}");
        assert!(brief.contains("belongs to no branch"), "{brief}");
        assert!(brief.contains("off `main`"), "{brief}");
    }

    #[test]
    fn a_repository_that_names_no_default_branch_asks_rather_than_inventing_one() {
        // Somebody's own convention rather than a broken repository. A guessed
        // name here is a branch that does not exist, and a job that spends its
        // first minutes failing to check one out.
        let unnamed = Footing { default_branch: None, ..standing("trunk") };
        let brief = unnamed.brief();

        assert!(brief.contains("Nothing here names a default branch"), "{brief}");
        assert!(brief.contains("say which branch you put it on"), "{brief}");
        // Naming the two it looked for is the explanation. What it must not do
        // is send the job to one, which is a checkout that fails and a first
        // minute spent on it.
        assert!(!brief.contains("start from"), "sent to a branch that does not exist: {brief}");
        assert!(!brief.contains("off `main`"), "sent to a branch that does not exist: {brief}");
    }

    #[test]
    fn every_count_says_what_it_was_measured_against() {
        // Zero is where this misleads rather than where it is safe to drop: a
        // branch merged upstream an hour ago and never fetched reads here as
        // work in flight, level with its upstream.
        let brief = standing("feature/x").brief();
        assert!(brief.contains("last fetch"), "{brief}");
        assert!(brief.contains("What you have been asked to do follows"), "{brief}");
    }

    #[test]
    fn a_single_commit_and_a_single_file_read_as_english() {
        let mut one = standing("feature/x");
        one.tree.dirty = 1;
        one.tree.ahead = 1;
        one.tree.behind = 1;
        let brief = one.brief();
        assert!(brief.contains("1 file is changed"), "{brief}");
        assert!(brief.contains("1 commit not pushed, 1 commit not pulled"), "{brief}");
    }

    /// Git with an identity of its own, so the suite does not depend on what
    /// the machine running it has configured and does not try to sign.
    async fn run_git(root: &Path, args: &[&str]) {
        let done = tokio::process::Command::new("git")
            .arg("-C")
            .arg(root)
            .args([
                "-c",
                "user.name=guac",
                "-c",
                "user.email=guac@example.com",
                "-c",
                "commit.gpgsign=false",
            ])
            .args(args)
            .output()
            .await
            .expect("git has to be installed to run this suite");
        assert!(done.status.success(), "git {args:?} failed: {done:?}");
    }

    /// A repository with a commit in it, because a merge test needs history and
    /// a default branch needs a ref that exists.
    async fn a_repository_with_history(name: &str, branch: &str) -> PathBuf {
        let root = std::env::temp_dir().join(format!("guac-hist-{name}-{}", std::process::id()));
        let _ = tokio::fs::remove_dir_all(&root).await;
        tokio::fs::create_dir_all(&root).await.unwrap();
        run_git(&root, &["init", "-b", branch]).await;
        tokio::fs::write(root.join("a.txt"), b"one").await.unwrap();
        run_git(&root, &["add", "."]).await;
        run_git(&root, &["commit", "-m", "one"]).await;
        tokio::fs::canonicalize(&root).await.unwrap()
    }

    #[tokio::test]
    async fn a_branch_git_has_already_folded_in_is_read_as_landed() {
        // Against real git rather than a fixture, for the reason the rest of
        // this file is: the whole value of the answer is that it agrees with
        // git, and a stub agrees with itself.
        let root = a_repository_with_history("landed", "main").await;
        let path = root.to_str().unwrap();

        run_git(&root, &["checkout", "-b", "landed"]).await;
        let standing = footing(path).await.expect("a repository with history has a footing");
        assert_eq!(standing.default_branch.as_deref(), Some("main"));
        assert!(standing.merged, "a branch carrying nothing has landed by definition");
        assert!(standing.brief().contains("start from `main`"), "{}", standing.brief());

        tokio::fs::write(root.join("b.txt"), b"two").await.unwrap();
        run_git(&root, &["add", "."]).await;
        run_git(&root, &["commit", "-m", "two"]).await;
        let moved = footing(path).await.unwrap();
        assert!(!moved.merged, "a commit the default does not have is work in flight");
        assert!(moved.brief().contains("work in flight"), "{}", moved.brief());

        let _ = tokio::fs::remove_dir_all(&root).await;
    }

    #[tokio::test]
    async fn the_default_branch_is_the_one_the_remote_published() {
        // Named `trunk` on purpose: neither fallback can find it, so this
        // passing is `origin/HEAD` having been read and its prefix taken off
        // correctly rather than a guess that happened to be right.
        let root = a_repository_with_history("published", "trunk").await;
        let path = root.to_str().unwrap();
        let bare = std::env::temp_dir().join(format!("guac-bare-{}", std::process::id()));
        let _ = tokio::fs::remove_dir_all(&bare).await;

        run_git(&root, &["init", "--bare", bare.to_str().unwrap()]).await;
        run_git(&root, &["remote", "add", "origin", bare.to_str().unwrap()]).await;
        run_git(&root, &["push", "-u", "origin", "trunk"]).await;
        run_git(&root, &["remote", "set-head", "origin", "trunk"]).await;

        let standing = footing(path).await.unwrap();
        assert_eq!(standing.default_branch.as_deref(), Some("trunk"), "{standing:?}");
        assert!(standing.tree.upstream, "the push set one: {standing:?}");
        assert!(
            standing.brief().contains("You are on `trunk`, the default branch"),
            "{}",
            standing.brief()
        );

        let _ = tokio::fs::remove_dir_all(&root).await;
        let _ = tokio::fs::remove_dir_all(&bare).await;
    }

    #[tokio::test]
    async fn a_repository_with_no_commits_is_not_told_to_go_back_to_a_branch() {
        // A fresh `git init` is an ordinary thing to link. Git names an unborn
        // HEAD after the branch the first commit will create, so without this
        // the preamble says `on branch main` and `there is no main` two lines
        // apart, and sends the job looking for somewhere else to start.
        let root = std::env::temp_dir().join(format!("guac-unborn-{}", std::process::id()));
        let _ = tokio::fs::remove_dir_all(&root).await;
        tokio::fs::create_dir_all(&root).await.unwrap();
        run_git(&root, &["init", "-b", "main"]).await;

        let standing = footing(root.to_str().unwrap()).await.unwrap();
        assert!(standing.unborn, "nothing has been committed: {standing:?}");
        let brief = standing.brief();
        assert!(brief.contains("No commits yet"), "{brief}");
        assert!(brief.contains("Commit your work on it"), "{brief}");
        assert!(!brief.contains("start from"), "there is nowhere to go back to: {brief}");

        let _ = tokio::fs::remove_dir_all(&root).await;
    }

    #[tokio::test]
    async fn a_directory_git_will_not_talk_about_has_no_footing() {
        // A repository moved or deleted since it was linked. The job goes ahead
        // without a preamble rather than being refused for one.
        assert!(footing("/no/such/directory/anywhere").await.is_none());
    }
}
