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
use crate::domain::computer::{
    ComputerAccess, ComputerRecord, Provider, ProviderChoice, RecordState, Secret,
};
use crate::domain::ids::{AgentId, ComputerId};
use crate::domain::now_ms;
use apple::AppleContainer;
use e2b::E2bProvider;
use provider::{
    ComputerProvider, CreateComputer, ExecRequest, Output, ProviderError, ProviderHandle,
    ProviderReadiness, ProviderState, ProviderStatus, ViewerTarget,
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
        //
        // `path` is spelled out because noVNC builds its WebSocket address
        // from the page's host and `/` + path, not from the page's own
        // directory: left to its default it asks the proxy for `/websockify`,
        // which names no computer, and the page draws "Failed to connect to
        // server" over a desktop that is up. Seen live on the Debian package's
        // noVNC; explicit is right for every version.
        up.then(|| {
            format!(
                "http://{VIEWER_HOST}:{port}/{id}/{vnc}/vnc.html\
                 ?autoconnect=1&resize=scale&reconnect=1&path={id}/{vnc}/websockify",
                port = self.viewer_port,
                id = self.handle.computer,
                vnc = desktop::VNC_PORT
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
    /// Carries its own sentence because the way out differs: a key to add, a
    /// package to install, or two providers that each said why not.
    #[error("{0}")]
    Unconfigured(String),
    #[error(transparent)]
    Provider(#[from] ProviderError),
    #[error(transparent)]
    Store(#[from] crate::db::StoreError),
    #[error("the computer could not be recorded and was released ({0}); try again")]
    Recording(String),
}

/// Said to whoever asked for E2B by name and has no key. The other way out is
/// named too: an operator who chose E2B on a Mac that could run a computer
/// locally would otherwise be sent to sign up for an account they do not need.
const NO_E2B_KEY: &str = "no E2B API key is set; add one in Settings, or choose Automatic or \
                          Apple Container as the computer provider";

/// The same fact as a row in Settings' list of providers, where "choose
/// something else" is the list itself and saying it again is noise.
const NO_E2B_KEY_STATUS: &str =
    "No E2B API key is set. Add one in Settings to give agents a sandbox in E2B's cloud.";

/// Every provider this build knows, in the order `automatic` tries them: a
/// machine on this Mac before one that is rented, because it costs nothing and
/// keeps the operator's work on their own disk.
const PROVIDERS: [Provider; 2] = [Provider::AppleContainer, Provider::E2b];

/// How long an answer about a provider is worth keeping. Long enough that
/// Settings, the prompt and a create moments apart ask once; short enough that
/// an operator who starts the runtime in Terminal sees it within one look.
const PROBE_TTL: std::time::Duration = std::time::Duration::from_secs(30);

/// The slowest the idle ticker ever runs: often enough that a minute of idle
/// is a minute, cheap enough that it is one command per running machine.
const IDLE_TICK_MAX: std::time::Duration = std::time::Duration::from_secs(60);

/// The file the guest's PID 1 watches. It exits when this goes stale, so a
/// machine outlives a force-quit of this app by one idle period and no more.
const HEARTBEAT: &str = "/run/guaca/heartbeat";

/// A touch on a running machine. Longer than the command needs and shorter
/// than the tick, so a wedged machine costs one tick rather than the ticker.
const HEARTBEAT_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(10);

/// How often a running local machine's heartbeat is touched: twice per idle
/// period, and never slower than a minute.
///
/// The guest's PID 1 exits when `/run/guaca/heartbeat` is older than
/// `GUAC_IDLE_SECONDS`, which is this same setting, and this ticker is the only
/// thing that writes that file. At the shortest setting the app allows — one
/// minute — a fixed minute tick has no margin at all: the guest stops itself
/// about as often as it survives, on a machine an agent may be working on.
fn idle_tick_period(idle_seconds: u32) -> std::time::Duration {
    // Never zero. A tick of no length is a loop with nothing to wait on, and a
    // setting that arrives here as zero should cost a wasted tick, not a core.
    std::time::Duration::from_secs((u64::from(idle_seconds) / 2).clamp(1, IDLE_TICK_MAX.as_secs()))
}

/// Whether a machine of this kind runs on this Mac.
///
/// Exhaustive on purpose, and asked rather than inferred from "not E2B": what
/// hangs on it is who stops a machine nobody is using. A kind added to the enum
/// and defaulted to the hosted side is a VM that nothing ever stops, and that
/// failure is silent until somebody notices the fans.
fn is_local(which: Provider) -> bool {
    match which {
        Provider::E2b => false,
        Provider::AppleContainer => true,
    }
}

/// One touch of the heartbeat: no credentials, nothing to interpret, and a
/// deadline, because this runs on every running machine every minute.
fn heartbeat() -> ExecRequest {
    ExecRequest {
        argv: vec!["touch".to_string(), HEARTBEAT.to_string()],
        env: BTreeMap::new(),
        cwd: "/".to_string(),
        timeout: HEARTBEAT_TIMEOUT,
    }
}

/// Why this build cannot drive a kind at all, as the error whoever asked for it
/// by name reads.
fn unconfigured(which: Provider) -> ComputerError {
    ComputerError::Unconfigured(match which {
        Provider::E2b => NO_E2B_KEY.to_string(),
        Provider::AppleContainer => apple::not_installed().detail,
    })
}

/// The same, as the status Settings draws and `automatic` explains itself with.
fn unconfigured_status(which: Provider) -> ProviderStatus {
    match which {
        Provider::E2b => ProviderStatus {
            state: ProviderReadiness::NotInstalled,
            can_start: false,
            detail: NO_E2B_KEY_STATUS.to_string(),
        },
        Provider::AppleContainer => apple::not_installed(),
    }
}

/// Whether a provider could make a machine if asked now. A stopped service
/// counts: starting a computer is what starts it, which is the one state where
/// this app acts on the operator's behalf rather than reporting.
fn usable(status: &ProviderStatus) -> bool {
    match status.state {
        ProviderReadiness::Ready => true,
        ProviderReadiness::NotRunning => status.can_start,
        _ => false,
    }
}

/// Whether "or choose Apple Container" is a way out worth telling an operator
/// about: a runtime that could make a machine now, or one that is simply not
/// installed yet. Never on a Mac that cannot run what is installed, where
/// installing it is the one remedy that cannot work.
fn worth_choosing(status: &ProviderStatus) -> bool {
    usable(status) || status.state == ProviderReadiness::NotInstalled
}

/// Nothing here can make a machine, in the providers' own words.
///
/// "No computer provider is ready" on its own is a dead end: what the operator
/// does next depends on which provider they are closest to having, and only
/// that provider's own sentence says. One detail or two, because an operator
/// who named a provider and one who named none are owed the same shape of
/// answer — the named one just has a shorter list.
fn nothing_ready(details: &[String]) -> ComputerError {
    ComputerError::Unconfigured(format!("no computer provider is ready. {}", details.join(" ")))
}

struct Inner {
    store: Store,
    config: Arc<RwLock<AppConfig>>,
    /// One lock per agent, so an operator click and an agent tool call cannot
    /// make two machines. Held across the whole `ensure`, including the create.
    locks: Mutex<HashMap<AgentId, Arc<tokio::sync::Mutex<()>>>>,
    /// The providers built so far, each under the kind it runs and the setting
    /// it was built from. Kept because the viewer resolves a target per proxied
    /// request and one noVNC page is fifty of those plus a WebSocket: building
    /// a provider there was a connection pool made and thrown away per asset,
    /// and a provider that drives a CLI would be a process per asset.
    ///
    /// The second half of the key is what a settings change moves: the E2B API
    /// key, and the installation id every local resource is labelled with. A
    /// provider built from either is wrong the moment it changes, and an entry
    /// under the old one is never read again.
    providers: RwLock<HashMap<(Provider, String), Arc<dyn ComputerProvider>>>,
    /// The last answer each provider gave about itself, with when it gave it.
    /// A probe spawns processes, and Settings, the prompt and every automatic
    /// resolution ask the same question within seconds of each other.
    probes: Mutex<HashMap<Provider, (std::time::Instant, ProviderStatus)>>,
    /// Loopback port of the viewer proxy. Zero until it is listening.
    viewer_port: AtomicU16,
    /// The whole registry, in tests: what is in here is what this build can
    /// drive, and a kind that is absent is one nothing on this Mac provides.
    #[cfg(test)]
    injected: Option<HashMap<Provider, Arc<dyn ComputerProvider>>>,
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
                providers: RwLock::new(HashMap::new()),
                probes: Mutex::new(HashMap::new()),
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
        Self::with_providers(store, config, vec![provider])
    }

    /// A manager whose registry holds exactly these providers, each under the
    /// kind it says it is, and nothing this Mac happens to have installed.
    ///
    /// An empty list is a real state and the reason this takes one: it is an
    /// install with no key and no local runtime, which is what a fresh one is.
    #[cfg(test)]
    pub(crate) fn with_providers(
        store: Store,
        config: Arc<RwLock<AppConfig>>,
        providers: Vec<Arc<dyn ComputerProvider>>,
    ) -> Self {
        let injected = providers.into_iter().map(|p| (p.kind(), p)).collect();
        Self {
            inner: Arc::new(Inner {
                store,
                config,
                locks: Mutex::new(HashMap::new()),
                providers: RwLock::new(HashMap::new()),
                probes: Mutex::new(HashMap::new()),
                viewer_port: AtomicU16::new(0),
                injected: Some(injected),
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

    /// What every provider this build knows would say about itself, in the
    /// order `automatic` tries them. What Settings draws, one row each.
    pub async fn statuses(&self) -> Vec<(Provider, ProviderStatus)> {
        let mut answers = Vec::with_capacity(PROVIDERS.len());
        for which in PROVIDERS {
            answers.push((which, self.status(which).await));
        }
        answers
    }

    /// One provider's answer, from the last one it gave while that is still
    /// worth believing.
    async fn status(&self, which: Provider) -> ProviderStatus {
        // Read out and released before the probe: probing a local runtime is
        // three processes, and a lock held across that is every other caller
        // queueing behind this one's CLI.
        let remembered = self
            .inner
            .probes
            .lock()
            .get(&which)
            .filter(|(asked, _)| asked.elapsed() < PROBE_TTL)
            .map(|(_, status)| status.clone());
        if let Some(status) = remembered {
            return status;
        }

        let status = match self.provider(which) {
            Ok(provider) => provider.probe().await,
            // Nothing to ask, and the reason is the same sentence a probe
            // would have given: there is no key, or there is no binary.
            Err(_) => unconfigured_status(which),
        };
        self.inner.probes.lock().insert(which, (std::time::Instant::now(), status.clone()));
        status
    }

    /// Forgets every provider built and everything they said about themselves.
    ///
    /// Called when settings change: the key a provider was built from, the
    /// installation its resources are labelled with, and which provider is even
    /// wanted can all move in one save, and an answer from before that is not
    /// an answer about now.
    pub fn invalidate(&self) {
        self.inner.providers.write().clear();
        self.forget_probes();
    }

    /// The answers only, for when this app changed the thing being asked about.
    fn forget_probes(&self) {
        self.inner.probes.lock().clear();
    }

    /// The provider, asked to make itself usable before a machine is made or
    /// woken on it: a local runtime may have a service that is installed and
    /// stopped, and a hosted one has nothing to prepare.
    ///
    /// The two halves travel together on purpose. Whatever `prepare` started,
    /// every answer given about that provider before it is now wrong, and a
    /// cache that says "installed but stopped" about a service this app has
    /// just started is one Settings draws for the next half minute.
    async fn prepared(&self, provider: &Arc<dyn ComputerProvider>) -> Result<(), ComputerError> {
        provider.prepare().await?;
        self.forget_probes();
        Ok(())
    }

    /// Whether this agent can be given a computer at all, which is what its
    /// prompt and its tool list are built from.
    ///
    /// An agent that owns one keeps it whatever the default provider is now: a
    /// setting that moved does not take away a disk, and telling a model
    /// mid-crew that its machine is gone is how a working computer disappears
    /// from under a turn.
    ///
    /// The refusal carries its own words because there is more than one way to
    /// have no computer, and the prompt built from this tells an operator what
    /// to do about theirs.
    pub async fn availability(&self, card: &AgentCard) -> ComputerAccess {
        if card.computer_id.is_some() || self.default_provider().await.is_ok() {
            return ComputerAccess::Available;
        }
        self.refusal().await
    }

    /// Why nothing here can make a machine, in the two clauses an agent repeats
    /// to its operator.
    ///
    /// Derived from what the providers actually say rather than written once:
    /// "installing Apple Container would give you one" is false on a Mac that
    /// cannot run it and beside the point when the operator has named E2B, and
    /// an agent that reports the wrong remedy sends them somewhere that does
    /// not help. What it must not repeat is the *detail* Settings draws: a
    /// terminal command and an install path are for the person at the window,
    /// not for a model quoting it into a chat.
    async fn refusal(&self) -> ComputerAccess {
        let choice = self.inner.config.read().computer.provider;
        // One read, and a cache hit on every path that gets here: whatever just
        // refused to make a machine asked this same question moments ago. Every
        // branch needs it, because the local runtime is the half that varies —
        // the hosted one answers `ready` on nothing but a key being present.
        let apple = self.status(Provider::AppleContainer).await;
        match choice {
            // Reached only when `E2bProvider::new` refused the key, and it
            // refuses exactly one thing: an empty one.
            ProviderChoice::Provider(Provider::E2b) => ComputerAccess::unavailable(
                "E2B is the chosen provider and it has no API key",
                if worth_choosing(&apple) {
                    "adding one in Settings, or choosing Apple Container, would give you one"
                } else {
                    "adding one in Settings would give you one"
                },
            ),
            ProviderChoice::Provider(Provider::AppleContainer) => match apple.state {
                // `discover` found no binary, which is also the one thing a
                // probe cannot be asked about afterwards.
                ProviderReadiness::NotInstalled => ComputerAccess::unavailable(
                    "Apple Container is the chosen provider and it is not installed",
                    "installing it, or choosing E2B and adding a key in Settings, would give you \
                     one",
                ),
                // Installed and out of the range this build drives, or on a
                // machine that will never run it. Telling this operator to
                // install it is the one answer that cannot work.
                ProviderReadiness::Unsupported => ComputerAccess::unavailable(
                    "Apple Container is the chosen provider and this Mac cannot run the installed \
                     version",
                    "choosing E2B and adding a key in Settings would give you one",
                ),
                // Installed, and something between it and a machine that this
                // app cannot clear on its own: a service that would not answer,
                // or one it has no way to start. What that is belongs in
                // Settings, where the operator can act on it — not in a
                // sentence a model reads out, which is why this points at the
                // status rather than quoting it.
                _ => ComputerAccess::unavailable(
                    "Apple Container is the chosen provider and it is not ready to make one",
                    "its status in Settings says what it needs; choosing E2B and adding a key \
                     there would also give you one",
                ),
            },
            // Nothing was usable, which for the hosted half means there is no
            // key: a provider built from one answers `ready` without being
            // asked anything else. So the local half is what varies.
            ProviderChoice::Automatic => match apple.state {
                // Said of the runtime rather than of the Mac, because it is
                // both cases: a machine that will never run Apple Container,
                // and one running a version this build refuses.
                ProviderReadiness::Unsupported => ComputerAccess::unavailable(
                    "Apple Container is unsupported here and no E2B key is set",
                    "adding an E2B key in Settings would give you one",
                ),
                ProviderReadiness::Error => ComputerAccess::unavailable(
                    "Apple Container is installed and not answering, and no E2B key is set",
                    "getting Apple Container answering again, or adding an E2B key in Settings, \
                     would give you one",
                ),
                _ => ComputerAccess::unavailable(
                    "no computer provider is set up on this Mac",
                    "installing Apple Container or adding an E2B key in Settings would give you \
                     one",
                ),
            },
        }
    }

    /// Keeps local machines alive while they are being used, and stops them
    /// when they are not.
    ///
    /// Hosted machines are not touched: E2B stops its own sandboxes on its own
    /// timeout, and two authorities over one machine is one too many.
    pub fn start_idle_ticker(&self, handle: tokio::runtime::Handle) {
        let manager = self.clone();
        handle.spawn(async move {
            loop {
                // Read every time round rather than once: an operator who
                // shortens the idle setting should not have to wait out the old
                // period, or restart the app, for the change to take.
                tokio::time::sleep(idle_tick_period(manager.idle_seconds())).await;
                manager.idle_tick(now_ms()).await;
            }
        });
    }

    /// One tick, taking the time it is rather than reading the clock, so a test
    /// can stand at any point in an idle period without waiting to get there.
    pub(crate) async fn idle_tick(&self, now: i64) {
        let listed = match self.inner.store.list_computers() {
            Ok(listed) => listed,
            Err(err) => {
                tracing::warn!(%err, "could not read the computers to keep them awake");
                return;
            }
        };
        let idle_ms = i64::from(self.idle_seconds()) * 1000;

        // Side by side, for the same reason the shutdown is: each one is a
        // command to a runtime, and one machine that is slow to answer must not
        // hold back the heartbeat of every machine behind it — the guest's own
        // watchdog stops a machine whose heartbeat goes stale, so a tick that
        // queues is a machine that stops while an agent is working on it. Each
        // waits on its own agent's lock, and an agent has at most one computer,
        // so no two of these want the same lock.
        let ticks = listed
            .into_iter()
            .filter(|record| is_local(record.provider))
            .map(|record| self.tick_one(record.agent_id, record.id, now, idle_ms));
        futures_util::future::join_all(ticks).await;
    }

    /// One machine: stopped if nobody has used it lately, touched if they have,
    /// under its agent's lock either way.
    async fn tick_one(&self, agent: AgentId, id: ComputerId, now: i64, idle_ms: i64) {
        let lock = self.lock_for(agent);
        let _held = lock.lock().await;
        let Some((record, provider, handle)) = self.running_local(id).await else {
            return;
        };

        if now - record.last_used_at > idle_ms {
            match provider.stop(&handle).await {
                Ok(()) => tracing::info!(
                    computer = %record.id,
                    "stopped a computer nobody has used lately; its disk is kept"
                ),
                Err(err) => {
                    tracing::warn!(%err, computer = %record.id, "could not stop an idle computer")
                }
            }
            return;
        }

        // The other half of the watchdog. The guest's PID 1 exits when this
        // file goes stale, which is what stops a machine when this app is
        // force-quit and there is nothing left to stop it.
        if let Err(err) = provider.exec(&handle, heartbeat()).await {
            tracing::debug!(
                %err,
                computer = %record.id,
                "could not touch a computer's heartbeat"
            );
        }
    }

    /// Stops every local machine, for a shutdown that is not a crash.
    ///
    /// Without it a VM holding four gigabytes outlives the app by a whole idle
    /// period, on a Mac whose owner has already closed the window. Hosted
    /// machines are left running: their timeout is the provider's, and stopping
    /// one early is a decision about somebody's bill that this is not the place
    /// to make.
    pub async fn stop_local_machines(&self) {
        let listed = match self.inner.store.list_computers() {
            Ok(listed) => listed,
            Err(err) => {
                tracing::warn!(%err, "could not read the computers to stop them");
                return;
            }
        };

        // Side by side, because the whole shutdown is on one deadline and each
        // stop is a command to a runtime. One after another, a crew of four
        // spends four deadlines out of the one budget and the last machine is
        // never asked at all. Each waits on its own agent's lock, and an agent
        // has at most one computer, so no two of these want the same lock.
        let stops = listed
            .into_iter()
            .filter(|record| is_local(record.provider))
            .map(|record| self.stop_on_the_way_out(record.agent_id, record.id));
        futures_util::future::join_all(stops).await;
    }

    /// One machine, stopped under its agent's lock, with nothing to report
    /// upwards: a shutdown carries on whatever any one machine says.
    async fn stop_on_the_way_out(&self, agent: AgentId, id: ComputerId) {
        let lock = self.lock_for(agent);
        let _held = lock.lock().await;
        let Some((record, provider, handle)) = self.running_local(id).await else {
            return;
        };
        match provider.stop(&handle).await {
            Ok(()) => tracing::info!(computer = %record.id, "stopped a computer on the way out"),
            Err(err) => {
                tracing::warn!(%err, computer = %record.id, "could not stop a computer on the way out")
            }
        }
    }

    /// This computer, its provider and its handle, if it is a machine that can
    /// be acted on right now.
    ///
    /// Re-read rather than taken from the list the caller walked: `ensure`
    /// writes `last_used_at` under this same lock, and a machine a turn started
    /// using a millisecond ago must not be stopped for being idle.
    async fn running_local(
        &self,
        id: ComputerId,
    ) -> Option<(ComputerRecord, Arc<dyn ComputerProvider>, ProviderHandle)> {
        let record = self.inner.store.computer(id).ok()??;
        if record.state != RecordState::Ready {
            return None;
        }
        let handle = Self::handle_of(&record)?;
        let provider = self.provider(record.provider).ok()?;
        match provider.inspect(&handle).await {
            Ok(ProviderState::Running) => Some((record, provider, handle)),
            // Asleep, gone, or a runtime that would not say: none of the three
            // is something to keep awake or stop, and none is worth a failure.
            _ => None,
        }
    }

    /// What every resource this install makes is labelled with, so another copy
    /// of Guac on the same Mac is never swept up as this one's orphan.
    pub fn installation_id(&self) -> String {
        self.inner.config.read().computer.installation_id.clone()
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
                            // A local runtime's service can have stopped since
                            // this disk was last used — a restart of the Mac is
                            // the ordinary way — and asking it to wake anything
                            // then fails. Somebody asking for their computer
                            // back is the same permission to start it as
                            // somebody asking for a new one.
                            self.prepared(&provider).await?;
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

        let provider = self.default_provider().await?;
        // Before the row: a failure here has made nothing and should leave
        // nothing behind.
        self.prepared(&provider).await?;

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
            Err(ComputerError::Unconfigured(_)) => return Ok(None),
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

        // Only a provider this build can drive can be asked, and each is asked
        // about its own kind alone: a container's name on this Mac and a
        // sandbox id in a cloud share a namespace with nothing, so a row
        // claiming one says nothing about the other.
        //
        // What a local runtime lists is matched against `provider_id`, which is
        // the name `create` chose. A live Apple Container 1.2.2 was measured
        // before this half was allowed to delete anything: `container ls --all
        // --format json` reports that name at `configuration.id`. Had it
        // reported a digest or a generated id instead, every live machine would
        // have looked unclaimed and the first sweep after a restart would have
        // deleted all of them.
        for which in PROVIDERS {
            let Ok(provider) = self.provider(which) else {
                continue;
            };
            let owned = match provider.list_owned().await {
                Ok(owned) => owned,
                // One runtime that will not answer is not a reason to leave
                // another one's machines running and billing.
                Err(err) => {
                    tracing::warn!(
                        %err,
                        provider = which.as_str(),
                        "could not ask a provider what it is running"
                    );
                    continue;
                }
            };
            // Read after the list, never before: a machine made while the list
            // was in flight is absent from it, but its row is here, so nothing
            // is deleted for being younger than the question.
            let (claimed, provisioning) = self.claims(which)?;
            if provisioning {
                // A create of this kind is in flight, and the machine it is
                // making exists under a name its row does not carry yet: the
                // provider id is written when the create returns, and a local
                // runtime makes the container first and boots its VM after.
                // Nothing here can tell that from a leak, so the whole
                // unclaimed half waits for the next sweep. Startup runs one,
                // and a machine that really is orphaned costs one more idle
                // period rather than being guessed at.
                tracing::info!(
                    provider = which.as_str(),
                    "a computer of this kind is still being made; leaving unclaimed ones for now"
                );
                continue;
            }

            for id in owned {
                if claimed.contains(&id) {
                    continue;
                }
                // Asked again, immediately before the delete. The list above is
                // seconds old by the time it arrives — long enough on a Mac for
                // a whole create — and this delete is the one irreversible
                // thing the sweep does.
                let (claimed_now, provisioning_now) = self.claims(which)?;
                if provisioning_now {
                    tracing::info!(
                        provider = which.as_str(),
                        "a computer of this kind is being made; leaving unclaimed ones for now"
                    );
                    break;
                }
                if claimed_now.contains(&id) {
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

    /// What the rows of one kind say right now: every provider id one of them
    /// names, and whether any create of that kind is still in flight.
    ///
    /// Both from one read, because the unclaimed half of the sweep is deciding
    /// whether a resource is a leak and needs them to be the same instant. An
    /// id is a claim; a create in flight is a claim that has not been written
    /// down yet.
    fn claims(&self, which: Provider) -> Result<(HashSet<String>, bool), ComputerError> {
        let mut claimed = HashSet::new();
        let mut provisioning = false;
        for record in self.inner.store.list_computers()? {
            if record.provider != which {
                continue;
            }
            provisioning |= record.state == RecordState::Provisioning;
            if let Some(id) = record.provider_id {
                claimed.insert(id);
            }
        }
        Ok((claimed, provisioning))
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
            return injected.get(&which).cloned().ok_or_else(|| unconfigured(which));
        }

        // What the entry is keyed on as well as what builds it, so an operator
        // who changes either in Settings gets a provider that uses the new one
        // on the next call.
        let key = match which {
            Provider::E2b => self.inner.config.read().e2b.api_key.trim().to_string(),
            Provider::AppleContainer => self.installation_id(),
        };
        // Cloned out rather than answered from under the guard: this is on the
        // viewer's path, and a read lock held while a provider is built is one
        // fifty concurrent asset requests queue behind.
        let cached = self.inner.providers.read().get(&(which, key.clone())).cloned();
        if let Some(provider) = cached {
            return Ok(provider);
        }

        let built = match which {
            Provider::E2b => E2bProvider::new(&key)
                .map(|provider| Arc::new(provider) as Arc<dyn ComputerProvider>),
            // `None` is the one thing a probe cannot tell us afterwards: there
            // is no binary to ask. Everything else about a runtime that is
            // installed is `probe`'s answer, not this one's.
            Provider::AppleContainer => AppleContainer::discover(&key)
                .map(|provider| Arc::new(provider) as Arc<dyn ComputerProvider>),
        }
        .ok_or_else(|| unconfigured(which))?;

        self.inner.providers.write().insert((which, key), built.clone());
        Ok(built)
    }

    /// Who runs a machine this app is about to make.
    ///
    /// Resolved here and written to the row by `ensure`, once: a computer that
    /// changed hands because a setting moved is a disk its agent can no longer
    /// reach, and a second machine on a provider nobody asked for.
    ///
    /// A provider named in settings is used or refused on its own terms —
    /// falling through to another one would be this app quietly overruling the
    /// choice, and the refusal is what says how to make that choice work.
    async fn default_provider(&self) -> Result<Arc<dyn ComputerProvider>, ComputerError> {
        // Copied out of the guard rather than matched under it: the automatic
        // half awaits, and a lock held across an await is a future that cannot
        // cross threads.
        let choice = self.inner.config.read().computer.provider;
        if let ProviderChoice::Provider(which) = choice {
            // Built first, because a provider that cannot be built at all has a
            // sentence of its own: no key, or no binary to ask anything of.
            let provider = self.provider(which)?;
            // And then asked the same question the automatic half asks, because
            // being buildable is not being ready. For a local runtime it means
            // only "the binary is there", which is true of a Mac whose version
            // this build refuses and of one whose service will not answer. The
            // pane offers a computer on `ready || canStart`, and an agent told
            // it has one that nothing can make spends a turn finding out.
            let status = self.status(which).await;
            if !usable(&status) {
                return Err(nothing_ready(&[status.detail]));
            }
            return Ok(provider);
        }

        let mut refusals = Vec::new();
        for which in PROVIDERS {
            let status = self.status(which).await;
            if usable(&status) {
                if let Ok(provider) = self.provider(which) {
                    return Ok(provider);
                }
            }
            refusals.push(status.detail);
        }
        Err(nothing_ready(&refusals))
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
        pub prepares: Mutex<u32>,
        pub probes: Mutex<u32>,
        pub fail_create: Mutex<bool>,
        pub fail_delete: Mutex<bool>,
        pub fail_list: Mutex<bool>,
        pub fail_prepare: Mutex<bool>,
        pub create_delay: Mutex<Option<std::time::Duration>>,
        pub list_delay: Mutex<Option<std::time::Duration>>,
        pub stop_delay: Mutex<Option<std::time::Duration>>,
        /// How many stops were ever in flight at the same moment, which is the
        /// deterministic half of "these ran side by side".
        pub stops_at_once: Mutex<u32>,
        stopping: Mutex<u32>,
        /// What every exec answers with, in order; the last one repeats.
        pub replies: Mutex<Vec<Output>>,
        /// What a command containing this needle answers with, in order, with
        /// the last one repeating. Read before `replies`, so a test that cares
        /// about one command in a long sequence does not have to count the ones
        /// it does not care about — and does not break when a step is added to
        /// a sequence it never mentioned.
        pub matched: Mutex<Vec<(String, Vec<Output>)>>,
        /// What `probe` answers. `None` is ready: a fake that had to be told it
        /// works before it would work is one every existing test would have to
        /// set up.
        pub probe: Mutex<Option<ProviderStatus>>,
        /// Which kind this stands in for. `None` is E2B for the same reason.
        pub kind: Mutex<Option<Provider>>,
    }

    impl FakeProvider {
        /// A fake of the local kind: what the manager keeps awake with a
        /// heartbeat, stops at shutdown, and sweeps behind the gate.
        pub fn local() -> Self {
            Self { kind: Mutex::new(Some(Provider::AppleContainer)), ..Self::default() }
        }

        /// The next answer scripted for a command like this one, if any.
        fn scripted(&self, command: &str) -> Option<Output> {
            let mut matched = self.matched.lock();
            let (_, answers) =
                matched.iter_mut().find(|(needle, _)| command.contains(needle.as_str()))?;
            // The last one stays: "down, down, up" means up from then on, which
            // is how a port that opens behaves.
            Some(if answers.len() > 1 { answers.remove(0) } else { answers.first()?.clone() })
        }
    }

    #[async_trait::async_trait]
    impl ComputerProvider for FakeProvider {
        fn kind(&self) -> Provider {
            self.kind.lock().unwrap_or(Provider::E2b)
        }

        async fn probe(&self) -> ProviderStatus {
            *self.probes.lock() += 1;
            self.probe.lock().clone().unwrap_or_else(|| ProviderStatus::ready("fake: ready"))
        }

        async fn prepare(&self) -> Result<(), ProviderError> {
            if *self.fail_prepare.lock() {
                return Err(ProviderError::Unavailable("fake: the service would not start".into()));
            }
            *self.prepares.lock() += 1;
            Ok(())
        }

        async fn create(&self, request: &CreateComputer) -> Result<ProviderHandle, ProviderError> {
            if *self.fail_create.lock() {
                return Err(ProviderError::Unavailable("fake: create refused".into()));
            }
            *self.creates.lock() += 1;
            let id = format!("fake-{}", request.computer.short());
            // Registered before the wait, not after, because that is what a
            // real runtime does: `container create` makes the resource and
            // starting its VM is what takes the seconds. For all of them a
            // machine exists that no row names yet, and anything that decides
            // what is unclaimed has to survive that gap.
            self.machines.lock().insert(id.clone(), ProviderState::Running);
            // Read out and released before the sleep: a guard held across an
            // await is a future that cannot cross threads.
            let delay = *self.create_delay.lock();
            if let Some(delay) = delay {
                tokio::time::sleep(delay).await;
            }
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
            // Counted before the wait and released after it, so what a test
            // reads is how many were genuinely overlapping rather than how
            // many happened.
            let delay = {
                let mut stopping = self.stopping.lock();
                *stopping += 1;
                let mut most = self.stops_at_once.lock();
                *most = (*most).max(*stopping);
                *self.stop_delay.lock()
            };
            if let Some(delay) = delay {
                tokio::time::sleep(delay).await;
            }
            *self.stopping.lock() -= 1;
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
            let command = request.argv.join(" ");
            self.execs.lock().push(request);
            if let Some(scripted) = self.scripted(&command) {
                return Ok(scripted);
            }
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
            if *self.fail_list.lock() {
                return Err(ProviderError::Unavailable("fake: cannot list".into()));
            }
            // Read after the wait, so what comes back is what the runtime held
            // when it answered rather than when it was asked. A real list of a
            // Mac's containers takes long enough for a machine to be made
            // inside it.
            let delay = *self.list_delay.lock();
            if let Some(delay) = delay {
                tokio::time::sleep(delay).await;
            }
            let mut owned: Vec<String> = self.machines.lock().keys().cloned().collect();
            // Sorted so a sweep's log and a test's assertion read the same way
            // twice running; a HashMap's order is not an answer.
            owned.sort();
            Ok(owned)
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
    use crate::domain::computer::{ComputerRecord, Provider, ProviderChoice, RecordState, Secret};

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

    /// A manager whose whole world is the fakes given: nothing here asks this
    /// Mac what it has installed, which is the difference between a test that
    /// passes everywhere and one that passes on the machine that wrote it.
    #[allow(clippy::type_complexity)]
    fn setup_with(
        providers: Vec<Arc<fake::FakeProvider>>,
    ) -> (ComputerManager, Store, AgentCard, Arc<RwLock<AppConfig>>, tempfile::TempDir) {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::open(&dir.path().join("guac.db")).unwrap();
        let card = store.create_agent(&draft("Manager")).unwrap();
        let config = Arc::new(parking_lot::RwLock::new(AppConfig::default()));
        let registry =
            providers.into_iter().map(|p| p as Arc<dyn ComputerProvider>).collect::<Vec<_>>();
        let manager = ComputerManager::with_providers(store.clone(), config.clone(), registry);
        (manager, store, card, config, dir)
    }

    fn says(state: ProviderReadiness, can_start: bool, detail: &str) -> ProviderStatus {
        ProviderStatus { state, can_start, detail: detail.to_string() }
    }

    fn provider_of(store: &Store, agent: AgentId) -> Provider {
        store.computer_for_agent(agent).unwrap().expect("a computer").provider
    }

    #[tokio::test]
    async fn automatic_takes_the_local_runtime_first_and_skips_one_that_is_not_installed() {
        // The order is the point: a machine on this Mac costs nothing and keeps
        // the operator's work on their own disk, so a rented one is what is
        // left when there is no local runtime to be had.
        let local = Arc::new(fake::FakeProvider::local());
        *local.probe.lock() = Some(says(ProviderReadiness::NotInstalled, false, "no binary"));
        let hosted = Arc::new(fake::FakeProvider::default());
        let (manager, store, card, _config, _dir) = setup_with(vec![local.clone(), hosted.clone()]);

        manager.ensure(&card, BTreeMap::new()).await.unwrap();

        assert_eq!((*hosted.creates.lock(), *local.creates.lock()), (1, 0));
        assert_eq!(provider_of(&store, card.id), Provider::E2b, "written to the row");
    }

    #[tokio::test]
    async fn a_local_runtime_that_is_only_stopped_is_started_and_chosen_first() {
        // "Installed and stopped" is the one state this app acts on rather than
        // reports: starting a computer is what starts the service, and the
        // operator asked for a computer.
        let local = Arc::new(fake::FakeProvider::local());
        *local.probe.lock() = Some(says(ProviderReadiness::NotRunning, true, "stopped"));
        let hosted = Arc::new(fake::FakeProvider::default());
        let (manager, store, card, _config, _dir) = setup_with(vec![local.clone(), hosted.clone()]);

        manager.ensure(&card, BTreeMap::new()).await.unwrap();

        assert_eq!((*local.creates.lock(), *hosted.creates.lock()), (1, 0));
        assert_eq!(*local.prepares.lock(), 1, "the service was started before anything was made");
        assert_eq!(provider_of(&store, card.id), Provider::AppleContainer);
    }

    #[tokio::test]
    async fn a_service_that_will_not_start_leaves_no_machine_and_no_row() {
        let local = Arc::new(fake::FakeProvider::local());
        *local.fail_prepare.lock() = true;
        let (manager, store, card, _config, _dir) = setup_with(vec![local.clone()]);

        let Err(err) = manager.ensure(&card, BTreeMap::new()).await else {
            panic!("nothing can be made on a runtime that would not start");
        };

        assert!(err.to_string().contains("would not start"), "{err}");
        assert_eq!(*local.creates.lock(), 0);
        assert!(store.list_computers().unwrap().is_empty(), "a claim on nothing is a leak");
    }

    #[tokio::test]
    async fn with_nothing_ready_the_refusal_repeats_what_each_provider_said() {
        // One sentence per provider, in their own words, because "no computer
        // provider is ready" on its own is a dead end: what the operator does
        // next depends on which of the two they are closer to having.
        let local = Arc::new(fake::FakeProvider::local());
        *local.probe.lock() =
            Some(says(ProviderReadiness::NotRunning, false, "the runtime is wedged."));
        let hosted = Arc::new(fake::FakeProvider::default());
        *hosted.probe.lock() = Some(says(ProviderReadiness::NotInstalled, false, "no key here."));
        let (manager, _store, card, _config, _dir) = setup_with(vec![local, hosted]);

        let Err(err) = manager.ensure(&card, BTreeMap::new()).await else {
            panic!("neither provider could make anything");
        };

        let said = err.to_string();
        assert!(matches!(err, ComputerError::Unconfigured(_)), "{err}");
        assert!(said.contains("no computer provider is ready"), "{said}");
        assert!(said.contains("the runtime is wedged."), "{said}");
        assert!(said.contains("no key here."), "{said}");
    }

    #[tokio::test]
    async fn asking_for_e2b_by_name_without_a_key_says_both_ways_out() {
        let (manager, _store, card, config, _dir) = setup_with(vec![]);
        config.write().computer.provider = ProviderChoice::Provider(Provider::E2b);

        let Err(err) = manager.ensure(&card, BTreeMap::new()).await else {
            panic!("there is no key to make a sandbox with");
        };

        let said = err.to_string();
        assert!(matches!(err, ComputerError::Unconfigured(_)), "{err}");
        assert!(said.contains("E2B API key"), "what is missing: {said}");
        assert!(said.contains("Settings"), "where to put it: {said}");
        assert!(said.contains("Apple Container"), "and that a key is not the only way: {said}");
    }

    #[tokio::test]
    async fn asking_for_a_local_runtime_that_is_not_installed_says_where_to_get_it() {
        let (manager, _store, card, config, _dir) = setup_with(vec![]);
        config.write().computer.provider = ProviderChoice::Provider(Provider::AppleContainer);

        let Err(err) = manager.ensure(&card, BTreeMap::new()).await else {
            panic!("there is no runtime on this machine");
        };

        assert!(matches!(err, ComputerError::Unconfigured(_)), "{err}");
        assert!(
            err.to_string().contains("github.com/apple/container/releases"),
            "the refusal is the provider's own words: {err}"
        );
    }

    #[tokio::test]
    async fn a_provider_named_in_settings_still_has_to_be_able_to_make_one() {
        // Being buildable is not being ready. For the local runtime, building
        // one means only that the binary is on this Mac, which is as true of a
        // version this build refuses as of one it drives — and the pane offers
        // a computer on whether the provider could make one, so an agent was
        // told it had a machine that nothing here could make.
        let local = Arc::new(fake::FakeProvider::local());
        *local.probe.lock() =
            Some(says(ProviderReadiness::Unsupported, false, "fake: not this version."));
        let (manager, store, card, config, _dir) = setup_with(vec![local.clone()]);
        config.write().computer.provider = ProviderChoice::Provider(Provider::AppleContainer);

        let Err(err) = manager.ensure(&card, BTreeMap::new()).await else {
            panic!("a runtime this build cannot drive makes nothing");
        };
        assert!(matches!(err, ComputerError::Unconfigured(_)), "{err}");
        assert!(err.to_string().contains("fake: not this version."), "its own words: {err}");
        assert_eq!(*local.creates.lock(), 0);
        assert!(store.list_computers().unwrap().is_empty(), "a claim on nothing is a leak");

        // And the agent hears the same answer the runtime gave, rather than
        // being handed four tools with nothing behind them.
        let access = manager.availability(&card).await;
        assert!(!access.is_available());
        let offered = crate::llm::tools::offered(access.is_available());
        for needs_a_machine in [
            crate::llm::tools::RUN_COMMAND,
            crate::llm::tools::OPEN_ON_DESKTOP,
            crate::llm::tools::USE_SCREEN,
            crate::llm::tools::BROWSE,
        ] {
            assert!(
                !offered.iter().any(|spec| spec.name == needs_a_machine),
                "{needs_a_machine} was offered with no machine to run it on"
            );
        }
    }

    #[tokio::test]
    async fn a_named_runtime_that_is_only_stopped_is_still_a_provider() {
        // The other side of the gate, and the state Apple Container is actually
        // in most mornings: installed, its service down, and one click from a
        // machine. Refusing this one would be the fix overshooting.
        let local = Arc::new(fake::FakeProvider::local());
        *local.probe.lock() = Some(says(ProviderReadiness::NotRunning, true, "stopped"));
        let (manager, store, card, config, _dir) = setup_with(vec![local.clone()]);
        config.write().computer.provider = ProviderChoice::Provider(Provider::AppleContainer);

        assert!(manager.availability(&card).await.is_available());
        manager.ensure(&card, BTreeMap::new()).await.unwrap();

        assert_eq!(*local.prepares.lock(), 1, "the service was started before anything was made");
        assert_eq!(provider_of(&store, card.id), Provider::AppleContainer);
    }

    #[tokio::test]
    async fn a_computer_keeps_the_provider_that_made_it_when_the_setting_changes() {
        let local = Arc::new(fake::FakeProvider::local());
        let hosted = Arc::new(fake::FakeProvider::default());
        let (manager, store, card, config, _dir) = setup_with(vec![local.clone(), hosted.clone()]);
        manager.ensure(&card, BTreeMap::new()).await.unwrap();
        assert_eq!(provider_of(&store, card.id), Provider::AppleContainer);

        // The operator changes their mind in Settings, which is exactly what
        // must not reach into a disk an agent is already using.
        config.write().computer.provider = ProviderChoice::Provider(Provider::E2b);
        manager.invalidate();

        let (_, again) = manager.ensure(&card, BTreeMap::new()).await.unwrap();
        assert_eq!(again, Provisioned::Reused);
        assert_eq!(provider_of(&store, card.id), Provider::AppleContainer);
        assert_eq!(*hosted.creates.lock(), 0, "the new choice is for the next computer");
    }

    #[tokio::test]
    async fn availability_is_the_agents_own_computer_or_a_provider_that_could_make_one() {
        // The prompt and the tool list are built from this. An agent that owns
        // a machine must keep being told so when the default provider is
        // unusable, or a working computer disappears from under a turn.
        let (manager, store, card, _config, _dir) = setup_with(vec![]);
        assert!(
            !manager.availability(&card).await.is_available(),
            "nothing configured, nothing owned"
        );

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
        let owner = store.list_agents().unwrap().into_iter().find(|a| a.id == card.id).unwrap();

        assert!(owner.computer_id.is_some());
        assert!(manager.availability(&owner).await.is_available());
    }

    #[tokio::test]
    async fn a_refusal_names_the_configuration_the_operator_actually_has() {
        // The prompt built from this is read out to the operator, so one
        // sentence for every configuration is wrong in most of them: it told an
        // operator whose Mac cannot run the local runtime to install it, and an
        // operator who had named E2B that nothing on their Mac was ready.
        let reason = |access: ComputerAccess| match access {
            ComputerAccess::Unavailable { because, remedy } => (because, remedy),
            ComputerAccess::Available => panic!("nothing here can make a machine"),
        };

        // Nothing installed and no key, which is what a fresh install is.
        let (manager, _store, card, config, _dir) = setup_with(vec![]);
        let (because, remedy) = reason(manager.availability(&card).await);
        assert_eq!(because, "no computer provider is set up on this Mac");
        assert!(remedy.contains("installing Apple Container"), "{remedy}");
        assert!(remedy.contains("adding an E2B key in Settings"), "{remedy}");

        // A provider the operator named, and did not finish setting up. What
        // the other one could do is beside the point: they chose this one.
        config.write().computer.provider = ProviderChoice::Provider(Provider::E2b);
        manager.invalidate();
        let (because, remedy) = reason(manager.availability(&card).await);
        assert_eq!(because, "E2B is the chosen provider and it has no API key");
        assert!(
            remedy.contains("choosing Apple Container"),
            "the other way out is theirs too: {remedy}"
        );

        config.write().computer.provider = ProviderChoice::Provider(Provider::AppleContainer);
        manager.invalidate();
        let (because, remedy) = reason(manager.availability(&card).await);
        assert_eq!(because, "Apple Container is the chosen provider and it is not installed");
        assert!(remedy.contains("choosing E2B"), "{remedy}");

        // A local runtime that cannot be driven here: a Mac that will never run
        // it, or a version outside this build's range. Telling this operator to
        // install it is the one answer that cannot work, and "this Mac cannot
        // run it" is not quite true of the second case either.
        let unsupported = Arc::new(fake::FakeProvider::local());
        *unsupported.probe.lock() = Some(ProviderStatus {
            state: ProviderReadiness::Unsupported,
            can_start: false,
            detail: "fake: not on this Mac".into(),
        });
        let (manager, _store, card, config, _dir) = setup_with(vec![unsupported]);
        let (because, remedy) = reason(manager.availability(&card).await);
        assert_eq!(because, "Apple Container is unsupported here and no E2B key is set");
        assert!(!remedy.contains("installing Apple Container"), "it cannot be: {remedy}");
        assert_eq!(remedy, "adding an E2B key in Settings would give you one");

        // The same Mac with E2B named. The remedy used to offer the local
        // runtime as the other way out, on a machine where choosing it would
        // have changed nothing.
        config.write().computer.provider = ProviderChoice::Provider(Provider::E2b);
        let (because, remedy) = reason(manager.availability(&card).await);
        assert_eq!(because, "E2B is the chosen provider and it has no API key");
        assert_eq!(remedy, "adding one in Settings would give you one");

        // And with the local runtime named on that same Mac: it is installed,
        // so "install it" is wrong in the other direction.
        config.write().computer.provider = ProviderChoice::Provider(Provider::AppleContainer);
        let (because, remedy) = reason(manager.availability(&card).await);
        assert_eq!(
            because,
            "Apple Container is the chosen provider and this Mac cannot run the installed version"
        );
        assert_eq!(remedy, "choosing E2B and adding a key in Settings would give you one");
        assert!(!remedy.contains("installing"), "there is nothing left to install: {remedy}");

        // Installed, answering nothing this build can use, and not startable.
        // What it needs is a sentence for Settings, so the agent is pointed at
        // it rather than reciting a terminal command into a chat.
        let wedged = Arc::new(fake::FakeProvider::local());
        *wedged.probe.lock() = Some(says(
            ProviderReadiness::Error,
            false,
            "run `container system status` in Terminal.",
        ));
        let (manager, _store, card, config, _dir) = setup_with(vec![wedged]);
        config.write().computer.provider = ProviderChoice::Provider(Provider::AppleContainer);
        let (because, remedy) = reason(manager.availability(&card).await);
        assert_eq!(
            because,
            "Apple Container is the chosen provider and it is not ready to make one"
        );
        assert!(remedy.contains("its status in Settings"), "{remedy}");
        assert!(!remedy.contains("container system status"), "not for a model to quote: {remedy}");
    }

    #[tokio::test]
    async fn a_local_machine_idle_past_the_setting_is_stopped_and_a_busy_one_is_kept_alive() {
        let local = Arc::new(fake::FakeProvider::local());
        let (manager, store, card, config, _dir) = setup_with(vec![local.clone()]);
        let busy = store.create_agent(&draft("Chef")).unwrap();
        let (forgotten, _) = manager.ensure(&card, BTreeMap::new()).await.unwrap();
        let (working, _) = manager.ensure(&busy, BTreeMap::new()).await.unwrap();

        let idle_minutes = config.read().computer.idle_minutes as i64;
        store.touch_computer(forgotten.id(), now_ms() - (idle_minutes + 1) * 60_000).unwrap();

        manager.idle_tick(now_ms()).await;

        let machines = local.machines.lock();
        assert_eq!(machines[&format!("fake-{}", forgotten.id().short())], ProviderState::Asleep);
        assert_eq!(machines[&format!("fake-{}", working.id().short())], ProviderState::Running);
        drop(machines);

        // The other half of the watchdog: the guest's PID 1 exits when this
        // file goes stale, so a machine nobody stopped still stops when the app
        // that was touching it is gone.
        let execs = local.execs.lock();
        assert_eq!(execs.len(), 1, "only the machine still in use is touched");
        assert_eq!(execs[0].argv, vec!["touch", "/run/guaca/heartbeat"]);
        assert!(execs[0].env.is_empty(), "a heartbeat carries no credentials");
    }

    #[test]
    fn the_tick_beats_twice_per_idle_period_and_never_slower_than_a_minute() {
        // The guest's watchdog exits when the heartbeat is older than
        // `GUAC_IDLE_SECONDS`, which is this same setting. A fixed minute tick
        // against a one-minute setting is a race the guest wins about half the
        // time: the machine stops itself while an agent is working on it.
        assert_eq!(idle_tick_period(60), Duration::from_secs(30));
        assert_eq!(idle_tick_period(120), Duration::from_secs(60));
        assert_eq!(idle_tick_period(900), Duration::from_secs(60), "and no slower than that");
    }

    #[tokio::test]
    async fn waking_a_machine_starts_the_service_it_needs_and_reusing_one_does_not() {
        // A local runtime's service can be stopped between one use and the
        // next — a restart of this Mac is the ordinary way. Waking a disk on a
        // service that is not running fails, and the operator asking for their
        // computer back is the same permission to start it as asking for a new
        // one. Reusing a machine that is already running asks nothing: that
        // path runs on every command an agent types.
        let local = Arc::new(fake::FakeProvider::local());
        let (manager, _store, card, _config, _dir) = setup_with(vec![local.clone()]);
        manager.ensure(&card, BTreeMap::new()).await.unwrap();
        assert_eq!(*local.prepares.lock(), 1, "the create asked");

        manager.sleep(card.id).await.unwrap();
        let (_, woken) = manager.ensure(&card, BTreeMap::new()).await.unwrap();
        assert_eq!(woken, Provisioned::Woken);
        assert_eq!(*local.prepares.lock(), 2, "and so did the wake");

        let (_, reused) = manager.ensure(&card, BTreeMap::new()).await.unwrap();
        assert_eq!(reused, Provisioned::Reused);
        assert_eq!(*local.prepares.lock(), 2, "a machine already running asks nothing");
    }

    #[tokio::test]
    async fn machines_are_stopped_side_by_side_on_the_way_out() {
        // The whole shutdown is bounded, and a stop is a CLI call per machine.
        // One after another, a crew of four spends four deadlines of the one
        // budget and the last of them is never asked at all.
        let local = Arc::new(fake::FakeProvider::local());
        *local.stop_delay.lock() = Some(Duration::from_millis(50));
        let (manager, store, card, _config, _dir) = setup_with(vec![local.clone()]);
        manager.ensure(&card, BTreeMap::new()).await.unwrap();
        let second = store.create_agent(&draft("Chef")).unwrap();
        manager.ensure(&second, BTreeMap::new()).await.unwrap();

        manager.stop_local_machines().await;

        assert_eq!(
            local.machines.lock().values().filter(|s| **s == ProviderState::Asleep).count(),
            2
        );
        assert_eq!(*local.stops_at_once.lock(), 2, "both were in flight together");
    }

    #[tokio::test]
    async fn the_idle_tick_leaves_a_hosted_machine_to_its_own_timeout() {
        // E2B stops its own sandboxes and bills for what it stopped. Reaching
        // in to stop one from here is a second authority over the same machine.
        let hosted = Arc::new(fake::FakeProvider::default());
        let (manager, store, card, config, _dir) = setup_with(vec![hosted.clone()]);
        let (machine, _) = manager.ensure(&card, BTreeMap::new()).await.unwrap();
        let idle_minutes = config.read().computer.idle_minutes as i64;
        store.touch_computer(machine.id(), now_ms() - (idle_minutes + 1) * 60_000).unwrap();

        manager.idle_tick(now_ms()).await;

        assert_eq!(
            hosted.machines.lock()[&format!("fake-{}", machine.id().short())],
            ProviderState::Running
        );
        assert!(hosted.execs.lock().is_empty(), "and nothing was spent keeping it awake");
    }

    #[tokio::test]
    async fn shutting_down_stops_local_machines_and_leaves_hosted_ones_alone() {
        let local = Arc::new(fake::FakeProvider::local());
        let hosted = Arc::new(fake::FakeProvider::default());
        let (manager, store, card, config, _dir) = setup_with(vec![local.clone(), hosted.clone()]);
        let (mine, _) = manager.ensure(&card, BTreeMap::new()).await.unwrap();

        config.write().computer.provider = ProviderChoice::Provider(Provider::E2b);
        let renter = store.create_agent(&draft("Chef")).unwrap();
        let (rented, _) = manager.ensure(&renter, BTreeMap::new()).await.unwrap();

        manager.stop_local_machines().await;

        assert_eq!(
            local.machines.lock()[&format!("fake-{}", mine.id().short())],
            ProviderState::Asleep
        );
        assert_eq!(
            hosted.machines.lock()[&format!("fake-{}", rented.id().short())],
            ProviderState::Running,
            "a hosted machine's own timeout stays authoritative"
        );
    }

    #[tokio::test]
    async fn the_sweep_reaps_within_a_kind_and_now_reaps_the_local_one_too() {
        // Two claims that look alike across two providers are not the same
        // machine: a name on this Mac and a sandbox id in a cloud share a
        // namespace with nothing.
        let local = Arc::new(fake::FakeProvider::local());
        let hosted = Arc::new(fake::FakeProvider::default());
        let (manager, store, card, _config, _dir) = setup_with(vec![local.clone(), hosted.clone()]);
        local.machines.lock().insert("guac-orphan".into(), ProviderState::Running);
        local.machines.lock().insert("guac-kept".into(), ProviderState::Running);
        hosted.machines.lock().insert("shared-name".into(), ProviderState::Running);

        for (agent, provider, id) in [
            (card.id, Provider::AppleContainer, "guac-kept"),
            // A local row claiming the name a sandbox happens to have. It says
            // nothing about that sandbox, and the sweep must not read it as a
            // claim on one.
            (
                store.create_agent(&draft("Chef")).unwrap().id,
                Provider::AppleContainer,
                "shared-name",
            ),
        ] {
            store
                .insert_computer(&ComputerRecord {
                    id: ComputerId::new(),
                    agent_id: agent,
                    provider,
                    provider_id: Some(id.into()),
                    control_secret: Secret::default(),
                    viewer_secret: Secret::default(),
                    image_ref: String::new(),
                    state: RecordState::Ready,
                    last_used_at: 0,
                    created_at: 0,
                    updated_at: 0,
                })
                .unwrap();
        }

        let released = manager.sweep().await.unwrap();

        // Both halves delete now: a live 1.2.2 confirmed that what a local
        // runtime lists is the name the rows hold, which is what the local half
        // was waiting on before it was allowed to act on its own list.
        assert_eq!(released, 2);
        assert_eq!(*hosted.deletes.lock(), vec!["shared-name".to_string()]);
        assert_eq!(
            *local.deletes.lock(),
            vec!["guac-orphan".to_string()],
            "the unclaimed local machine is released, and the claimed one is not"
        );
    }

    #[tokio::test]
    async fn a_create_that_starts_after_the_sweep_did_survives_it() {
        // The other half of "the sweep leaves alone a machine being created
        // under it", and the half the per-agent lock cannot cover: this create
        // has no row at all when the sweep reads the rows, so there is no lock
        // it could have taken and nothing in the snapshot to re-read. Asking
        // the runtime what it owns takes seconds on a Mac — long enough for a
        // container to be made inside the question — and what comes back is a
        // name that no row carries yet, because the create that chose it is
        // still booting its VM. Deleting that is deleting a machine an agent
        // was just given.
        let local = Arc::new(fake::FakeProvider::local());
        *local.list_delay.lock() = Some(Duration::from_millis(100));
        *local.create_delay.lock() = Some(Duration::from_millis(300));
        let (manager, store, card, _config, _dir) = setup_with(vec![local.clone()]);

        let sweeping = {
            let manager = manager.clone();
            tokio::spawn(async move { manager.sweep().await })
        };
        // After the sweep has read the rows and while it is still asking the
        // runtime what it owns.
        tokio::time::sleep(Duration::from_millis(20)).await;
        let (machine, made) = manager.ensure(&card, BTreeMap::new()).await.unwrap();

        assert_eq!(made, Provisioned::Created);
        assert_eq!(sweeping.await.unwrap().unwrap(), 0, "there was nothing to release");
        assert!(local.deletes.lock().is_empty(), "the machine survived");
        assert!(store.computer(machine.id()).unwrap().is_some(), "and so did its row");
        assert_eq!(
            local.machines.lock().get(&format!("fake-{}", machine.id().short())),
            Some(&ProviderState::Running)
        );
    }

    #[tokio::test]
    async fn a_provider_that_will_not_list_does_not_stop_the_other_from_being_swept() {
        // A wedged local runtime and a leaking cloud account are two problems,
        // and only one of them is charging by the hour.
        let local = Arc::new(fake::FakeProvider::local());
        *local.fail_list.lock() = true;
        let hosted = Arc::new(fake::FakeProvider::default());
        hosted.machines.lock().insert("sbx-orphan".into(), ProviderState::Running);
        let (manager, _store, _card, _config, _dir) = setup_with(vec![local, hosted.clone()]);

        assert_eq!(manager.sweep().await.unwrap(), 1);
        assert_eq!(*hosted.deletes.lock(), vec!["sbx-orphan".to_string()]);
    }

    #[tokio::test]
    async fn a_provider_is_asked_how_it_is_once_until_something_makes_the_answer_wrong() {
        // Settings draws this, the prompt asks it per turn, and every automatic
        // resolution asks it again. Probing a local runtime is three processes.
        let local = Arc::new(fake::FakeProvider::local());
        let hosted = Arc::new(fake::FakeProvider::default());
        let (manager, _store, _card, _config, _dir) =
            setup_with(vec![local.clone(), hosted.clone()]);

        let first = manager.statuses().await;
        manager.statuses().await;

        assert_eq!(
            first.iter().map(|(which, _)| *which).collect::<Vec<_>>(),
            vec![Provider::AppleContainer, Provider::E2b],
            "one row per provider this build knows, in the order automatic tries them"
        );
        assert_eq!((*local.probes.lock(), *hosted.probes.lock()), (1, 1));

        manager.invalidate();
        manager.statuses().await;
        assert_eq!((*local.probes.lock(), *hosted.probes.lock()), (2, 2));
    }

    #[tokio::test]
    async fn a_service_this_app_started_is_not_still_reported_as_stopped() {
        let local = Arc::new(fake::FakeProvider::local());
        *local.probe.lock() = Some(says(ProviderReadiness::NotRunning, true, "stopped"));
        let (manager, _store, card, _config, _dir) = setup_with(vec![local.clone()]);
        manager.statuses().await;

        manager.ensure(&card, BTreeMap::new()).await.unwrap();
        *local.probe.lock() = Some(ProviderStatus::ready("running now"));

        assert_eq!(manager.statuses().await[0].1.state, ProviderReadiness::Ready);
        assert_eq!(*local.probes.lock(), 2, "starting it is a change, not a cache hit");
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
        let config = Arc::new(parking_lot::RwLock::new(AppConfig::default()));
        // Named rather than left automatic: this is the real registry, and what
        // `automatic` would resolve to depends on what the machine running the
        // test has installed.
        config.write().computer.provider = ProviderChoice::Provider(Provider::E2b);
        let manager = ComputerManager::new(store, config);
        let Err(err) = manager.ensure(&card, BTreeMap::new()).await else {
            panic!("there is nothing to make a machine with");
        };
        assert!(matches!(err, ComputerError::Unconfigured(_)), "{err}");
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

    #[tokio::test]
    async fn the_viewer_url_names_the_websocket_path_because_novnc_will_not_infer_it() {
        // noVNC joins the page's host with `/` + `path`; left to its default it
        // asked the proxy for `/websockify`, which names no computer, and drew
        // "Failed to connect to server" over a desktop that was up.
        let provider = Arc::new(fake::FakeProvider::default());
        *provider.replies.lock() =
            vec![Output { stdout: "up\n".into(), stderr: String::new(), exit_code: 0 }];
        let id = ComputerId::new();
        let handle = ProviderHandle {
            computer: id,
            provider_id: "m".into(),
            control_secret: Secret::new(""),
            viewer_secret: Secret::new(""),
        };
        let machine = Machine::new(provider, handle, BTreeMap::new(), 4321);

        let url = machine.vnc_url().await.expect("the port answered");

        assert!(url.starts_with(&format!("http://127.0.0.1:4321/{id}/6080/vnc.html?")), "{url}");
        assert!(url.contains(&format!("&path={id}/6080/websockify")), "{url}");
        assert!(url.contains("autoconnect=1"), "{url}");
    }
}
