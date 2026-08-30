//! Repositories: the directories an agent may write code in.
//!
//! A repository is one thing and carries one decision. The thing is a directory
//! on this machine that is the root of a git work tree. The decision is which of
//! the crew's agents works in it, and each of them works in at most one.
//!
//! ## There is no engineer, and that is the design
//!
//! The obvious shape is a tier: mark an agent a specialist, or an engineer, and
//! let the marked ones write code. It was not built, because the mark carries
//! no information the grant does not already carry. An engineer with no
//! repository and an ordinary agent with no repository are the same agent: both
//! are offered nothing that reaches a working tree and neither can make one.
//! A tier on top of that is a second answer to "what may this agent do", and
//! the two would have to be kept in step by hand.
//!
//! It also cannot be refused usefully. Every refusal in this app names what
//! happened and what to do about it, because a model reads it mid-turn.
//! "You are not an engineer" is a fact about a category; "no repository has
//! been given to you, and only the operator can give you one" is a fact the
//! agent can act on and put in its reply.
//!
//! What a tier feels like it would buy is already content rather than runtime.
//! `cafeteria.ts` ships a Software Engineer, a Code Reviewer and a QA Tester,
//! `roles.ts` scores an agent's own words into OpenRouter's `programming`
//! category, and both work today. Designating an engineer is hiring one and
//! giving it a repository. Nothing in the runtime has to know the word.
//!
//! ## An agent works in at most one, and that is about coordination
//!
//! Not permissions. The question a many-to-many answered was "who is allowed in
//! here", and the question this one answers is "who owns this codebase", which
//! is the one an operator actually has.
//!
//! Two agents on one repository settle a change between themselves, in the crew
//! they already share, with messages the operator can read. One agent quietly
//! holding two repositories is a change whose shape nobody can see until it
//! lands in both, and there is no conversation anywhere that says it was
//! coming. The cost is real and it is the intended one: a change that spans two
//! codebases is now two agents talking, which is the thing this app is for.
//!
//! It is a column on the agent rather than a table between the two, because
//! "at most one" is the rule and a column is the only shape that cannot
//! represent anything else. It is also what makes the rail a tree: a repository
//! is a heading with its agents under it, each drawn once, which a
//! many-to-many cannot be.
//!
//! A repository the operator has linked and given to nobody is an ordinary
//! state and is drawn as one. Nothing is inherited: an agent hired next week
//! starts in no repository, like every other capability in this app.
//!
//! ## The path is the root, and git is why
//!
//! A repository has to be a git work tree, and the linked directory has to be
//! its root. The requirement is not ceremony about tooling: git is the undo.
//! Everything an agent does in there is recoverable exactly because a diff, a
//! branch and a revert exist, and that is what makes handing a directory over
//! at all defensible.
//!
//! The root specifically, because the boundary and the undo have to be the same
//! directory. Linked at a subdirectory, an agent writes inside its boundary
//! while `git status` reports a tree it cannot see all of, and a revert reaches
//! outside the boundary to fix it. One directory, one repository, one undo.
//! Narrowing an agent to part of a repository is a sentence in [`Repository::note`],
//! which the model reads, not a second boundary that only half exists.
//!
//! The check itself is I/O and lives in [`crate::repo`]. This module holds the
//! shape and the rules that need no filesystem.

use serde::{Deserialize, Serialize};

use super::ids::{GroupId, RepositoryId};

/// A label longer than this is a sentence, not a name.
pub const MAX_NAME_LEN: usize = 48;
/// Read by a model on every turn, so it is one line, not a page. The same cap
/// [`super::connector::Connector`]'s note carries, for the same reason.
pub const MAX_NOTE_LEN: usize = 240;
/// Longer than any real path and short enough that a pasted document is refused
/// as a path rather than stored as one.
pub const MAX_PATH_LEN: usize = 1024;

