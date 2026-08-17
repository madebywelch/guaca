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
/// reads it: configure, wait or restart, or look at the message.
#[derive(Debug, thiserror::Error)]
pub enum ProviderError {
    #[error("{0}")]
    Unconfigured(String),
    #[error("{0}")]
    Unavailable(String),
    #[error("{0}")]
    Operation(String),
}

#[async_trait::async_trait]
pub trait ComputerProvider: Send + Sync {
    fn kind(&self) -> Provider;

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
    fn rendering_favours_the_output_and_mentions_the_exit_code_only_when_it_matters() {
        let ok = Output { stdout: "72F sunny\n".into(), stderr: String::new(), exit_code: 0 };
        assert_eq!(ok.rendered(), "72F sunny");

        let bad = Output { stdout: String::new(), stderr: "not found".into(), exit_code: 127 };
        assert_eq!(bad.rendered(), "stderr: not found\n(exit code 127)");

        let quiet = Output { stdout: String::new(), stderr: String::new(), exit_code: 0 };
        assert_eq!(quiet.rendered(), "(no output)", "silence must not look like a failure");
    }
}
