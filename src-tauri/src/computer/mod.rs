//! Agents' computers, behind a provider boundary.
//!
//! The runtime asks for "this agent's machine" and gets a `Machine` it can run
//! commands on. Who actually runs that machine — E2B today, a local container
//! runtime later — is a `ComputerProvider`, and nothing above this module
//! knows which one it got.

pub mod apple;
pub mod cli;
pub mod desktop;
pub mod e2b;
pub mod image;
pub mod provider;

use std::collections::{BTreeMap, HashMap, HashSet};
use std::sync::atomic::{AtomicU16, Ordering};
use std::sync::Arc;

use parking_lot::{Mutex, RwLock};
use serde::{Deserialize, Serialize};

use crate::config::AppConfig;
use crate::db::Store;
use crate::domain::agent::AgentCard;
use crate::domain::computer::{ComputerRecord, Provider, RecordState, Secret};
use crate::domain::ids::{AgentId, ComputerId};
use crate::domain::now_ms;
use e2b::E2bProvider;
use provider::{
    ComputerProvider, CreateComputer, ExecRequest, Output, ProviderError, ProviderHandle,
    ProviderState, ViewerTarget,
};

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

/// An agent's computer as the operator's window sees it: an id of this app's
/// own, who runs it, what it is doing, and somewhere to watch it.
///
/// Deliberately not `ComputerRecord`. That row carries the tokens that reach a
/// machine, and this is the only shape of it that crosses IPC.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Computer {
    pub id: ComputerId,
    pub provider: Provider,
    /// `running`, or `asleep` with its disk intact.
    pub state: String,
    /// Absent until the desktop inside the machine is actually serving.
    pub vnc_url: Option<String>,
}

/// What `ensure` had to do, because only two of the three are worth telling the
/// window about.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Provisioned {
    /// A new machine and a new row, so the agent's card now names a computer
    /// where it named none.
    Created,
    /// The same disk, woken. The card is unchanged, but what the pane can draw
    /// is not.
    Woken,
    /// Already running, which is nearly every call: a command, a page, a
    /// screenshot. Nothing changed and nothing needs redrawing.
    Reused,
}