/// Which program does the writing.
///
/// Two, and there is not meant to be a general one. A harness is a coding agent
/// with its own loop, its own context and its own sign-in, and the operator
/// already has whichever ones they have: the choice here is which of them Guaca
/// starts, not how it is configured. Everything else about it (the model, the
/// thinking level, the extensions, the rules file) belongs to the harness and
/// stays there. A second place to say it is a second place for it to be wrong.
///
/// ## Why this is a choice at all
///
/// Because a subscription is spent by the program it was issued to, and by no
/// other. `pi` can hold an Anthropic OAuth credential and dial the Messages API
/// with it, and what comes back is *You're out of extra usage* while `claude`
/// on the same machine, signed in to the same account, runs the same work off
/// the plan. That is the fact `docs/PROTOCOL.md` already states from the other
/// end, where it is why Guaca's own turns cannot be paid for with a Claude
/// sign-in.
///
/// So an operator whose ChatGPT plan is spent and whose Claude plan is not
/// cannot be helped by any amount of configuration on one harness. They need
/// the other program. That is the whole requirement, and two variants are the
/// whole of the answer.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Harness {
    /// `pi`, and the default because it is what every repository linked before
    /// this column existed was already running.
    #[default]
    Pi,
    /// Claude Code, run headless in the repository.
    Claude,
}

impl Harness {
    /// What the column holds, and what crosses IPC. One spelling for both, or
    /// the two drift and only one of them is the one a job is started with.
    pub fn as_str(self) -> &'static str {
        match self {
            Harness::Pi => "pi",
            Harness::Claude => "claude",
        }
    }

    /// What an operator is shown. `pi` is spelled the way its own binary is.
    pub fn label(self) -> &'static str {
        match self {
            Harness::Pi => "pi",
            Harness::Claude => "Claude Code",
        }
    }

    /// Every one this build knows, in the order the panel offers them.
    pub const ALL: [Harness; 2] = [Harness::Pi, Harness::Claude];

    /// What a stored row means.
    ///
    /// Anything unrecognized is [`Harness::Pi`], which is the default the column
    /// was added with and the harness every earlier row ran. The only way to
    /// write an unrecognized one is a newer build and then a downgrade, and the
    /// alternative, refusing to read the row, would take the repository off the
    /// one panel where the operator could fix it. Same reason
    /// `group_repositories` still returns a directory that has been moved on
    /// disk.
    pub fn parse(raw: &str) -> Harness {
        match raw {
            "claude" => Harness::Claude,
            _ => Harness::Pi,
        }
    }
}

/// Whether a job in this directory stops before it reaches outside it.
///
/// A push, a merge or a release is the operator's own name going somewhere git
/// cannot take it back from, and this is whether one of those parks on the desk
/// first. Everything else a job does, every edit and every test run, is what the
/// directory and git already cover and is never gated.
///
/// ## Why it is per repository, and why it is off
///
/// Per repository for the reason [`Harness`] is: it is a fact about how work
/// happens *here*. A crew's own tooling repository and the codebase that ships
/// to customers want opposite answers, and one global setting cannot say that.
///
/// Off by default, and that is not caution about a migration. `coding`'s
/// appended prompt tells every job that it is running unattended and that
/// nobody will answer a question. Switching this on for everybody would make
/// that sentence false in every repository at once, and a job that believes it
/// while a hook silently holds it is a job that reports a push it never made.
/// An operator turning it on is an operator saying they will be there.
///
/// ## It is not a boundary
///
/// The gate reads a shell line and decides whether it looks like a push, which
/// is a judgment about the ordinary case. A job that wanted to get around it
/// could, and it was already running as the operator with their credentials and
/// their network before any of this existed. `coding/bridge.rs` says the same
/// thing where the reading happens, and `docs/CODING.md` says it at length.
/// Neither should ever be softened into a claim about confinement.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Gate {
    /// Nothing stops. The directory and git are the boundary, as they were
    /// before a job could be stopped at all.
    #[default]
    Open,
    /// A push, a pull request, a merge or a release asks the operator first.
    AskBeforePushing,
}

impl Gate {
    /// What the column holds, and what crosses IPC. One spelling for both, for
    /// the reason [`Harness::as_str`] has one.
    pub fn as_str(self) -> &'static str {
        match self {
            Gate::Open => "open",
            Gate::AskBeforePushing => "askBeforePushing",
        }
    }

    /// What a stored row means. Anything unrecognized is [`Gate::Open`], which
    /// is the default the column was added with and what every job before it
    /// did. Same reasoning as [`Harness::parse`], with one addition: reading an
    /// unknown value as the *asking* variant would park jobs on a desk over a
    /// value a downgrade wrote.
    pub fn parse(raw: &str) -> Gate {
        match raw {
            "askBeforePushing" => Gate::AskBeforePushing,
            _ => Gate::Open,
        }
    }

    pub fn asks(self) -> bool {
        self == Gate::AskBeforePushing
    }
}

