//! The boundary between "an agent's computer" and whoever runs it.
//!
//! One operation matters: `exec`. The desktop, the browser, screenshots,
//! attachments and sign-in detection are all commands, so a provider that can
//! create a machine and run a command on it gets every feature. The rest of
//! the trait is lifecycle: find out what state it is in, sleep, wake, delete,
//! and list what this app owns so a crash cannot leak one.

use std::collections::BTreeMap;
use std::time::Duration;

use serde::{Deserialize, Serialize};

use crate::domain::computer::{Provider, Secret};
use crate::domain::ids::{AgentId, ComputerId};

/// How a provider finds a machine again. Backend-only: never serialised, never
/// over IPC. The secrets are on it because they are useless apart from the id
/// they belong to.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProviderHandle {
    pub computer: ComputerId,
    pub provider_id: String,
    pub control_secret: Secret,
    pub viewer_secret: Secret,
}

/// What a machine is doing. Anything a provider cannot classify is an error,
/// not `Gone`: `Gone` is permission to replace a disk.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProviderState {
    Running,
    /// Stopped with its disk intact. Can be started.
    Asleep,
    /// The provider positively reports there is no such machine.
    Gone,
}

/// One command. Always an argument vector: the guest sees exactly these
/// strings, and nothing on the host interprets them.
#[derive(Clone, PartialEq, Eq)]
pub struct ExecRequest {
    pub argv: Vec<String>,
    /// Credentials the group holds. Values live here and in the guest process
    /// and nowhere else; see `Debug` below.
    pub env: BTreeMap<String, String>,
    pub cwd: String,
    pub timeout: Duration,
}

impl std::fmt::Debug for ExecRequest {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ExecRequest")
            .field("argv", &self.argv)
            .field("env", &self.env.keys().collect::<Vec<_>>())
            .field("cwd", &self.cwd)
            .field("timeout", &self.timeout)
            .finish()
    }
}

/// The result of one command.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Output {
    pub stdout: String,
    pub stderr: String,
    pub exit_code: i32,
}

impl Output {
    /// What the model is shown. Both streams, labelled, with the exit code only
    /// when it is not zero: a successful command should read as its output and
    /// nothing else.
    pub fn rendered(&self) -> String {
        let mut out = String::new();
        if !self.stdout.trim().is_empty() {
            out.push_str(self.stdout.trim_end());
        }
        if !self.stderr.trim().is_empty() {
            if !out.is_empty() {
                out.push('\n');
            }
            out.push_str("stderr: ");
            out.push_str(self.stderr.trim_end());
        }
        if self.exit_code != 0 {
            if !out.is_empty() {
                out.push('\n');
            }
            out.push_str(&format!("(exit code {})", self.exit_code));
        }
        if out.is_empty() {
            out.push_str("(no output)");
        }
        out
    }
}

/// Where the viewer proxy should connect for one machine's port, and what to
/// add to the request head on the way. Backend-only.
#[derive(Clone, PartialEq, Eq)]
pub struct ViewerTarget {
    pub tls: bool,
    pub host: String,
    pub port: u16,
    /// Headers the upstream needs and the webview must never hold. Values live
    /// here and on the wire to the provider and nowhere else; see `Debug`
    /// below.
    pub headers: Vec<(String, String)>,
}

// A header value here is the token that reaches a machine's desktop. Nothing
// prints a target today; a derived Debug is what would make the first
// `tracing::warn!(?target, ..)` a leak, so the impl exists before the caller.
impl std::fmt::Debug for ViewerTarget {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ViewerTarget")
            .field("tls", &self.tls)
            .field("host", &self.host)
            .field("port", &self.port)
            .field("headers", &self.headers.iter().map(|(name, _)| name).collect::<Vec<_>>())
            .finish()
    }
}

