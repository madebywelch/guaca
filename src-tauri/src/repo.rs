//! What git says about the directory a coding job is about to start in.
//!
//! One question, asked of git itself rather than of a table: which branch is
//! this tree on, is anything uncommitted, and has that branch already landed.
//! The answer goes in front of every brief, because a harness handed a brief
//! starts editing wherever it is standing. [`Footing`] is the argument.
//!
//! Nothing here changes the tree. An agent's terminal is its own, and what it
//! clones into it is its own to fetch, switch and clean; this only reads.

/// What `git status` says about a work tree.
///
/// Read rather than remembered. Nothing here is stored: the answer changes when
/// anybody commits, pulls or switches, and none of those go through Guaca. A
/// cached copy would be wrong exactly when somebody looked.
#[derive(Debug, Clone, PartialEq)]
pub struct RepoStatus {
    /// The branch name, or `HEAD` detached at a commit.
    pub branch: String,
    /// Whether the work tree is at a commit rather than on a branch, which is
    /// a state somebody has to get out of rather than a place to work from.
    pub detached: bool,
    /// Paths that differ from HEAD: modified, staged, untracked, unmerged.
    /// One number rather than four, because the question it answers is "is
    /// there uncommitted work here".
    pub dirty: u32,
    /// Commits on this branch the upstream does not have, and the reverse.
    /// Both zero when there is no upstream, which is not the same as being in
    /// sync and is why `upstream` is separate.
    pub ahead: u32,
    pub behind: u32,
    pub upstream: bool,
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

/// Where a coding job is about to start, and what that means for the branch.
///
/// Read once, by a program that is about to write to the tree, and it exists to
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
    /// What git says about the tree.
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
    /// A fresh `git init` is an ordinary place to start, and it is the one state
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

    /// Whether this tree can be put back on the default branch without losing
    /// anything anyone would miss.
    ///
    /// Asked only of a tree Guaca owns, which is what makes an automatic answer
    /// defensible at all. The operator's own checkout is never reset on any
    /// answer this gives, and [`Footing::rule`] is the version of this question
    /// that is put to the harness in prose instead.
    ///
    /// Three ways for the answer to be no, and each one is work that exists in
    /// exactly one place:
    ///
    /// - Uncommitted changes. A job killed at the ceiling leaves them, and they
    ///   are the only copy.
    /// - Commits this branch has that neither the default branch nor a remote
    ///   has. `merged` covers work that landed; `upstream && ahead == 0` covers
    ///   work that is pushed and can be fetched back. Anything else is local
    ///   only.
    /// - No default branch published here, or no commits at all, which are the
    ///   two states with nowhere to be put back *to*.
    ///
    /// Note what is deliberately not a reason to hold. A branch that is pushed
    /// and has a pull request open is reset away from, because its work is
    /// safe on the remote and the next brief is usually about something else.
    /// A job that needs to go back to it says so and checks it out, which is
    /// one command, against a tree that otherwise stays on a landed branch for
    /// weeks.
    pub fn resettable(&self) -> bool {
        !self.unborn
            && self.default_branch.is_some()
            && self.tree.dirty == 0
            && (self.merged || (self.tree.upstream && self.tree.ahead == 0))
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
/// `None` is a directory git will not talk about: not a repository at all, or
/// one that is not there any more. The job goes ahead without the preamble
/// rather than being refused for it, and the brief says so instead.
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
/// Every way of not knowing is one answer and it is not zero: `gh` not
/// installed, not signed in, no remote, or a network that is down all mean the
/// question could not be asked, and drawn as zero each would claim there is no
/// pull request open for this branch. The first row is
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
    use std::path::{Path, PathBuf};

    use super::*;

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
        // A fresh `git init` is an ordinary place to start. Git names an unborn
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
        // Not a repository, or not there. The job goes ahead without a
        // preamble rather than being refused for one.
        assert!(footing("/no/such/directory/anywhere").await.is_none());
    }
}
