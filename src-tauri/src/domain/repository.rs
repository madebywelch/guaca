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
    pub path: String,
    #[serde(default)]
    pub note: String,
    /// Absent is `pi`, which is what a caller that has never heard of this field
    /// means and what every repository linked before it existed ran.
    #[serde(default)]
    pub harness: Harness,
}

/// A draft that has passed everything checkable without touching the disk.
#[derive(Debug, Clone, PartialEq)]
pub struct CleanRepository {
    pub group_id: GroupId,
    pub name: String,
    pub path: String,
    pub note: String,
    pub harness: Harness,
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

        Ok(RepositoryEdit { name, note, harness: self.harness })
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
}

impl RepositoryDraft {
    /// Everything that can be decided without a filesystem.
    ///
    /// Trailing separators are taken off so `/src/app` and `/src/app/` cannot
    /// become two repositories pointed at one directory. The unique index in
    /// the store is on the path, and it can only hold if the same directory
    /// spells the same way every time.
    pub fn clean(&self) -> Result<CleanRepository, RepositoryError> {
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
        }
        .clean()?;

        Ok(CleanRepository {
            group_id: self.group_id,
            name: edit.name,
            path: path.to_string(),
            note: edit.note,
            harness: edit.harness,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn draft(path: &str) -> RepositoryDraft {
        RepositoryDraft {
            group_id: GroupId::new(),
            name: String::new(),
            path: path.to_string(),
            note: String::new(),
            harness: Harness::default(),
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
            name: "  guac  ".into(),
            note: "  never touch migrations  ".into(),
            harness: Harness::Claude,
        }
        .clean()
        .expect("an edit carries no path and must not be refused for one");

        assert_eq!(clean.name, "guac");
        assert_eq!(clean.note, "never touch migrations");
        assert_eq!(clean.harness, Harness::Claude);
    }

    #[test]
    fn an_edit_refuses_what_a_link_refuses_and_a_name_it_cannot_backfill() {
        let refused = |name: &str, note: &str| {
            RepositoryEdit { name: name.into(), note: note.into(), harness: Harness::Pi }
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
            name: "guaca".into(),
            path: "/dev/guaca".into(),
            note: "run ./scripts/ci.sh before you finish".into(),
            harness: Harness::Pi,
            created_at: 0,
            updated_at: 0,
        };
        let line = repo.own_line();
        assert!(line.contains("/dev/guaca"), "an agent works by path: {line}");
        assert!(line.contains("./scripts/ci.sh"), "the note is written to be read here: {line}");
    }
}