/// What a provider is told when asked for a new machine.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CreateComputer {
    pub computer: ComputerId,
    pub agent: AgentId,
    /// For the provider's own labelling only. Never an identity: agents can be
    /// renamed.
    pub agent_name: String,
    pub idle_seconds: u32,
}

/// Why a provider could not. Each variant is a different next step for whoever
/// reads it: install or configure something, wait or restart, make a new
/// machine, or look at the message.
#[derive(Debug, thiserror::Error)]
pub enum ProviderError {
    #[error("{0}")]
    Unconfigured(String),
    /// This Mac cannot run machines of this kind at all, and no amount of
    /// configuration changes that.
    #[error("{0}")]
    Unsupported(String),
    #[error("{0}")]
    Unavailable(String),
    /// The image a machine boots could not be fetched. Separate from
    /// `Unavailable` because the way out is a network or a different image
    /// reference, not waiting.
    #[error("{0}")]
    Image(String),
    /// A command outran its deadline. Deliberately not `Unavailable`: the
    /// machine is fine and the work may still be running on it, so this is
    /// never permission to replace anything.
    #[error("{0}")]
    Timeout(String),
    /// The provider positively reports the resource is not there. The only
    /// failure that means "make a new one" rather than "try again".
    #[error("{0}")]
    ResourceGone(String),
    #[error("{0}")]
    Operation(String),
}

/// A command that outran its deadline, said to the model that ran it.
///
/// The way forward is in the message because the model is the only one who can
/// take it: the process is still running on its machine, and a rerun of the
/// same command gets the same deadline. A refusal that only names the limit is
/// reworded and tried again.
///
/// Shared, because a command that hangs is the same thing to an agent whether
/// the machine is in a cloud or in a VM on this Mac, and two providers wording
/// it differently is two things for a model to learn.
pub fn timed_out(timeout: Duration) -> ProviderError {
    ProviderError::Timeout(format!(
        "the command did not finish within {}s; run long work in the background with nohup or \
         setsid and poll for its output",
        timeout.as_secs()
    ))
}

/// Whether a provider could make a machine right now, and if not, what the
/// operator would have to do about it. The one shape Settings draws for every
/// provider, so the answer is a state and a sentence rather than an error.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProviderStatus {
    pub state: ProviderReadiness,
    /// Whether starting a computer would start whatever is stopped. False when
    /// there is nothing to start, and false when starting it is not this app's
    /// to do.
    pub can_start: bool,
    /// What to tell the operator: what is true now, and the next step when
    /// there is one. Read by a person in Settings.
    pub detail: String,
}

impl ProviderStatus {
    /// The one state with no next step, and the only one built by more than one
    /// provider.
    pub fn ready(detail: impl Into<String>) -> Self {
        Self { state: ProviderReadiness::Ready, can_start: false, detail: detail.into() }
    }
}

/// Deliberately not `Option<ProviderStatus>`: "not installed", "installed but
/// stopped" and "this Mac cannot run it" are three different sentences and
/// three different things to offer, and they used to be one absent provider.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum ProviderReadiness {
    Ready,
    NotInstalled,
    /// Installed, and its service is not running. Startable.
    NotRunning,
    /// This machine or this build cannot drive it, whatever is installed.
    Unsupported,
    /// It answered, and what it said was not something this build can read.
    Error,
}

#[async_trait::async_trait]
pub trait ComputerProvider: Send + Sync {
    fn kind(&self) -> Provider;

    /// Whether this provider could make a machine, asked without making one.
    ///
    /// Answered for Settings and for whether an agent is told it has a
    /// computer at all, so it runs on paths that have nothing to do with an
    /// agent's turn. Never fails: not being able to find out is one of the
    /// states.
    async fn probe(&self) -> ProviderStatus;

    async fn create(&self, request: &CreateComputer) -> Result<ProviderHandle, ProviderError>;

    /// Without waking it. Asking a sleeping machine anything else would wake
    /// it, and waking is a decision the manager makes.
    async fn inspect(&self, handle: &ProviderHandle) -> Result<ProviderState, ProviderError>;