/// Where an agent's jobs actually run inside a repository.
///
/// The linked directory is the operator's own checkout. A coding job used to
/// run in it, which made three things true at once and all of them bad: the
/// job and the operator shared one branch, two agents in one codebase could not
/// work at the same time, and a job that opened a pull request left the tree
/// standing on a feature branch that landed a week later.
///
/// [`Bench::Own`] gives each agent a linked git worktree of its own, off the
/// same repository, and the job runs there. The operator's checkout is never
/// checked out, never switched and never cleaned. Two agents get two
/// directories, so two jobs run at once. And because Guaca owns the tree, it
/// can put it back: [`crate::repo::prepare`] resets it to the default branch at
/// the start of every job, whenever nothing would be lost by doing so.
///
/// ## Why the reset is at the start and never at the end
///
/// The same argument [`crate::repo::Footing`] is built on. A job killed at the
/// forty-five minute ceiling never runs its cleanup, and a job that died on a
/// spent plan never got there either. Cleanup at the end is a step that
/// sometimes does not happen; preparation at the start always does.
///
/// ## Why it is a choice and not simply how it works
///
/// A worktree is a fresh checkout, and a fresh checkout has no ignored files
/// in it: no `node_modules`, no `target`, no `.venv`, no `.env`. That is the
/// exact thing [`crate::repo`]'s own header says the linked-directory design
/// exists to avoid. Long-lived per agent, the cost is paid once and the caches
/// survive every later job, and the brief says where the operator's checkout is
/// so a job that needs a gitignored file can go and get it. That is a good
/// trade in most repositories and a bad one in a few: submodules, LFS, and
/// checkouts large enough that a second one is a real amount of disk. Those
/// keep [`Bench::Shared`].
///
/// Per repository, for the reason [`Harness`] and [`Gate`] are: it is a fact
/// about how work happens *here*.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Bench {
    /// A worktree per agent, reset to the default branch before every job.
    ///
    /// The default, and the only default in this module that is an opinion
    /// rather than a statement of fact. [`Harness`] and [`Gate`] both default to
    /// what every row written before them was already doing, because changing
    /// that under an operator is not a migration's business. This one cannot do
    /// both: what every earlier row was doing is [`Bench::Shared`], and a new
    /// repository linked today should get the arrangement that keeps the
    /// operator's checkout out of it.
    ///
    /// So the two answers are split by *who is asking*. [`Bench::parse`] reads a
    /// stored row and answers `Shared`, because a row is somebody's decision or
    /// a downgrade's typo and neither is a reason to move their jobs. This
    /// answers a caller that named no preference at all, and migration 44
    /// backfills `shared` explicitly so no existing row is ever read by it.
    #[default]
    Own,
    /// Jobs run in the linked directory itself, as they did before worktrees.
    Shared,
}

impl Bench {
    /// What the column holds, and what crosses IPC. One spelling for both, for
    /// the reason [`Harness::as_str`] has one.
    pub fn as_str(self) -> &'static str {
        match self {
            Bench::Own => "own",
            Bench::Shared => "shared",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Bench::Own => "A worktree per agent",
            Bench::Shared => "The linked directory",
        }
    }

    pub const ALL: [Bench; 2] = [Bench::Own, Bench::Shared];

    /// What a stored row means.
    ///
    /// Anything unrecognized is [`Bench::Shared`], which is deliberately not
    /// [`Bench::default`]. The only way to write an unrecognized value is a
    /// newer build and then a downgrade, and reading one as `Own` would move a
    /// repository's jobs into a worktree the operator never asked for, over a
    /// string this build cannot read. Reading it as `Shared` runs them where
    /// they have always run. Same direction [`Gate::parse`] leans for the same
    /// kind of reason.
    pub fn parse(raw: &str) -> Bench {
        match raw {
            "own" => Bench::Own,
            _ => Bench::Shared,
        }
    }

    /// Whether an agent working here gets a work tree of its own.
    pub fn is_own(self) -> bool {
        self == Bench::Own
    }
}

