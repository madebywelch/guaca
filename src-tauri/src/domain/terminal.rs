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
/// them Guaca starts. The sign-in, the extensions and the rules files belong to
/// the harness and stay there. The model and the effort default to the
/// program's own setting, and can be chosen per agent the way the program's
/// own window chooses them: [`Tuning`].
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

/// What an operator can choose inside a harness, the way its own program lets
/// them: which model, how hard it thinks, and for `pi`, which account pays.
///
/// Each is kept per agent *and per harness*. One field shared by all three was
/// the mistake `InferenceConfig` already made once with two providers: the
/// programs have disjoint model names and effort words, so every switch broke
/// the model and switching back did not put it right. An operator who moves an
/// engineer to Codex for an afternoon because a Claude plan ran out comes back
/// to the Claude model they had.
///
/// Absent is the program's own setting, which is what an operator who never
/// opens this gets, and what a job ran before any of this existed. Nothing
/// here is a sign-in: which account a program is signed in to stays the
/// program's, and the one exception, [`Payer::GuacaKey`], lends Guaca's key
/// without handing it over.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Tuning {
    /// The program's own name for a model: an alias Claude Code resolves, a
    /// Codex model id, or a `pi` model id under the provider that pays.
    pub model: Option<String>,
    /// One of [`Harness::efforts`].
    pub effort: Option<String>,
    #[serde(default)]
    pub pays: Payer,
}

/// Which account pays for a harness's model calls.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Payer {
    /// Whatever the program is signed in to on the host.
    #[default]
    Own,
    /// Guaca's own API key, reached through a loopback relay that holds it.
    /// Only `pi`, which is the harness that spends a key rather than a plan.
    GuacaKey,
}

impl Payer {
    pub fn as_str(self) -> &'static str {
        match self {
            Payer::Own => "own",
            Payer::GuacaKey => "guacaKey",
        }
    }

    /// Anything unrecognized is [`Payer::Own`]: a value a downgrade wrote
    /// must not start spending the operator's key.
    pub fn parse(raw: &str) -> Payer {
        match raw {
            "guacaKey" => Payer::GuacaKey,
            _ => Payer::Own,
        }
    }
}

impl Harness {
    /// The effort words each program accepts, as its own help says them.
    ///
    /// Measured against Claude Code 2.1.260 (`--effort`), Codex 0.153.3
    /// (`model/list`'s `supportedReasoningEfforts`, whose union this is) and pi
    /// 0.84.4 (`--thinking`). A model may take fewer than its program does,
    /// which is why the panel offers the model's own list when the program
    /// gives one and this is only the check a stored value has to pass.
    pub fn efforts(self) -> &'static [&'static str] {
        match self {
            Harness::Claude => &["low", "medium", "high", "xhigh", "max"],
            Harness::Codex => &["low", "medium", "high", "xhigh", "max", "ultra"],
            Harness::Pi => &["off", "minimal", "low", "medium", "high", "xhigh", "max"],
        }
    }

    /// Whether this program can be paid for with Guaca's own key.
    pub fn takes_guaca_key(self) -> bool {
        self == Harness::Pi
    }
}

/// Why a tuning was refused, in a sentence the operator can act on.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum TuningError {
    #[error(
        "`{0}` is not a model name {1} can be given: use the name its own model list shows, \
         without spaces"
    )]
    Model(String, &'static str),
    #[error("{1} does not take the effort `{0}`. It takes {2}")]
    Effort(String, &'static str, String),
    #[error(
        "only pi can be paid for with Guaca's key. {0} spends the plan it is signed in to on \
         the backend"
    )]
    Payer(&'static str),
}

