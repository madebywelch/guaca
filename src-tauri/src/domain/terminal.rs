//! A terminal: a directory of an agent's own on the machine Guaca runs on,
//! and the shell that starts in it.
//!
//! An agent given one can run commands there (`shell`), read, write and edit
//! the files in it (`read_file`, `write_file`, `edit_file`), clone whatever
//! repositories it needs into it, and hand a change to a coding harness that
//! works in one of them (`code`). Everything about the disk is in
//! [`crate::terminal`]; this module is the shape and the rules that need none.
//!
//! ## Why a directory and not a repository
//!
//! A repository used to be a row: a directory the operator linked, or a remote
//! the workspace cloned, with a credential, a commit identity, a harness, a
//! gate and a worktree policy on it, and at most one agent assigned. By the end
//! each agent already worked in a worktree of its own rather than in the
//! operator's checkout, so the row was only bookkeeping about a directory the
//! agent owned anyway. The agent now owns it outright. It clones what it
//! needs, with the machine's own git credentials, and the two decisions that
//! were about the *work* rather than the directory (which program writes the
//! code, and whether a push stops first) moved onto the agent.
//!
//! ## It is not a sandbox, and must never be described as one
//!
//! The shell runs as whoever runs Guaca: the operator on their own machine,
//! the daemon's user on a box, with that user's credentials and network. A
//! command can reach anywhere that user can. The directory is where work
//! starts and where an agent's files are kept, not a boundary around them.
//! What makes handing it over defensible is the same thing it always was: the
//! operator decides which agents get one, and git is the undo for what happens
//! in a repository.

use serde::{Deserialize, Serialize};

/// Which program writes the code when an agent calls `code`.
///
/// Three, and there is not meant to be a general one. A harness is a coding
/// agent with its own loop, its own context and its own sign-in, and the
/// operator already has whichever ones they have: the choice here is which of
/// them Guaca starts, not how it is configured. The model, the thinking level,
/// the extensions and the rules files belong to the harness and stay there.
///
/// ## Why this is a choice at all
///
/// Because a subscription is spent by the program it was issued to, and by no
/// other. A Claude plan pays for work done by `claude`, a ChatGPT plan for
/// work done by `codex`, and `pi` spends whatever it is signed in to, which is
/// how an OpenRouter key writes code. That is the fact `docs/PROTOCOL.md`
/// states from the other end, where it is why Guaca's own turns cannot be paid
/// for with a Claude sign-in except by running the program.
///
/// On the agent rather than the crew, because the agent is the one doing the
/// work and one crew can hold an engineer on each plan.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Harness {
    /// `pi`, and the default because it is what every coding job before this
    /// column ran.
    #[default]
    Pi,
    /// Claude Code, run headless.
    Claude,
    /// The official Codex CLI, over its app-server protocol.
    Codex,
}

impl Harness {
    /// What the column holds, and what crosses IPC. One spelling for both, or
    /// the two drift and only one of them is the one a job is started with.
    pub fn as_str(self) -> &'static str {
        match self {
            Harness::Pi => "pi",
            Harness::Claude => "claude",
            Harness::Codex => "codex",
        }
    }

    /// What an operator is shown. `pi` is spelled the way its own binary is.
    pub fn label(self) -> &'static str {
        match self {
            Harness::Pi => "pi",
            Harness::Claude => "Claude Code",
            Harness::Codex => "Codex",
        }
    }

    /// Every one this build knows, in the order the panel offers them.
    pub const ALL: [Harness; 3] = [Harness::Codex, Harness::Claude, Harness::Pi];

    /// What a stored row means.
    ///
    /// Anything unrecognized is [`Harness::Pi`], the column's default. The only
    /// way to write one is a newer build and then a downgrade, and refusing to
    /// read the row would take the agent off the one panel where the operator
    /// could fix it.
    pub fn parse(raw: &str) -> Harness {
        match raw {
            "claude" => Harness::Claude,
            "codex" => Harness::Codex,
            _ => Harness::Pi,
        }
    }
}