/// A directory a crew may work in.
///
/// Who is in it is not on this type. An agent carries the repository it works
/// in, so the roster is the answer, and a list here would be the same fact in
/// two places with nothing keeping them in step.
///
/// Serializable in full. There is nothing secret on it: a path is not a
/// credential.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Repository {
    pub id: RepositoryId,
    /// Scoped to a group, like everything else an agent can see. A crew works
    /// on a codebase; another crew's repository is as unreachable as its
    /// credentials and its agents.
    pub group_id: GroupId,
    /// What the operator calls it. Defaults to the directory's own name, which
    /// is right often enough that the field is usually left alone.
    pub name: String,
    /// Absolute, and the root of a git work tree on this machine.
    pub path: String,
    /// One line for the agents that have it, in the operator's words: `run
    /// ./scripts/ci.sh before you say you are done`, `never touch migrations`.
    pub note: String,
    /// Which coding harness a job in this directory starts.
    ///
    /// Per repository rather than per workspace, because it is the same shape
    /// as the note: a fact about how work happens *here*. One codebase can be
    /// the one the operator has a plan left on and another can be the one they
    /// run on an API key, and a single global answer cannot say that. It is not
    /// on the agent, because two agents in one directory running two different
    /// programs is two coding agents in one work tree, which is the thing
    /// `Runtime::start_job` takes a lock to prevent.
    pub harness: Harness,
    /// Whether a job here stops before it reaches outside the directory.
    pub gate: Gate,
    /// Where a job here actually runs: the linked directory, or a worktree of
    /// the working agent's own.
    pub bench: Bench,
    /// Where this was cloned from, for a repository the workspace cloned for
    /// itself. `None` is a directory the operator picked, which is what every
    /// desktop repository is. The credential a clone may hold is not on this
    /// type and never will be: this type crosses IPC.
    pub remote: Option<String>,
    pub created_at: i64,
    pub updated_at: i64,
}

impl Repository {
    /// The line an agent is shown about a repository it has.
    ///
    /// The path is in it because the agent works by path and would otherwise
    /// spend a tool call finding out where it is. The note is in it because the
    /// operator wrote it to be read at exactly this moment.
    pub fn own_line(&self) -> String {
        let mut line = format!("- {} at `{}`", self.name, self.path);
        if !self.note.trim().is_empty() {
            line.push_str(&format!(" ({})", self.note.trim()));
        }
        line
    }
}

/// What an operator submits. Cleaned before it reaches the store.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RepositoryDraft {
    pub group_id: GroupId,
    /// Blank takes the directory's own name.
    #[serde(default)]
    pub name: String,
    /// Blank when `remote` is given: a clone's directory is the workspace's
    /// to choose.
    #[serde(default)]
    pub path: String,
    #[serde(default)]
    pub note: String,
    /// Absent is `pi`, which is what a caller that has never heard of this field
    /// means and what every repository linked before it existed ran.
    #[serde(default)]
    pub harness: Harness,
    /// Absent is `open`, for the same reason and one more: the appended prompt
    /// tells a job nobody will answer a question, so asking has to be something
    /// the operator said rather than something a caller forgot to mention.
    #[serde(default)]
    pub gate: Gate,
    /// Absent is `own`, and this is the one field where absent is an opinion
    /// rather than a statement about what earlier rows did. [`Bench::Own`] says
    /// why: a repository linked today should keep jobs out of the operator's
    /// own checkout, and no stored row ever reaches this default because
    /// migration 44 backfills every one of them.
    #[serde(default)]
    pub bench: Bench,
    /// A remote to clone, instead of a directory to link. The other way a
    /// repository begins, and the only way on a box: the workspace clones it
    /// into a directory of its own and works there. Exactly one of this and
    /// `path` is given.
    #[serde(default)]
    pub remote: Option<String>,
    /// A token for that remote, for a private repository over https. Read by
    /// the command that clones and written to a file beside the settings;
    /// never stored on the row and never read back out.
    #[serde(default)]
    pub credential: Option<String>,
}

/// A draft that has passed everything checkable without touching the disk.
#[derive(Debug, Clone, PartialEq)]
pub struct CleanRepository {
    pub group_id: GroupId,
    pub name: String,
    pub path: String,
    pub note: String,
    pub harness: Harness,
    pub gate: Gate,
    pub bench: Bench,
    pub remote: Option<String>,
}

