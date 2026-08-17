//! Agents' computers, behind a provider boundary.
//!
//! The runtime asks for "this agent's machine" and gets a `Machine` it can run
//! commands on. Who actually runs that machine — E2B today, a local container
//! runtime later — is a `ComputerProvider`, and nothing above this module
//! knows which one it got.

pub mod desktop;
pub mod e2b;
pub mod provider;

use std::collections::BTreeMap;
use std::sync::Arc;

use crate::domain::ids::ComputerId;
use provider::{ComputerProvider, ExecRequest, ProviderError, ProviderHandle};

pub use provider::Output;

/// The host the webview loads an agent's desktop from. Named here because the
/// window's CSP has to allow exactly this, and the two silently disagreeing is
/// a blocked iframe that looks identical to a desktop that failed to start.
pub const VIEWER_HOST: &str = "127.0.0.1";

/// A machine the runtime can act on: a provider, the handle it knows the
/// machine by, and the credentials its agent's group holds.
///
/// Credentials are carried on the machine rather than threaded through each
/// call because "which agent is this acting for" is a property of the whole
/// session, and a parameter on `run` would be one that eight call sites could
/// each forget.
#[derive(Clone)]
pub struct Machine {
    provider: Arc<dyn ComputerProvider>,
    handle: ProviderHandle,
    env: BTreeMap<String, String>,
    viewer_port: u16,
}

impl Machine {
    pub fn new(
        provider: Arc<dyn ComputerProvider>,
        handle: ProviderHandle,
        env: BTreeMap<String, String>,
        viewer_port: u16,
    ) -> Self {
        Self { provider, handle, env, viewer_port }
    }

    pub fn id(&self) -> ComputerId {
        self.handle.computer
    }

    /// Runs what an agent typed, with the credentials its group holds.
    ///
    /// Through a login shell so PATH and the usual environment are what a
    /// person would get, not a bare exec. The text is one argument: nothing
    /// on the host interprets it.
    pub async fn run(&self, command: &str) -> Result<Output, ProviderError> {
        self.exec(command, self.env.clone()).await
    }

    /// Runs housekeeping with no credentials. Starting the desktop or reading
    /// the cookie jar never needs a token, and a command that does not need
    /// one should not be able to print it.
    pub async fn run_plain(&self, command: &str) -> Result<Output, ProviderError> {
        self.exec(command, BTreeMap::new()).await
    }

    /// Somewhere to watch the desktop, once it answers.
    ///
    /// Asked of the port, not of the process list. A process that exists is
    /// not the same as one that is serving, and this check used to match the
    /// shell running it: the desktop was reported up when nothing was
    /// listening, so the viewer was handed a dead address and drew a black
    /// rectangle.
    pub async fn vnc_url(&self) -> Option<String> {
        let up = self
            .run_plain(&format!(
                "{} 2>/dev/null && echo up || echo down",
                desktop::port_open(desktop::VNC_PORT)
            ))
            .await
            .map(|o| o.stdout.trim() == "up")
            .unwrap_or(false);

        // Through the local viewer, never straight at the provider: E2B
        // refuses public traffic without a header the webview must not hold,
        // and a local guest's address is nobody's business but the proxy's.
        up.then(|| {
            format!(
                "http://{VIEWER_HOST}:{}/{}/{}/vnc.html?autoconnect=1&resize=scale&reconnect=1",
                self.viewer_port,
                self.handle.computer,
                desktop::VNC_PORT
            )
        })
    }

    async fn exec(
        &self,
        command: &str,
        env: BTreeMap<String, String>,
    ) -> Result<Output, ProviderError> {
        self.provider
            .exec(
                &self.handle,
                ExecRequest {
                    argv: vec!["/bin/bash".into(), "-l".into(), "-c".into(), command.to_string()],
                    env,
                    cwd: "/home/user".into(),
                    timeout: desktop::RUN_TIMEOUT,
                },
            )
            .await
    }
}

#[cfg(test)]
pub(crate) mod fake {
    use super::provider::*;
    use crate::domain::computer::{Provider, Secret};
    use parking_lot::Mutex;
    use std::collections::HashMap;

    /// A provider that keeps its machines in memory and records what it was
    /// asked, so the manager can be tested without a network.
    #[derive(Default)]
    pub struct FakeProvider {
        pub machines: Mutex<HashMap<String, ProviderState>>,
        pub execs: Mutex<Vec<ExecRequest>>,
        pub creates: Mutex<u32>,
        pub deletes: Mutex<Vec<String>>,
        pub fail_create: Mutex<bool>,
        pub fail_delete: Mutex<bool>,
        pub create_delay: Mutex<Option<std::time::Duration>>,
        /// What every exec answers with, in order; the last one repeats.
        pub replies: Mutex<Vec<Output>>,
    }

