//! Where the runtime is running, and the handful of things that decides.
//!
//! Guaca runs in two places. In the desktop app it is a `tokio` runtime inside
//! the window's own process, on the operator's machine, with their disk and
//! their programs. On a server it is a daemon on a machine reached over a
//! network, and the operator is somewhere else entirely.
//!
//! ## Two, not three
//!
//! An operator chooses between three things: run it here, run it on a box I
//! rent, or let guaca.ai hand me one. The runtime only ever sees two, because
//! the difference between the last two is who pressed the button at the
//! provider. A managed box and an operator's own box run the same binary, hold
//! the same state and refuse the same things, and nothing below this line can
//! tell them apart or has any business trying. Whoever provisioned it is a fact
//! about the bill, and the bill is not the runtime's subject.
//!
//! That is also what makes bring-your-own-box free rather than a second
//! product: there is no managed code path to fall out of step with.
//!
//! ## What it decides, and where the decision is read
//!
//! Five capabilities, and every one of them is something that is *on the
//! operator's machine* rather than something Guaca chose not to implement. A
//! credential bound to the program it was issued to, a working tree with
//! uncommitted work in it, a model server on loopback, and a file on a disk
//! cannot be reached from an origin, and no amount of engineering here changes
//! that.
//!
//! The rule is that a refusal is drawn **in the panel, before anything is
//! spent**, and never discovered at turn time. This is the same shape as a
//! model named on a group running on Claude, which is kept and never used and
//! says so on the row: a capability that is quietly absent is the one thing
//! nothing else on screen would explain.

use serde::{Deserialize, Serialize};

/// Where this runtime is.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub enum Deployment {
    /// In the desktop app's own process, on the operator's machine.
    #[default]
    Desktop,
    /// A daemon on a machine the operator reaches over a network. Their own box
    /// or one guaca.ai provisioned; this type cannot tell, and neither can
    /// anything that reads it.
    Server,
}

impl Deployment {
    /// What this deployment can do, which is the value every panel asks.
    pub const fn capabilities(self) -> Capabilities {
        match self {
            Deployment::Desktop => Capabilities {
                local_directories: true,
                loopback_endpoints: true,
                claude_provider: true,
                claude_code_harness: true,
                local_files: true,
            },
            Deployment::Server => Capabilities {
                local_directories: false,
                loopback_endpoints: false,
                claude_provider: false,
                claude_code_harness: false,
                local_files: false,
            },
        }
    }

    pub const fn is_server(self) -> bool {
        matches!(self, Deployment::Server)
    }
}

/// What a deployment can do.
///
/// A struct of flags rather than a set of `matches!` calls scattered through
/// the panels. The list is short and it is meant to stay short: a sixth flag
/// has to be something that is physically on the operator's machine, not a
/// feature somebody has not got round to.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Capabilities {
    /// Whether a repository may be a directory the operator picked.
    ///
    /// On a server a repository is a remote and a credential instead. That is
    /// a different feature rather than this one degraded: `docs/CODING.md` is
    /// explicit that seeing the branch you are on and the change you have not
    /// committed is the point of the local version, and it does not survive
    /// the move.
    pub local_directories: bool,
    /// Whether inference may be pointed at a loopback address.
    ///
    /// LM Studio and Ollama are two clicks in Settings on a desktop. A server
    /// cannot reach them, and a tunnel that let it would put "is your laptop
    /// awake" back in front of exactly the turns that use it.
    pub loopback_endpoints: bool,
    /// Whether a turn may be paid for by a Claude plan.
    ///
    /// `Provider::Claude` works by *being the program*: `claude` runs on the
    /// operator's machine, signed in where they signed in, so the credential
    /// never leaves the program it was issued to. There is no version of that
    /// which involves shipping the credential to a box.
    pub claude_provider: bool,
    /// Whether Claude Code may be the harness that writes the code.
    ///
    /// The same fact one level down, and it is about the subscription rather
    /// than the program: Claude Code driven by an API key is a different
    /// credential and is not what this flag is about.
    pub claude_code_harness: bool,
    /// Whether a file may be named by a path on the operator's disk, and a
    /// saved copy land in their downloads folder.
    ///
    /// On a server both become the browser's own upload and download. The
    /// capability is gone; the ability to hand a document over is not.
    pub local_files: bool,
}