    /// Wakes it, and answers with the handle it now answers to: E2B reissues
    /// both tokens on resume, so the old handle is wrong the moment this
    /// returns.
    async fn start(
        &self,
        handle: &ProviderHandle,
        idle_seconds: u32,
    ) -> Result<ProviderHandle, ProviderError>;

    /// The manager is about to use it. Failure is not worth interrupting an
    /// agent for, so there is no result.
    async fn keep_awake(&self, handle: &ProviderHandle, idle_seconds: u32);

    async fn stop(&self, handle: &ProviderHandle) -> Result<(), ProviderError>;

    /// Idempotent: a machine that is already gone is the outcome wanted.
    async fn delete(&self, handle: &ProviderHandle) -> Result<(), ProviderError>;

    async fn exec(
        &self,
        handle: &ProviderHandle,
        request: ExecRequest,
    ) -> Result<Output, ProviderError>;

    async fn viewer_target(
        &self,
        handle: &ProviderHandle,
        port: u16,
    ) -> Result<ViewerTarget, ProviderError>;

    /// Every machine this app made, by provider id, whether or not anything
    /// still refers to it.
    async fn list_owned(&self) -> Result<Vec<String>, ProviderError>;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_exec_request_prints_variable_names_but_never_values() {
        // The one place a credential value exists on the host side is this
        // struct. A derived Debug on it would put the value in the first log
        // line that mentions the request.
        let request = ExecRequest {
            argv: vec!["/bin/bash".into(), "-lc".into(), "echo hi".into()],
            env: BTreeMap::from([("GITHUB_TOKEN".to_string(), "ghp_sentinel".to_string())]),
            cwd: "/home/user".into(),
            timeout: Duration::from_secs(1),
        };
        let printed = format!("{request:?}");
        assert!(printed.contains("GITHUB_TOKEN"), "{printed}");
        assert!(!printed.contains("ghp_sentinel"), "{printed}");
    }

    #[test]
    fn a_viewer_target_prints_header_names_but_never_values() {
        // The traffic token rides in a header here, and the proxy logs the
        // target it could not reach. A derived Debug would put that token in
        // the log line for every desktop that failed to answer.
        let target = ViewerTarget {
            tls: true,
            host: "6080-sbx.e2b.app".into(),
            port: 443,
            headers: vec![("e2b-traffic-access-token".into(), "tok_sentinel".into())],
        };
        let printed = format!("{target:?}");
        assert!(printed.contains("e2b-traffic-access-token"), "{printed}");
        assert!(!printed.contains("tok_sentinel"), "{printed}");
    }

    #[test]
    fn a_command_that_outran_its_deadline_is_told_how_to_outlive_one() {
        // Read mid-turn by the model that ran the command, and it is the only
        // one who can act on it: the same command run again gets the same
        // deadline, so the message has to name the way past it.
        let err = timed_out(Duration::from_secs(120));
        let ProviderError::Timeout(message) = err else {
            panic!("a deadline is its own outcome: the machine is fine and the work may go on");
        };
        assert!(message.contains("120s"), "{message}");
        assert!(message.contains("nohup"), "a refusal with no way forward is reworded and retried");
    }

    #[test]
    fn rendering_favours_the_output_and_mentions_the_exit_code_only_when_it_matters() {
        let ok = Output { stdout: "72F sunny\n".into(), stderr: String::new(), exit_code: 0 };
        assert_eq!(ok.rendered(), "72F sunny");

        let bad = Output { stdout: String::new(), stderr: "not found".into(), exit_code: 127 };
        assert_eq!(bad.rendered(), "stderr: not found\n(exit code 127)");

        let quiet = Output { stdout: String::new(), stderr: String::new(), exit_code: 0 };
        assert_eq!(quiet.rendered(), "(no output)", "silence must not look like a failure");
    }
}