impl Tuning {
    /// Trimmed and checked against the program it is for.
    ///
    /// A model is passed to a program as the argument after `--model`, so one
    /// starting with `-` is refused rather than handed over to be read as a
    /// flag. Blank is absent.
    pub fn clean(self, harness: Harness) -> Result<Tuning, TuningError> {
        let blank = |value: Option<String>| {
            value.map(|value| value.trim().to_string()).filter(|value| !value.is_empty())
        };
        let model = blank(self.model);
        if let Some(model) = &model {
            let shaped = model.len() <= 200
                && !model.starts_with('-')
                && model.chars().all(|c| !c.is_whitespace() && !c.is_control());
            if !shaped {
                return Err(TuningError::Model(model.clone(), harness.label()));
            }
        }
        let effort = blank(self.effort);
        if let Some(effort) = &effort {
            if !harness.efforts().contains(&effort.as_str()) {
                return Err(TuningError::Effort(
                    effort.clone(),
                    harness.label(),
                    harness.efforts().join(", "),
                ));
            }
        }
        if self.pays == Payer::GuacaKey && !harness.takes_guaca_key() {
            return Err(TuningError::Payer(harness.label()));
        }
        Ok(Tuning { model, effort, pays: self.pays })
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

    /// The same, as one line an operator pastes into a shell on the host:
    /// into the directory it worked in, then into the session.
    ///
    /// Quoted for a POSIX shell, because the directory is a name the agent
    /// chose and an apostrophe in it would otherwise end the argument early.
    pub fn resume_line(&self, terminal: &std::path::Path) -> String {
        let at = match self.directory.as_str() {
            "." => terminal.to_path_buf(),
            directory => terminal.join(directory),
        };
        let at = at.to_string_lossy().replace('\'', r"'\''");
        format!("cd '{at}' && {}", self.resume_command())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_tuning_is_refused_with_what_the_program_takes() {
        let effort = Tuning { effort: Some("ultra".into()), ..Tuning::default() };
        let why = effort.clean(Harness::Claude).unwrap_err().to_string();
        assert!(why.contains("low, medium, high, xhigh, max"), "{why}");

        // Read as a flag if it were handed over.
        let flag = Tuning { model: Some("-e".into()), ..Tuning::default() };
        assert!(matches!(flag.clean(Harness::Pi), Err(TuningError::Model(..))));
        let spaced = Tuning { model: Some("gpt 5".into()), ..Tuning::default() };
        assert!(spaced.clean(Harness::Codex).is_err());

        // A plan is spent by its program: only pi takes Guaca's key.
        let lent = Tuning { pays: Payer::GuacaKey, ..Tuning::default() };
        assert!(matches!(lent.clone().clean(Harness::Codex), Err(TuningError::Payer(_))));
        assert_eq!(lent.clone().clean(Harness::Pi), Ok(lent));
    }

    #[test]
    fn a_blank_tuning_is_the_programs_own_setting() {
        let blank =
            Tuning { model: Some("  ".into()), effort: Some("".into()), ..Tuning::default() };
        assert_eq!(blank.clean(Harness::Claude), Ok(Tuning::default()));
        let kept = Tuning {
            model: Some(" opus ".into()),
            effort: Some("max".into()),
            ..Tuning::default()
        };
        assert_eq!(
            kept.clean(Harness::Claude).unwrap(),
            Tuning { model: Some("opus".into()), effort: Some("max".into()), ..Tuning::default() }
        );
        // A value a downgrade wrote never starts spending the operator's key.
        assert_eq!(Payer::parse("someday"), Payer::Own);
    }

    #[test]
    fn a_resume_line_survives_a_directory_the_agent_named_badly() {
        let session = Session {
            harness: Harness::Claude,
            id: "s1".into(),
            directory: "bob's site".into(),
            updated_at: 0,
        };
        assert_eq!(
            session.resume_line(std::path::Path::new("/data/terminals/a1")),
            r"cd '/data/terminals/a1/bob'\''s site' && claude --resume s1"
        );
        let home = Session { directory: ".".into(), harness: Harness::Pi, ..session };
        assert_eq!(home.resume_line(std::path::Path::new("/t")), "cd '/t' && pi --session s1");
    }

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