/// Why a capability is absent, phrased for whoever is about to hit it.
///
/// Every one of these says the same two things in different words: what is
/// missing, and what to do instead. A refusal that only says no gets reworded
/// and retried, and on a panel it gets reported as a bug.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Absent {
    LocalDirectories,
    LoopbackEndpoints,
    ClaudeProvider,
    ClaudeCodeHarness,
    LocalFiles,
}

impl Absent {
    pub const fn sentence(self) -> &'static str {
        match self {
            Absent::LocalDirectories => {
                "this workspace runs on a server, so it cannot open a directory on your own \
                 machine. Link the repository by its remote instead, and the work comes back as \
                 a branch and a pull request"
            }
            Absent::LoopbackEndpoints => {
                "this workspace runs on a server, so it cannot reach a model running on your own \
                 machine. Point it at an endpoint the server can reach, or run this crew in a \
                 local workspace"
            }
            Absent::ClaudeProvider => {
                "a Claude plan pays for a turn by running the `claude` program where you signed \
                 in, which is your own machine, so it is not offered on a server. Use an API key \
                 or a ChatGPT subscription here, or run this crew in a local workspace"
            }
            Absent::ClaudeCodeHarness => {
                "Claude Code spends the plan you signed in to on your own machine, so it is not \
                 offered on a server. Pick the other harness, or run this repository from a \
                 local workspace"
            }
            Absent::LocalFiles => {
                "this workspace runs on a server, so it cannot read a path on your own machine. \
                 Drop the file onto the window or pick it with the file button, and the bytes \
                 are uploaded"
            }
        }
    }
}

impl std::fmt::Display for Absent {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.sentence())
    }
}

impl Capabilities {
    /// Refuses when the capability is absent, with the sentence to say.
    ///
    /// Written as a `Result` so a call site is a `?` rather than an `if` with
    /// a hand-written message beside it. Two panels writing their own wording
    /// for one refusal is how an operator gets told two different things about
    /// one fact.
    pub const fn require(self, what: Absent) -> Result<(), Absent> {
        let have = match what {
            Absent::LocalDirectories => self.local_directories,
            Absent::LoopbackEndpoints => self.loopback_endpoints,
            Absent::ClaudeProvider => self.claude_provider,
            Absent::ClaudeCodeHarness => self.claude_code_harness,
            Absent::LocalFiles => self.local_files,
        };
        if have {
            Ok(())
        } else {
            Err(what)
        }
    }
}

/// Whether a URL is one only the machine it is typed on can reach.
///
/// Read before an endpoint is stored rather than when a turn fails against it,
/// which is the difference between a sentence in a settings pane and a crew
/// that silently cannot think. Covers the three spellings of loopback plus the
/// name every local model server prints in its own console, because an operator
/// copying that console line is the whole case this exists for.
///
/// It reads the address and never resolves it. A hostname that happens to point
/// at loopback (`127.0.0.1.nip.io`, or an entry somebody put in `/etc/hosts`)
/// is not caught, and could not honestly be: a name resolves differently on two
/// machines and differently tomorrow, so an answer computed here would be a
/// guess presented as a rule. What is caught is the mistake this exists for,
/// which is a console line pasted into a settings box.
pub fn is_loopback(url: &str) -> bool {
    let Some(rest) = url.split_once("://").map(|(_, rest)| rest) else {
        return false;
    };
    let host = rest.split(['/', '?', '#']).next().unwrap_or_default();
    // Strip credentials and the port. An IPv6 literal keeps its brackets until
    // the port is taken off, so the bracket check comes first.
    let host = host.rsplit_once('@').map(|(_, h)| h).unwrap_or(host);
    let host = if let Some(end) = host.find(']') {
        &host[..=end]
    } else {
        host.split(':').next().unwrap_or_default()
    };
    let host = host.trim_start_matches('[').trim_end_matches(']').to_ascii_lowercase();
    host == "localhost"
        || host == "::1"
        || host == "0.0.0.0"
        || is_loopback_v4(&host)
        // What a machine calls itself from inside a container, which is the
        // same mistake wearing a different hostname.
        || host == "host.docker.internal"
}