/// What an operator may change about a repository that is already linked.
///
/// The path is not on it, and the absence is the whole point of the type. A
/// different directory is a different repository: whoever was given this one
/// was given that directory, so editing a path in place would move their
/// boundary, and their undo, with nothing on screen saying a decision had been
/// taken. A shape that cannot carry a path is the only version of that rule
/// nothing downstream can forget.
///
/// It is also here because the obvious alternative does not work, and fails in
/// the one way nobody looks for. An edit routed through [`RepositoryDraft`]
/// needs a stand-in path; the stand-in was `/`, and `/` is the empty string
/// once [`RepositoryDraft::clean`] takes its trailing separator off. Every
/// rename, every note and every harness switch came back as *a repository needs
/// a directory; pick one to link*, about a directory the operator had already
/// picked and could read on the row above the box. Neither the panel nor the
/// store was wrong, and both have tests, which is how it survived.
#[derive(Debug, Clone, PartialEq)]
pub struct RepositoryEdit {
    pub name: String,
    pub note: String,
    pub harness: Harness,
    pub gate: Gate,
    pub bench: Bench,
}

impl RepositoryEdit {
    /// Everything checkable about an edit, which is everything it carries.
    ///
    /// A blank name is refused rather than backfilled. The directory's own name
    /// is what *linking* falls back to and there is no path here to take one
    /// from, and keeping the stored name instead would be a save that quietly
    /// did not do what the box in front of the operator says.
    pub fn clean(&self) -> Result<RepositoryEdit, RepositoryError> {
        let name = self.name.trim().to_string();
        if name.is_empty() {
            return Err(RepositoryError::NoName);
        }
        if name.chars().count() > MAX_NAME_LEN {
            return Err(RepositoryError::NameTooLong);
        }

        let note = self.note.trim().to_string();
        if note.chars().count() > MAX_NOTE_LEN {
            return Err(RepositoryError::NoteTooLong);
        }

        Ok(RepositoryEdit { name, note, harness: self.harness, gate: self.gate, bench: self.bench })
    }
}

#[derive(Debug, thiserror::Error, PartialEq)]
pub enum RepositoryError {
    #[error("a repository needs a directory; pick one to link")]
    NoPath,
    #[error("a repository needs a name; it is what its agents are told the directory is called")]
    NoName,
    #[error(
        "`{0}` is not an absolute path; link a directory by its full path, starting from the root"
    )]
    NotAbsolute(String),
    #[error("that path is longer than {MAX_PATH_LEN} characters, which is not a directory")]
    PathTooLong,
    #[error("a repository's name is at most {MAX_NAME_LEN} characters; this one is a sentence")]
    NameTooLong,
    #[error(
        "a repository's note is at most {MAX_NOTE_LEN} characters. It is read by an agent on \
         every turn, so keep it to the one thing you would say out loud"
    )]
    NoteTooLong,
    #[error(
        "give a directory or a remote, not both: a linked directory is already a clone of \
         wherever it came from"
    )]
    TwoSources,
    #[error(
        "`{0}` does not look like a git remote. An https URL, an ssh address or a `git@` \
         address is one this workspace can clone"
    )]
    NotARemote(String),
}

/// The spellings of a remote this app will clone.
///
/// `git@` and `ssh://` are keys the box holds; `file://` is what every offline
/// test clones from. Anything else typed here is more likely a path or a web
/// page than a remote, and the refusal beats a clone error written for a
/// machine.
pub fn plausible_remote(remote: &str) -> bool {
    ["https://", "http://", "ssh://", "git@", "file://"]
        .iter()
        .any(|scheme| remote.starts_with(scheme))
}