/// Why an agent has no machine to work on. Each variant is a different next
/// step: set a key, wait and retry, or read the message.
#[derive(Debug, thiserror::Error)]
pub enum ComputerError {
    #[error(
        "no computer provider is configured; add an E2B API key in app settings to give agents a computer"
    )]
    Unconfigured,
    #[error(transparent)]
    Provider(#[from] ProviderError),
    #[error(transparent)]
    Store(#[from] crate::db::StoreError),
    #[error("the computer could not be recorded and was released ({0}); try again")]
    Recording(String),
}

struct Inner {
    store: Store,
    config: Arc<RwLock<AppConfig>>,
    /// One lock per agent, so an operator click and an agent tool call cannot
    /// make two machines. Held across the whole `ensure`, including the create.
    locks: Mutex<HashMap<AgentId, Arc<tokio::sync::Mutex<()>>>>,
    /// The provider last built, with the API key it was built from. Kept
    /// because the viewer resolves a target per proxied request and one noVNC
    /// page is fifty of those plus a WebSocket: building a provider there was
    /// a connection pool made and thrown away per asset, and a provider that
    /// drives a CLI would be a process per asset. Rebuilt when the key differs,
    /// which is how a settings change takes effect.
    e2b_provider: RwLock<Option<(String, Arc<dyn ComputerProvider>)>>,
    /// Loopback port of the viewer proxy. Zero until it is listening.
    viewer_port: AtomicU16,
    #[cfg(test)]
    injected: Option<Arc<dyn ComputerProvider>>,
}

/// Every agent's computer: who runs it, what state it is in, and the row that
/// remembers it between restarts.
///
/// The runtime asks for a `Machine` and gets one; which provider made it, and
/// whether it had to be created, woken or simply used, is settled here. That is
/// deliberate: a machine made in one place and recorded in another is how a
/// resource ends up running with nothing referring to it, and this app has done
/// that.
#[derive(Clone)]
pub struct ComputerManager {
    inner: Arc<Inner>,
}

impl ComputerManager {
    pub fn new(store: Store, config: Arc<RwLock<AppConfig>>) -> Self {
        Self {
            inner: Arc::new(Inner {
                store,
                config,
                locks: Mutex::new(HashMap::new()),
                e2b_provider: RwLock::new(None),
                viewer_port: AtomicU16::new(0),
                #[cfg(test)]
                injected: None,
            }),
        }
    }

    /// A manager driving a provider the caller supplies, for tests that must
    /// not reach a network.
    #[cfg(test)]
    pub(crate) fn with_provider(
        store: Store,
        config: Arc<RwLock<AppConfig>>,
        provider: Arc<dyn ComputerProvider>,
    ) -> Self {
        Self {
            inner: Arc::new(Inner {
                store,
                config,
                locks: Mutex::new(HashMap::new()),
                e2b_provider: RwLock::new(None),
                viewer_port: AtomicU16::new(0),
                injected: Some(provider),
            }),
        }
    }

    /// Where the viewer proxy is listening, once it is. Held here because the
    /// address of a machine's desktop is built from it and nothing above the
    /// boundary should have to know that.
    pub fn set_viewer_port(&self, port: u16) {
        self.inner.viewer_port.store(port, Ordering::SeqCst);
    }

    pub fn viewer_port(&self) -> u16 {
        self.inner.viewer_port.load(Ordering::SeqCst)
    }

    /// This agent's machine, made, woken or reused. `env` is the group's
    /// credentials, which every command on the returned machine carries.
    ///
    /// Says which of the three it was, because the window only has to redraw
    /// for two of them and this is called on every command an agent runs.
    pub async fn ensure(
        &self,
        card: &AgentCard,
        env: BTreeMap<String, String>,
    ) -> Result<(Machine, Provisioned), ComputerError> {
        let lock = self.lock_for(card.id);
        let _held = lock.lock().await;

        if let Some(record) = self.inner.store.computer_for_agent(card.id)? {
            match (record.state, Self::handle_of(&record)) {
                (RecordState::Ready, Some(handle)) => {
                    let provider = self.provider(record.provider)?;
                    // A provider that cannot answer preserves the row and the
                    // disk: not knowing is not the same as gone.
                    match provider.inspect(&handle).await? {
                        ProviderState::Running => {
                            // Every use pushes the sleep deadline back, which is
                            // what makes the timeout idle time rather than a
                            // lifetime.
                            provider.keep_awake(&handle, self.idle_seconds()).await;
                            self.inner.store.touch_computer(record.id, now_ms())?;
                            return Ok((self.machine(provider, handle, env), Provisioned::Reused));
                        }
                        ProviderState::Asleep => {
                            // Woken rather than replaced. The disk is the point:
                            // a browser that was signed in still is.
                            let woken = provider.start(&handle, self.idle_seconds()).await?;
                            // The whole handle is reissued on waking, the
                            // identifier included, so the stored one is now
                            // wrong. Keeping it is a machine that is running and
                            // unreachable, which looks exactly like a broken
                            // one, and a running machine the sweep can no longer
                            // see anything claiming.
                            if let Err(err) = self.inner.store.set_computer_handle(
                                record.id,
                                &woken.provider_id,
                                &woken.control_secret,
                                &woken.viewer_secret,
                            ) {
                                tracing::error!(
                                    %err,
                                    computer = %record.id,
                                    "could not record the woken machine's handle"
                                );
                            }
                            self.inner.store.touch_computer(record.id, now_ms())?;
                            return Ok((self.machine(provider, woken, env), Provisioned::Woken));
                        }
                        ProviderState::Gone => self.inner.store.delete_computer(record.id)?,
                    }
                }
                (RecordState::DeletePending, _) => {
                    // Making a second machine now would leave the first one
                    // billing with nothing referring to it. The sweep finishes
                    // the removal; the agent is told to come back.
                    return Err(ProviderError::Unavailable(
                        "this agent's previous computer is still being removed; try again in a \
                         moment"
                            .into(),
                    )
                    .into());
                }
                // Still provisioning, or ready and naming nothing: a crash
                // between the insert and the create. Whatever it made, if
                // anything, is unclaimed and the sweep releases it.
                _ => self.inner.store.delete_computer(record.id)?,
            }
        }

        let provider = self.provider_for_new()?;
        let id = ComputerId::new();
        let now = now_ms();
        // Written down before it exists: a resource made with nothing claiming
        // it is invisible to this app and bills exactly like one in use.
        self.inner.store.insert_computer(&ComputerRecord {
            id,
            agent_id: card.id,
            provider: provider.kind(),
            provider_id: None,
            control_secret: Secret::default(),
            viewer_secret: Secret::default(),
            image_ref: String::new(),
            state: RecordState::Provisioning,
            last_used_at: now,
            created_at: now,
            updated_at: now,
        })?;

        let handle = match provider
            .create(&CreateComputer {
                computer: id,
                agent: card.id,
                agent_name: card.name.clone(),
                idle_seconds: self.idle_seconds(),
            })
            .await
        {
            Ok(handle) => handle,
            Err(err) => {
                // Nothing was made, so the claim is a row the sweep would trip
                // over on every startup.
                if let Err(err) = self.inner.store.delete_computer(id) {
                    tracing::warn!(
                        %err,
                        computer = %id,
                        "could not clear the record of a computer that was never made"
                    );
                }
                return Err(err.into());
            }
        };

        // A machine that cannot be written down is a machine nobody can reach
        // and nobody will stop paying for, so it is killed rather than left.
        // Failing to read the create reply once already orphaned three of them.
        if let Err(err) = self.inner.store.set_computer_ready(
            id,
            &handle.provider_id,
            &handle.control_secret,
            &handle.viewer_secret,
        ) {
            tracing::error!(%err, computer = %id, "could not record a computer; releasing it");
            let _ = provider.delete(&handle).await;
            let _ = self.inner.store.delete_computer(id);
            return Err(ComputerError::Recording(err.to_string()));
        }

        Ok((self.machine(provider, handle, env), Provisioned::Created))
    }

    /// The machine only if it is already running; never wakes, never creates.
    ///
    /// For sign-in scans, which happen because somebody opened a pane. Waking a
    /// machine to refresh a list would cost money every time anyone looked at
    /// an agent.
    pub async fn if_running(&self, agent: AgentId) -> Result<Option<Machine>, ComputerError> {
        let Some((record, handle)) = self.ready(agent)? else {
            return Ok(None);
        };
        let provider = match self.provider(record.provider) {
            Ok(provider) => provider,
            // Nothing to ask. Whoever wanted the scan already holds the last
            // answer, and telling them to configure a provider is not the reply
            // to "what is this browser signed in to".
            Err(ComputerError::Unconfigured) => return Ok(None),
            Err(err) => return Err(err),
        };
        match provider.inspect(&handle).await {
            Ok(ProviderState::Running) => Ok(Some(self.machine(provider, handle, BTreeMap::new()))),
            // Asleep, gone, or a provider that would not answer: a scan is
            // best-effort and none of those is worth failing whatever asked.
            _ => Ok(None),
        }
    }

    /// What the pane shows. `None` if the agent has no computer or it is gone,
    /// in which case the row is cleared so the pane offers a new one.
    pub async fn describe(&self, agent: AgentId) -> Result<Option<Computer>, ComputerError> {
        let Some((record, handle)) = self.ready(agent)? else {
            return Ok(None);
        };
        let provider = self.provider(record.provider)?;
        let shown = |state: &str, vnc_url| {
            Some(Computer {
                id: record.id,
                provider: record.provider,
                state: state.to_string(),
                vnc_url,
            })
        };

        match provider.inspect(&handle).await? {
            ProviderState::Gone => {
                // A reclaimed machine leaves a dangling row. Clearing it turns
                // a dead end into an offer to make a new one.
                self.inner.store.delete_computer(record.id)?;
                Ok(None)
            }
            // Asked for only when the machine is up: finding out costs a
            // command, and a command is the one thing that would wake it.
            ProviderState::Running => {
                let machine = self.machine(provider, handle, BTreeMap::new());
                Ok(shown("running", machine.vnc_url().await))
            }
            ProviderState::Asleep => Ok(shown("asleep", None)),
        }
    }

    /// Puts a machine to sleep, keeping its disk.
    pub async fn sleep(&self, agent: AgentId) -> Result<Option<Computer>, ComputerError> {
        let lock = self.lock_for(agent);
        let held = lock.lock().await;

        let Some((record, handle)) = self.ready(agent)? else {
            return Ok(None);
        };
        self.provider(record.provider)?.stop(&handle).await?;
        // Released before describing: the answer is a fresh look at the
        // machine, and holding the lock through it would block a turn behind an
        // operator's click for no reason.
        drop(held);
        self.describe(agent).await
    }

    /// Explicit destroy: the row is cleared only once the provider says the
    /// machine is gone, so a failure leaves something to retry rather than an
    /// agent whose disk is unreachable and unaccounted for.
    pub async fn destroy(&self, agent: AgentId) -> Result<(), ComputerError> {
        let lock = self.lock_for(agent);
        let _held = lock.lock().await;

        let Some(record) = self.inner.store.computer_for_agent(agent)? else {
            return Ok(());
        };
        if let Some(handle) = Self::handle_of(&record) {
            self.provider(record.provider)?.delete(&handle).await?;
        }
        self.inner.store.delete_computer(record.id)?;
        Ok(())
    }

    /// Best-effort teardown when the agent itself is going.
    ///
    /// A deleted agent cannot be asked to tidy up after itself and its deletion
    /// must not fail because a provider was unreachable, so a failure here is
    /// written down as `deletePending` and retried at the next startup.
    pub async fn release(&self, agent: AgentId) {
        let lock = self.lock_for(agent);
        let _held = lock.lock().await;

        let record = match self.inner.store.computer_for_agent(agent) {
            Ok(Some(record)) => record,
            Ok(None) => return,
            Err(err) => {
                tracing::warn!(%err, %agent, "could not look up the agent's computer to release it");
                return;
            }
        };

        if let Some(handle) = Self::handle_of(&record) {
            let released = match self.provider(record.provider) {
                Ok(provider) => provider.delete(&handle).await.map_err(ComputerError::from),
                Err(err) => Err(err),
            };
            if let Err(err) = released {
                tracing::warn!(
                    %err,
                    computer = %record.id,
                    "could not destroy a deleted agent's computer; it will be retried at startup"
                );
                if let Err(err) =
                    self.inner.store.set_computer_state(record.id, RecordState::DeletePending)
                {
                    tracing::warn!(%err, computer = %record.id, "could not mark a computer for removal");
                }
                return;
            }
        }

        if let Err(err) = self.inner.store.delete_computer(record.id) {
            tracing::warn!(%err, computer = %record.id, "could not clear a released computer");
        }
    }

    /// Startup reconciliation, and the answer to how many resources it freed.
    ///
    /// Two halves, because a machine can be lost in two directions: a row whose
    /// removal never finished, and a resource whose row never existed. The
    /// second is the expensive one — a machine nothing refers to bills exactly
    /// like one in use and is invisible from inside the app.
    pub async fn sweep(&self) -> Result<usize, ComputerError> {
        let mut released = 0;

        for listed in self.inner.store.list_computers()? {
            // Under the agent's own lock, and re-read once it is held. `ensure`
            // keeps a provisioning row for the whole of a create, and the
            // startup sweep runs alongside a scheduler that can have a routine
            // due: deleting that row from under a turn already in flight would
            // destroy the machine that turn had just made.
            let lock = self.lock_for(listed.agent_id);
            let _held = lock.lock().await;
            let Some(record) = self.inner.store.computer(listed.id)? else {
                continue;
            };

            match (record.state, Self::handle_of(&record)) {
                (RecordState::DeletePending, Some(handle)) => {
                    let deleted = match self.provider(record.provider) {
                        Ok(provider) => provider.delete(&handle).await.map_err(ComputerError::from),
                        Err(err) => Err(err),
                    };
                    match deleted {
                        Ok(()) => {
                            self.inner.store.delete_computer(record.id)?;
                            released += 1;
                        }
                        // Left exactly as it is, to be tried again next time.
                        Err(err) => tracing::warn!(
                            %err,
                            computer = %record.id,
                            "a computer marked for removal is still there"
                        ),
                    }
                }
                // A row that names no machine: either the create never
                // happened, or what it made is unclaimed and caught below.
                (RecordState::Provisioning, None) | (RecordState::DeletePending, None) => {
                    self.inner.store.delete_computer(record.id)?
                }
                _ => {}
            }
        }

        // Only a provider that is configured can be asked, and E2B is the only
        // one there is. Anything it made that no row refers to is a leak.
        if let Ok(provider) = self.provider(Provider::E2b) {
            let owned = provider.list_owned().await?;
            // Read after the list, never before: a machine made while the list
            // was in flight is absent from it, but its row is here, so nothing
            // is deleted for being younger than the question. What is left is a
            // create that returned before the list and was recorded after this
            // read, which is a window of microseconds and the shape the sweep
            // has always had.
            let claimed: HashSet<String> = self
                .inner
                .store
                .list_computers()?
                .into_iter()
                .filter_map(|record| record.provider_id)
                .collect();

            for id in owned {
                if claimed.contains(&id) {
                    continue;
                }
                tracing::info!(%id, "releasing a computer no agent refers to");
                // The handle is an address and nothing more here: an unclaimed
                // resource has no row, so there are no secrets to delete it
                // with and none are needed.
                let handle = ProviderHandle {
                    computer: ComputerId::new(),
                    provider_id: id,
                    control_secret: Secret::default(),
                    viewer_secret: Secret::default(),
                };
                if provider.delete(&handle).await.is_ok() {
                    released += 1;
                }
            }
        }

        Ok(released)
    }

    /// This agent's computer if it is ready to be used, with the handle that
    /// reaches it. A row that is provisioning or on its way out is not one
    /// anybody can look at or act on.
    fn ready(
        &self,
        agent: AgentId,
    ) -> Result<Option<(ComputerRecord, ProviderHandle)>, ComputerError> {
        let Some(record) = self.inner.store.computer_for_agent(agent)? else {
            return Ok(None);
        };
        if record.state != RecordState::Ready {
            return Ok(None);
        }
        Ok(Self::handle_of(&record).map(|handle| (record, handle)))
    }

    fn machine(
        &self,
        provider: Arc<dyn ComputerProvider>,
        handle: ProviderHandle,
        env: BTreeMap<String, String>,
    ) -> Machine {
        Machine::new(provider, handle, env, self.viewer_port())
    }

    /// How a provider finds this machine again, or `None` while there is
    /// nothing yet for it to find.
    fn handle_of(record: &ComputerRecord) -> Option<ProviderHandle> {
        Some(ProviderHandle {
            computer: record.id,
            provider_id: record.provider_id.clone()?,
            control_secret: record.control_secret.clone(),
            viewer_secret: record.viewer_secret.clone(),
        })
    }

    /// Whoever runs machines of this kind, if this build can.
    ///
    /// The one built last is kept, because this is on the viewer's path: the
    /// proxy resolves a target for every request a desktop page makes. The key
    /// it was built from is what the cache is keyed on, so an operator who
    /// changes it in settings gets a provider that uses it on the next call.
    fn provider(&self, which: Provider) -> Result<Arc<dyn ComputerProvider>, ComputerError> {
        #[cfg(test)]
        if let Some(injected) = &self.inner.injected {
            return Ok(injected.clone());
        }
        match which {
            Provider::E2b => {
                let key = self.inner.config.read().e2b.api_key.trim().to_string();
                if let Some((built_from, provider)) = self.inner.e2b_provider.read().as_ref() {
                    if *built_from == key {
                        return Ok(provider.clone());
                    }
                }
                let provider = E2bProvider::new(&key)
                    .map(|provider| Arc::new(provider) as Arc<dyn ComputerProvider>)
                    .ok_or(ComputerError::Unconfigured)?;
                *self.inner.e2b_provider.write() = Some((key, provider.clone()));
                Ok(provider)
            }
            // Nothing in this build makes one, so only a row written by a newer
            // one can name it, and there is nothing here that could drive it.
            // The Apple Container provider is the next task.
            Provider::AppleContainer => Err(ComputerError::Unconfigured),
        }
    }

    /// Who runs a machine this app is about to make. One provider today; PR B
    /// turns this into the automatic resolution.
    fn provider_for_new(&self) -> Result<Arc<dyn ComputerProvider>, ComputerError> {
        self.provider(Provider::E2b)
    }

    /// How long a machine may sit unused. Pushed back on every use, so what
    /// expires is idle time rather than a lifetime.
    fn idle_seconds(&self) -> u32 {
        self.inner.config.read().computer.idle_minutes.max(1) * 60
    }

    fn lock_for(&self, agent: AgentId) -> Arc<tokio::sync::Mutex<()>> {
        self.inner.locks.lock().entry(agent).or_default().clone()
    }
}

/// The proxy is handed a computer in a URL and asks here, so nothing that
/// reaches a machine has to travel through the webview.
#[async_trait::async_trait]
impl crate::proxy::ViewerResolver for ComputerManager {
    async fn viewer_target(&self, computer: &str, port: u16) -> Option<ViewerTarget> {
        let id: ComputerId = computer.parse().ok()?;
        let record = self.inner.store.computer(id).ok()??;
        if record.state != RecordState::Ready {
            return None;
        }
        let handle = Self::handle_of(&record)?;
        match self.provider(record.provider).ok()?.viewer_target(&handle, port).await {
            Ok(target) => Some(target),
            Err(err) => {
                // The proxy can only answer 404, which reads as "no such
                // machine". The reason it could not be reached is only ever
                // said here.
                tracing::warn!(
                    %err,
                    computer = %id,
                    "the viewer has nowhere to send this computer's desktop"
                );
                None
            }
        }
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
        /// What `probe` answers. `None` is ready: a fake that had to be told it
        /// works before it would work is one every existing test would have to
        /// set up.
        pub probe: Mutex<Option<ProviderStatus>>,
    }

    #[async_trait::async_trait]
    impl ComputerProvider for FakeProvider {
        fn kind(&self) -> Provider {
            Provider::E2b
        }

        async fn probe(&self) -> ProviderStatus {
            self.probe.lock().clone().unwrap_or_else(|| ProviderStatus::ready("fake: ready"))
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

        /// Wakes it under a *different* identifier, which is what the trait
        /// allows and what E2B's resume reply actually does: everything on the
        /// handle is reissued, and a caller that keeps the old one is holding
        /// the address of a machine that is not there.
        async fn start(
            &self,
            handle: &ProviderHandle,
            _idle_seconds: u32,
        ) -> Result<ProviderHandle, ProviderError> {
            let woken = format!("{}-woken", handle.provider_id);
            let mut machines = self.machines.lock();
            machines.remove(&handle.provider_id);
            machines.insert(woken.clone(), ProviderState::Running);
            Ok(ProviderHandle {
                provider_id: woken,
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
    use std::time::Duration;

    use super::provider::ProviderState;
    use super::*;
    use crate::config::AppConfig;
    use crate::db::Store;
    use crate::domain::agent::{AgentCard, CleanDraft};
    use crate::domain::computer::{ComputerRecord, Provider, RecordState, Secret};

    fn draft(name: &str) -> CleanDraft {
        CleanDraft {
            group_id: None,
            name: name.into(),
            avatar: "avocado".into(),
            color: "#7fb069".into(),
            model: "test/model".into(),
            system_prompt: "You coordinate the kitchen.".into(),
            skills: vec!["delegation".into()],
        }
    }

    fn setup() -> (ComputerManager, Arc<fake::FakeProvider>, Store, AgentCard, tempfile::TempDir) {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::open(&dir.path().join("guac.db")).unwrap();
        let card = store.create_agent(&draft("Manager")).unwrap();
        let provider = Arc::new(fake::FakeProvider::default());
        let config = Arc::new(parking_lot::RwLock::new(AppConfig::default()));
        let manager = ComputerManager::with_provider(store.clone(), config, provider.clone());
        (manager, provider, store, card, dir)
    }

    #[tokio::test]
    async fn two_callers_at_once_get_one_machine() {
        // An operator click and an agent tool call land together. Without the
        // per-agent lock both saw "no computer" and both created one.
        let (manager, provider, _store, card, _dir) = setup();
        *provider.create_delay.lock() = Some(Duration::from_millis(50));
        let (a, b) = tokio::join!(
            manager.ensure(&card, BTreeMap::new()),
            manager.ensure(&card, BTreeMap::new())
        );
        let (first, made) = a.unwrap();
        let (second, then) = b.unwrap();
        assert_eq!(first.id(), second.id());
        assert_eq!(*provider.creates.lock(), 1);
        // The loser of the race waited and found a machine, which is exactly
        // what it should report: one create, one thing worth redrawing.
        assert_eq!((made, then), (Provisioned::Created, Provisioned::Reused));
    }

    #[tokio::test]
    async fn a_machine_that_cannot_be_recorded_is_released_rather_than_left() {
        // Failing to read the create reply once already orphaned three
        // sandboxes; a row that cannot be written is the same failure.
        let (manager, provider, store, card, _dir) = setup();
        *provider.create_delay.lock() = Some(Duration::from_millis(50));

        // The row goes out from under the create while it is in flight, which
        // is the shape of every way this fails: the machine exists and there
        // is nowhere left to write it down.
        let vanish = {
            let store = store.clone();
            tokio::spawn(async move {
                tokio::time::sleep(Duration::from_millis(10)).await;
                for record in store.list_computers().unwrap() {
                    store.delete_computer(record.id).unwrap();
                }
            })
        };

        let Err(err) = manager.ensure(&card, BTreeMap::new()).await else {
            panic!("a machine nothing recorded must not be handed out");
        };
        vanish.await.unwrap();
        assert!(matches!(err, ComputerError::Recording(_)), "{err}");
        assert_eq!(provider.deletes.lock().len(), 1, "the machine was killed");
    }

    #[tokio::test]
    async fn a_sleeping_machine_is_woken_and_the_whole_reissued_handle_is_kept() {
        let (manager, provider, store, card, _dir) = setup();
        let (first, made) = manager.ensure(&card, BTreeMap::new()).await.unwrap();
        let born = store.computer(first.id()).unwrap().unwrap().provider_id;
        manager.sleep(card.id).await.unwrap();

        let (again, woken) = manager.ensure(&card, BTreeMap::new()).await.unwrap();
        assert_eq!(first.id(), again.id(), "the disk is the point");
        assert_eq!(*provider.creates.lock(), 1);
        assert_eq!((made, woken), (Provisioned::Created, Provisioned::Woken));

        // Waking reissues the identifier as well as the tokens. A row that kept
        // the old one would name a machine that is not there, and the running
        // one would look like an orphan to the sweep.
        let record = store.computer(first.id()).unwrap().unwrap();
        assert_eq!(record.provider_id, born.map(|id| format!("{id}-woken")));
        assert_eq!(record.control_secret.expose(), "ctl-2");
        assert_eq!(record.viewer_secret.expose(), "view-2");

        // And the row that was written is the one that answers next time: found
        // running, not created again.
        let (third, reused) = manager.ensure(&card, BTreeMap::new()).await.unwrap();
        assert_eq!(third.id(), first.id());
        assert_eq!(reused, Provisioned::Reused);
        assert_eq!(*provider.creates.lock(), 1, "the woken machine was found, not replaced");
    }

    #[tokio::test]
    async fn a_machine_the_provider_reports_gone_is_replaced_and_the_old_row_cleared() {
        let (manager, provider, store, card, _dir) = setup();
        let (first, _) = manager.ensure(&card, BTreeMap::new()).await.unwrap();
        provider.machines.lock().clear();
        let (second, made) = manager.ensure(&card, BTreeMap::new()).await.unwrap();
        assert_eq!(made, Provisioned::Created);
        assert_ne!(first.id(), second.id());
        assert!(store.computer(first.id()).unwrap().is_none());
        assert_eq!(*provider.creates.lock(), 2);
    }

    #[tokio::test]
    async fn describe_never_wakes_and_clears_a_gone_machine() {
        let (manager, provider, store, card, _dir) = setup();
        let (machine, _) = manager.ensure(&card, BTreeMap::new()).await.unwrap();
        manager.sleep(card.id).await.unwrap();
        let shown = manager.describe(card.id).await.unwrap().unwrap();
        assert_eq!(shown.state, "asleep");
        assert_eq!(shown.provider, Provider::E2b);
        assert_eq!(
            provider.machines.lock()[&format!("fake-{}", machine.id().short())],
            ProviderState::Asleep,
            "describing did not wake it"
        );
        provider.machines.lock().clear();
        assert!(manager.describe(card.id).await.unwrap().is_none());
        assert!(store.computer(machine.id()).unwrap().is_none());
    }

    #[tokio::test]
    async fn if_running_returns_nothing_for_a_sleeping_machine() {
        let (manager, _provider, _store, card, _dir) = setup();
        assert!(manager.if_running(card.id).await.unwrap().is_none(), "no computer yet");
        manager.ensure(&card, BTreeMap::new()).await.unwrap();
        assert!(manager.if_running(card.id).await.unwrap().is_some());
        manager.sleep(card.id).await.unwrap();
        assert!(
            manager.if_running(card.id).await.unwrap().is_none(),
            "a sign-in scan must not wake anything"
        );
    }

    #[tokio::test]
    async fn a_failed_explicit_destroy_keeps_the_row_and_a_failed_release_marks_it_pending() {
        let (manager, provider, store, card, _dir) = setup();
        let (machine, _) = manager.ensure(&card, BTreeMap::new()).await.unwrap();
        *provider.fail_delete.lock() = true;
        assert!(manager.destroy(card.id).await.is_err());
        assert_eq!(store.computer(machine.id()).unwrap().unwrap().state, RecordState::Ready);
        manager.release(card.id).await;
        assert_eq!(
            store.computer(machine.id()).unwrap().unwrap().state,
            RecordState::DeletePending
        );
        *provider.fail_delete.lock() = false;
        assert_eq!(manager.sweep().await.unwrap(), 1, "the retry at startup finishes the job");
        assert!(store.computer(machine.id()).unwrap().is_none());
    }

    #[tokio::test]
    async fn the_sweep_releases_what_nothing_claims_and_leaves_what_something_does() {
        let (manager, provider, store, card, _dir) = setup();
        let (kept, _) = manager.ensure(&card, BTreeMap::new()).await.unwrap();
        provider.machines.lock().insert("fake-orphan".into(), ProviderState::Running);
        // A provisioning row that never got its provider id is a crash mid-create.
        let stranded = store.create_agent(&draft("Chef")).unwrap();
        store
            .insert_computer(&ComputerRecord {
                id: ComputerId::new(),
                agent_id: stranded.id,
                provider: Provider::E2b,
                provider_id: None,
                control_secret: Secret::default(),
                viewer_secret: Secret::default(),
                image_ref: String::new(),
                state: RecordState::Provisioning,
                last_used_at: 0,
                created_at: 0,
                updated_at: 0,
            })
            .unwrap();

        assert_eq!(manager.sweep().await.unwrap(), 1);
        assert_eq!(*provider.deletes.lock(), vec!["fake-orphan".to_string()]);
        assert!(store.computer(kept.id()).unwrap().is_some());
        assert_eq!(store.list_computers().unwrap().len(), 1, "the stale provisioning row is gone");
    }

    #[tokio::test]
    async fn the_sweep_leaves_alone_a_machine_that_is_being_created_under_it() {
        // The startup sweep runs beside a scheduler that can have a routine due
        // in the same second. `ensure` holds a provisioning row across the whole
        // create, and a sweep that took it away made `set_computer_ready` find
        // no row, which released the machine the turn had just been given.
        let (manager, provider, store, card, _dir) = setup();
        *provider.create_delay.lock() = Some(Duration::from_millis(80));

        let sweeping = {
            let manager = manager.clone();
            tokio::spawn(async move {
                tokio::time::sleep(Duration::from_millis(20)).await;
                manager.sweep().await
            })
        };

        let (machine, made) = manager
            .ensure(&card, BTreeMap::new())
            .await
            .expect("a create in flight must survive the sweep");
        assert_eq!(made, Provisioned::Created);
        assert_eq!(sweeping.await.unwrap().unwrap(), 0, "there was nothing to release");
        assert!(store.computer(machine.id()).unwrap().is_some(), "the row survived");
        assert!(provider.deletes.lock().is_empty(), "the machine survived");
    }

    #[tokio::test]
    async fn a_scan_with_no_provider_configured_says_nothing_rather_than_failing() {
        // The key can be removed from settings while an agent still has a row.
        // Whoever asked for a scan holds the last answer already, and "add an
        // API key" is not a reply to "what is this browser signed in to".
        let dir = tempfile::tempdir().unwrap();
        let store = Store::open(&dir.path().join("guac.db")).unwrap();
        let card = store.create_agent(&draft("Manager")).unwrap();
        store
            .insert_computer(&ComputerRecord {
                id: ComputerId::new(),
                agent_id: card.id,
                provider: Provider::E2b,
                provider_id: Some("sbx".into()),
                control_secret: Secret::new("ctl"),
                viewer_secret: Secret::new("view"),
                image_ref: String::new(),
                state: RecordState::Ready,
                last_used_at: 0,
                created_at: 0,
                updated_at: 0,
            })
            .unwrap();
        let manager =
            ComputerManager::new(store, Arc::new(parking_lot::RwLock::new(AppConfig::default())));

        assert!(manager.if_running(card.id).await.unwrap().is_none());
    }

    #[test]
    fn one_provider_is_kept_and_rebuilt_only_when_the_key_changes() {
        // The viewer resolves a target per proxied request, and a single noVNC
        // page is around fifty of them plus a WebSocket. Building a provider
        // per call made and dropped an HTTP client, and its connection pool,
        // for every asset on that page.
        let dir = tempfile::tempdir().unwrap();
        let store = Store::open(&dir.path().join("guac.db")).unwrap();
        let config = Arc::new(parking_lot::RwLock::new(AppConfig::default()));
        config.write().e2b.api_key = "e2b_first".into();
        let manager = ComputerManager::new(store, config.clone());

        let first = manager.provider(Provider::E2b).unwrap();
        let again = manager.provider(Provider::E2b).unwrap();
        assert!(Arc::ptr_eq(&first, &again), "the same key is the same provider");

        // An operator changing the key in settings is the one thing that must
        // not be answered from the cache: the kept provider holds the old one.
        config.write().e2b.api_key = "e2b_second".into();
        let rebuilt = manager.provider(Provider::E2b).unwrap();
        assert!(!Arc::ptr_eq(&first, &rebuilt), "a new key is a new provider");
    }

    #[tokio::test]
    async fn without_a_provider_the_answer_is_unconfigured_not_a_crash() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::open(&dir.path().join("guac.db")).unwrap();
        let card = store.create_agent(&draft("Manager")).unwrap();
        let manager =
            ComputerManager::new(store, Arc::new(parking_lot::RwLock::new(AppConfig::default())));
        let Err(err) = manager.ensure(&card, BTreeMap::new()).await else {
            panic!("there is nothing to make a machine with");
        };
        assert!(matches!(err, ComputerError::Unconfigured), "{err}");
        assert!(
            err.to_string().contains("E2B API key"),
            "the operator has to be told what to do about it: {err}"
        );
        assert!(manager.describe(card.id).await.unwrap().is_none(), "no computer, no error");
    }

    #[tokio::test]
    async fn the_viewer_is_told_where_a_computer_is_and_nothing_about_one_it_does_not_know() {
        use crate::proxy::ViewerResolver;

        let (manager, _provider, _store, card, _dir) = setup();
        let (machine, _) = manager.ensure(&card, BTreeMap::new()).await.unwrap();

        let target =
            manager.viewer_target(&machine.id().to_string(), 6080).await.expect("a target");
        assert_eq!(target.host, format!("fake-{}.fake", machine.id().short()));
        assert_eq!(target.port, 6080);

        // An address that is not a computer id, and one that is nobody's, are
        // both simply not registered: the proxy answers 404 rather than
        // reaching for a machine somebody guessed at.
        assert!(manager.viewer_target("not-a-uuid", 6080).await.is_none());
        assert!(manager.viewer_target(&ComputerId::new().to_string(), 6080).await.is_none());
    }

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