    #[async_trait::async_trait]
    impl ComputerProvider for FakeProvider {
        fn kind(&self) -> Provider {
            Provider::E2b
        }

        async fn create(&self, request: &CreateComputer) -> Result<ProviderHandle, ProviderError> {
            // Read out and released before the sleep: a guard held across an
            // await is a future that cannot cross threads.
            let delay = *self.create_delay.lock();
            if let Some(delay) = delay {
                tokio::time::sleep(delay).await;
            }
            if *self.fail_create.lock() {
                return Err(ProviderError::Unavailable("fake: create refused".into()));
            }
            *self.creates.lock() += 1;
            let id = format!("fake-{}", request.computer.short());
            self.machines.lock().insert(id.clone(), ProviderState::Running);
            Ok(ProviderHandle {
                computer: request.computer,
                provider_id: id,
                control_secret: Secret::new("ctl"),
                viewer_secret: Secret::new("view"),
            })
        }

        async fn inspect(&self, handle: &ProviderHandle) -> Result<ProviderState, ProviderError> {
            Ok(self
                .machines
                .lock()
                .get(&handle.provider_id)
                .copied()
                .unwrap_or(ProviderState::Gone))
        }

        async fn start(
            &self,
            handle: &ProviderHandle,
            _idle_seconds: u32,
        ) -> Result<ProviderHandle, ProviderError> {
            self.machines.lock().insert(handle.provider_id.clone(), ProviderState::Running);
            Ok(ProviderHandle {
                control_secret: Secret::new("ctl-2"),
                viewer_secret: Secret::new("view-2"),
                ..handle.clone()
            })
        }

        async fn keep_awake(&self, _handle: &ProviderHandle, _idle_seconds: u32) {}

        async fn stop(&self, handle: &ProviderHandle) -> Result<(), ProviderError> {
            self.machines.lock().insert(handle.provider_id.clone(), ProviderState::Asleep);
            Ok(())
        }

        async fn delete(&self, handle: &ProviderHandle) -> Result<(), ProviderError> {
            if *self.fail_delete.lock() {
                return Err(ProviderError::Unavailable("fake: delete refused".into()));
            }
            self.deletes.lock().push(handle.provider_id.clone());
            self.machines.lock().remove(&handle.provider_id);
            Ok(())
        }

        async fn exec(
            &self,
            _handle: &ProviderHandle,
            request: ExecRequest,
        ) -> Result<Output, ProviderError> {
            self.execs.lock().push(request);
            let replies = self.replies.lock();
            let n = self.execs.lock().len();
            Ok(replies.get(n - 1).or(replies.last()).cloned().unwrap_or(Output {
                stdout: String::new(),
                stderr: String::new(),
                exit_code: 0,
            }))
        }

        async fn viewer_target(
            &self,
            handle: &ProviderHandle,
            port: u16,
        ) -> Result<ViewerTarget, ProviderError> {
            Ok(ViewerTarget {
                tls: false,
                host: format!("{}.fake", handle.provider_id),
                port,
                headers: vec![],
            })
        }

        async fn list_owned(&self) -> Result<Vec<String>, ProviderError> {
            Ok(self.machines.lock().keys().cloned().collect())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::computer::Secret;

    #[tokio::test]
    async fn a_command_reaches_the_guest_as_a_login_shell_with_the_groups_credentials() {
        // What the model typed is one argument, and the credentials are in
        // the environment. Neither is ever part of the other.
        let provider = Arc::new(fake::FakeProvider::default());
        let handle = ProviderHandle {
            computer: ComputerId::new(),
            provider_id: "m".into(),
            control_secret: Secret::new(""),
            viewer_secret: Secret::new(""),
        };
        let env = BTreeMap::from([("TOKEN".to_string(), "sentinel".to_string())]);
        let machine = Machine::new(provider.clone(), handle, env, 0);

        machine.run("echo $TOKEN; ls 'a b'").await.unwrap();
        machine.run_plain("pgrep -x Xvfb").await.unwrap();

        let execs = provider.execs.lock();
        assert_eq!(execs[0].argv, vec!["/bin/bash", "-l", "-c", "echo $TOKEN; ls 'a b'"]);
        assert_eq!(execs[0].cwd, "/home/user");
        assert_eq!(execs[0].env.get("TOKEN").map(String::as_str), Some("sentinel"));
        assert!(execs[1].env.is_empty(), "desktop maintenance never carries credentials");
    }
}