impl RepositoryDraft {
    /// Everything that can be decided without a filesystem.
    ///
    /// Trailing separators are taken off so `/src/app` and `/src/app/` cannot
    /// become two repositories pointed at one directory. The unique index in
    /// the store is on the path, and it can only hold if the same directory
    /// spells the same way every time.
    pub fn clean(&self) -> Result<CleanRepository, RepositoryError> {
        // A remote instead of a path: the workspace clones it and the clone's
        // directory becomes the path, so there is nothing to check about one
        // here beyond its spelling and its name.
        if let Some(remote) = self.remote.as_deref().map(str::trim).filter(|r| !r.is_empty()) {
            if !self.path.trim().is_empty() {
                return Err(RepositoryError::TwoSources);
            }
            if remote.len() > MAX_PATH_LEN {
                return Err(RepositoryError::PathTooLong);
            }
            if !plausible_remote(remote) {
                return Err(RepositoryError::NotARemote(remote.to_string()));
            }
            let edit = RepositoryEdit {
                name: match self.name.trim() {
                    // The repository's own name, as every host spells it last.
                    "" => remote
                        .trim_end_matches('/')
                        .trim_end_matches(".git")
                        .rsplit(['/', ':'])
                        .next()
                        .unwrap_or(remote)
                        .to_string(),
                    given => given.to_string(),
                },
                note: self.note.clone(),
                harness: self.harness,
                gate: self.gate,
                bench: self.bench,
            }
            .clean()?;
            return Ok(CleanRepository {
                group_id: self.group_id,
                name: edit.name,
                path: String::new(),
                note: edit.note,
                harness: edit.harness,
                gate: edit.gate,
                bench: edit.bench,
                remote: Some(remote.to_string()),
            });
        }

        let path = self.path.trim().trim_end_matches('/');
        if path.is_empty() {
            return Err(RepositoryError::NoPath);
        }
        if path.len() > MAX_PATH_LEN {
            return Err(RepositoryError::PathTooLong);
        }
        if !path.starts_with('/') {
            return Err(RepositoryError::NotAbsolute(path.to_string()));
        }

        // The name is decided here rather than inside the edit because only a
        // path can supply the fallback, and everything after it is the same
        // question an edit asks, answered in one place.
        let edit = RepositoryEdit {
            name: match self.name.trim() {
                "" => path.rsplit('/').next().unwrap_or(path).to_string(),
                given => given.to_string(),
            },
            note: self.note.clone(),
            harness: self.harness,
            gate: self.gate,
            bench: self.bench,
        }
        .clean()?;

        Ok(CleanRepository {
            group_id: self.group_id,
            name: edit.name,
            path: path.to_string(),
            note: edit.note,
            harness: edit.harness,
            gate: edit.gate,
            bench: edit.bench,
            remote: None,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn draft(path: &str) -> RepositoryDraft {
        RepositoryDraft {
            gate: Gate::Open,
            group_id: GroupId::new(),
            name: String::new(),
            path: path.to_string(),
            note: String::new(),
            harness: Harness::default(),
            bench: Bench::default(),
            remote: None,
            credential: None,
        }
    }

    fn remote_draft(remote: &str) -> RepositoryDraft {
        RepositoryDraft { remote: Some(remote.to_string()), ..draft("") }
    }

    #[test]
    fn a_remote_is_cleaned_to_a_clone_with_the_repositorys_own_name() {
        for (remote, name) in [
            ("https://github.com/madebywelch/guaca.git", "guaca"),
            ("https://github.com/madebywelch/guaca", "guaca"),
            ("git@github.com:madebywelch/guaca.git", "guaca"),
            ("file:///tmp/bare/guaca.git", "guaca"),
        ] {
            let clean = remote_draft(remote).clean().unwrap();
            assert_eq!(clean.name, name, "{remote}");
            assert_eq!(clean.remote.as_deref(), Some(remote));
            // The clone's directory is the command's to choose, not the draft's.
            assert_eq!(clean.path, "");
        }
    }

    #[test]
    fn a_remote_and_a_directory_together_are_refused() {
        let both = RepositoryDraft {
            remote: Some("https://github.com/x/y".into()),
            ..draft("/dev/guaca")
        };
        assert_eq!(both.clean().unwrap_err(), RepositoryError::TwoSources);
    }

    #[test]
    fn a_web_page_is_not_a_remote() {
        // The likeliest paste after a real remote is the repository's web
        // address with the scheme dropped, or a bare path.
        for wrong in ["github.com/x/y", "/tmp/somewhere", "y.git"] {
            assert!(
                matches!(remote_draft(wrong).clean(), Err(RepositoryError::NotARemote(_))),
                "{wrong}"
            );
        }
    }

    #[test]
    fn a_blank_name_takes_the_directorys_own() {
        assert_eq!(draft("/Users/robert/dev/guaca").clean().unwrap().name, "guaca");
    }

    #[test]
    fn a_trailing_slash_is_the_same_directory() {
        // Not cosmetic. The store's unique index is on the path, so two
        // spellings of one directory would be two repositories, each with its
        // own reach, and the operator would fix one and wonder why nothing
        // changed.
        assert_eq!(draft("/dev/guaca/").clean().unwrap().path, "/dev/guaca");
        assert_eq!(draft("/dev/guaca").clean().unwrap().path, "/dev/guaca");
    }

    #[test]
    fn a_relative_path_is_refused_by_name() {
        let err = draft("dev/guaca").clean().unwrap_err();
        assert_eq!(err, RepositoryError::NotAbsolute("dev/guaca".into()));
        assert!(err.to_string().contains("full path"), "the refusal has to say what to do");
    }

    #[test]
    fn an_empty_path_is_refused_before_anything_else() {
        assert_eq!(draft("   ").clean().unwrap_err(), RepositoryError::NoPath);
    }

    #[test]
    fn the_root_is_an_empty_path_once_its_separator_is_off() {
        // Kept as an explanation rather than as a rule. `/` is what an edit
        // routed through a draft used as its stand-in path, and this is why
        // every rename, note and harness switch was refused for having no
        // directory. `RepositoryEdit` is the fix; this is the reason.
        assert_eq!(draft("/").clean().unwrap_err(), RepositoryError::NoPath);
    }

    #[test]
    fn an_edit_is_not_refused_for_having_no_path() {
        let clean = RepositoryEdit {
            bench: Bench::default(),
            name: "  guac  ".into(),
            note: "  never touch migrations  ".into(),
            harness: Harness::Claude,
            gate: Gate::AskBeforePushing,
        }
        .clean()
        .expect("an edit carries no path and must not be refused for one");

        assert_eq!(clean.name, "guac");
        assert_eq!(clean.note, "never touch migrations");
        assert_eq!(clean.harness, Harness::Claude);
        assert_eq!(clean.gate, Gate::AskBeforePushing);
    }

    #[test]
    fn an_edit_refuses_what_a_link_refuses_and_a_name_it_cannot_backfill() {
        let refused = |name: &str, note: &str| {
            RepositoryEdit {
                bench: Bench::default(),
                name: name.into(),
                note: note.into(),
                harness: Harness::Pi,
                gate: Gate::Open,
            }
            .clean()
            .unwrap_err()
        };

        assert_eq!(refused(&"x".repeat(MAX_NAME_LEN + 1), ""), RepositoryError::NameTooLong);
        assert_eq!(refused("guaca", &"x".repeat(MAX_NOTE_LEN + 1)), RepositoryError::NoteTooLong);
        // There is no path here to take the directory's own name from, and a
        // blank one stored would draw as a repository with no name at all.
        assert_eq!(refused("   ", ""), RepositoryError::NoName);
    }

    #[test]
    fn a_pasted_document_is_not_a_path() {
        assert_eq!(
            draft(&format!("/{}", "a".repeat(MAX_PATH_LEN))).clean().unwrap_err(),
            RepositoryError::PathTooLong
        );
    }

    #[test]
    fn a_note_is_one_line() {
        let mut long = draft("/dev/guaca");
        long.note = "x".repeat(MAX_NOTE_LEN + 1);
        assert_eq!(long.clean().unwrap_err(), RepositoryError::NoteTooLong);
    }

    #[test]
    fn the_line_an_agent_reads_carries_the_path_and_the_note() {
        let repo = Repository {
            id: RepositoryId::new(),
            group_id: GroupId::new(),
            gate: Gate::Open,
            bench: Bench::default(),
            name: "guaca".into(),
            path: "/dev/guaca".into(),
            note: "run ./scripts/ci.sh before you finish".into(),
            harness: Harness::Pi,
            remote: None,
            created_at: 0,
            updated_at: 0,
        };
        let line = repo.own_line();
        assert!(line.contains("/dev/guaca"), "an agent works by path: {line}");
        assert!(line.contains("./scripts/ci.sh"), "the note is written to be read here: {line}");
    }
}