/// Whether a host is a dotted-quad in `127.0.0.0/8`.
///
/// Four numeric octets, checked rather than prefix-matched. `127.0.0.1.nip.io`
/// starts with the same four characters and is a *name*: this function is about
/// literals, and the paragraph above says why nothing here resolves one.
fn is_loopback_v4(host: &str) -> bool {
    let mut octets = host.split('.');
    let first = octets.next().and_then(|o| o.parse::<u8>().ok());
    let rest = octets.filter_map(|o| o.parse::<u8>().ok()).count();
    first == Some(127) && rest == 3 && host.split('.').count() == 4
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_desktop_can_do_everything_and_a_server_can_do_none_of_it() {
        // Not a tautology: it is the assertion that the five are exactly the
        // things that live on the operator's machine. A flag that is true in
        // both places does not belong in this struct, and one that is false in
        // both is a feature nobody built rather than a capability.
        let desktop = Deployment::Desktop.capabilities();
        let server = Deployment::Server.capabilities();
        for what in [
            Absent::LocalDirectories,
            Absent::LoopbackEndpoints,
            Absent::ClaudeProvider,
            Absent::ClaudeCodeHarness,
            Absent::LocalFiles,
        ] {
            assert_eq!(desktop.require(what), Ok(()), "a desktop has {what:?}");
            assert_eq!(server.require(what), Err(what), "a server does not have {what:?}");
        }
    }

    #[test]
    fn every_refusal_says_what_to_do_instead() {
        // The rule the whole app is written to: a refusal that only says no
        // gets reworded and retried by a model, and reported as a bug by a
        // person. Each of these has to carry a way forward, and the cheapest
        // check that it does is that it offers an alternative in words.
        for what in [
            Absent::LocalDirectories,
            Absent::LoopbackEndpoints,
            Absent::ClaudeProvider,
            Absent::ClaudeCodeHarness,
            Absent::LocalFiles,
        ] {
            let said = what.sentence();
            assert!(said.len() > 60, "{what:?} is too short to have said anything: {said}");
            assert!(
                said.contains(" or ") || said.contains("instead") || said.contains(", and "),
                "{what:?} refuses without offering a way forward: {said}"
            );
            // Interpolated mid-sentence, so it opens lowercase. A proper noun
            // is the one thing that legitimately does not, and it is named
            // here rather than the rule being dropped for it.
            assert!(
                !said.starts_with(char::is_uppercase) || said.starts_with("Claude Code"),
                "{what:?} is interpolated mid-sentence and must not open with a capital: {said}"
            );
        }
    }

    #[test]
    fn the_default_is_the_desktop_because_that_is_what_exists_today() {
        // A missing value must never silently take capabilities away from an
        // app that has always had them.
        assert_eq!(Deployment::default(), Deployment::Desktop);
    }

    #[test]
    fn a_server_is_a_server_however_it_was_provisioned() {
        // The whole argument for two variants rather than three. If this ever
        // needs a third, something below this line has started caring who paid
        // for the box, and that is the thing to undo.
        assert!(Deployment::Server.is_server());
        assert!(!Deployment::Desktop.is_server());
    }

    #[test]
    fn loopback_is_recognized_in_every_spelling_a_console_prints() {
        for url in [
            "http://localhost:1234/v1",
            "http://LOCALHOST:1234/v1",
            "http://127.0.0.1:11434/v1",
            "http://127.1.2.3:8080",
            "https://[::1]:443/v1",
            "http://0.0.0.0:8000/v1",
            "http://host.docker.internal:1234/v1",
            "http://user:pass@localhost:1234/v1",
        ] {
            assert!(is_loopback(url), "{url} is only reachable from the machine it is typed on");
        }
    }

    #[test]
    fn a_reachable_endpoint_is_not_mistaken_for_a_local_one() {
        // The expensive direction. A wrong "this is loopback" refuses an
        // endpoint that works, in a settings pane, with no way round it.
        for url in [
            "https://openrouter.ai/api/v1",
            "https://api.openai.com/v1",
            "http://192.168.1.10:1234/v1",
            "http://10.0.0.4:1234/v1",
            // Names that merely start the same way. A server called
            // `localhost.example.com` is somebody else's machine, and
            // `127.0.0.1.nip.io` is a name this deliberately does not resolve.
            "https://localhost.example.com/v1",
            "https://127.0.0.1.nip.io/v1",
            "http://127.0.0.1.example.com/v1",
            "not a url at all",
        ] {
            assert!(!is_loopback(url), "{url} is reachable from anywhere and must not be refused");
        }
    }
}