/// Whether an agent's pushes stop on the operator's desk first.
///
/// A push, a merge, a pull request or a release is the operator's own name
/// going somewhere git cannot take it back from, and this is whether one of
/// those parks first. It covers every door the agent has: its own `shell`, and
/// each harness's command tool, through the same
/// [`crate::coding::bridge::outward`] reading and the same
/// `Runtime::ask_about_push`. Everything else, every edit and every test run,
/// is never gated.
///
/// Off by default, and that is not caution about a migration. `coding`'s
/// appended prompt tells every job that it is running unattended and that
/// nobody will answer a question. An operator turning this on is an operator
/// saying they will be there.
///
/// ## It is not a boundary
///
/// The gate reads a shell line and decides whether it looks like a push, which
/// is a judgment about the ordinary case. A command that wanted to get around
/// it could, and it was already running as the operator with their credentials
/// before any of this existed. `docs/CODING.md` says the same at length.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Gate {
    /// Nothing stops.
    #[default]
    Open,
    /// A push, a pull request, a merge or a release asks the operator first.
    AskBeforePushing,
}

impl Gate {
    /// What the column holds, and what crosses IPC.
    pub fn as_str(self) -> &'static str {
        match self {
            Gate::Open => "open",
            Gate::AskBeforePushing => "askBeforePushing",
        }
    }

    /// What a stored row means. Anything unrecognized is [`Gate::Open`]:
    /// reading an unknown value as the asking variant would park work on a
    /// desk over a value a downgrade wrote.
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

/// The coding session an agent last ran, which `code` can carry on.
///
/// A harness keeps its own conversation on disk, so a follow-up can resume it
/// with everything the job had already read and decided rather than starting
/// over from a new brief. That is what an operator gets by typing another
/// message into the program's own window, and what an agent gets here with
/// `code` and `continue`.
///
/// Stored rather than held in memory, because a follow-up often comes the next
/// day. Only the last one per agent: an agent runs one job at a time, and the
/// session before the last is one the agent has already moved on from.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Session {
    /// Which program owns it. A session is only ever continued by the program
    /// that wrote it; the ids mean nothing to the other two.
    pub harness: Harness,
    /// The harness's own id for it: the session id `claude` and `pi` were
    /// started with, or the thread Codex returned.
    pub id: String,
    /// Where it ran, relative to the agent's terminal. A session is resumed in
    /// the directory it was started in, because each harness files its
    /// sessions by working directory.
    pub directory: String,
    pub updated_at: i64,
}

impl Session {
    /// The command that opens the same session in the operator's own terminal,
    /// run from the directory it worked in.
    pub fn resume_command(&self) -> String {
        match self.harness {
            Harness::Claude => format!("claude --resume {}", self.id),
            Harness::Codex => format!("codex resume {}", self.id),
            Harness::Pi => format!("pi --session {}", self.id),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_stored_value_reads_back_as_what_was_written() {
        for harness in Harness::ALL {
            assert_eq!(Harness::parse(harness.as_str()), harness);
        }
        for gate in [Gate::Open, Gate::AskBeforePushing] {
            assert_eq!(Gate::parse(gate.as_str()), gate);
        }
    }

    #[test]
    fn an_unknown_value_reads_as_the_one_that_changes_nothing() {
        assert_eq!(Harness::parse("emacs"), Harness::Pi);
        // Never the asking variant: a value a downgrade wrote must not start
        // parking work on the operator's desk.
        assert_eq!(Gate::parse("askBeforePushingEverything"), Gate::Open);
    }

    #[test]
    fn a_session_is_resumed_by_the_program_that_wrote_it() {
        let session = |harness| Session {
            harness,
            id: "abc".into(),
            directory: "guaca".into(),
            updated_at: 0,
        };
        assert_eq!(session(Harness::Claude).resume_command(), "claude --resume abc");
        assert_eq!(session(Harness::Codex).resume_command(), "codex resume abc");
        assert_eq!(session(Harness::Pi).resume_command(), "pi --session abc");
    }
}
