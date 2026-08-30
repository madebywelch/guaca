//! The IPC surface.
//!
//! Everything the webview can ask for, and nothing else. Note what is absent:
//! there is no command that returns the API key, and no command that performs
//! network access on the frontend's behalf with a caller-supplied URL. The
//! webview never holds a credential.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;

use serde::{Deserialize, Serialize};

use crate::account::{Account, AccountError, Connectors};
use crate::artifact::Artifacts;
use crate::config::{self, AppConfig, RedactedConfig};
use crate::domain::agent::{copy_name, hire_names, AgentCard, AgentDraft, Consent, Lifecycle};
use crate::domain::approval::{Approval, ApprovalState, Decision, ProtectedAction};
use crate::domain::attachment::Attachment;
use crate::domain::connector::{Connector, ConnectorDraft};
use crate::domain::deployment::{Absent, Capabilities, Deployment};
use crate::domain::envelope::Envelope;
use crate::domain::escalation::Escalation;
use crate::domain::group::{Group, GroupDraft, GroupInference};
use crate::domain::ids::{
    AgentId, ApprovalId, ConnectorId, EscalationId, GroupId, MessageId, OccasionId, PluginId,
    RepositoryId, RoutineId, RunId,
};
use crate::domain::now_ms;
use crate::domain::occasion::{self, Occasion};
use crate::domain::plugin::{
    self, HeaderPair, Headers, Plugin, PluginAccess, PluginKind, PluginOffer, ServerReport,
};
use crate::domain::repository::{
    Bench, Gate, Harness, Repository, RepositoryDraft, RepositoryEdit,
};
use crate::domain::routine::{self, Routine, RoutineRun, Trigger};
use crate::domain::search::SearchHits;
use crate::domain::signin::Signin;
use crate::domain::usage::{GroupUsage, RunUsage};
use crate::domain::worknote::WorkingNote;
use crate::e2b::{Computer, E2bClient, E2bError};
use crate::kernel::{Browser, KernelClient, KernelError};
use crate::llm::catalog::{Catalog, CatalogError, RankedModel};
use crate::runtime::events::{Activity, UiEvent};
use crate::runtime::guard::GuardLimits;
use crate::runtime::Runtime;
use crate::subscription::{DeviceCode, SigninError, Status, Subscription};

/// How the operator's own browser is opened, which is a property of the host.
///
/// A sign-in that needs a person goes to the browser they are sitting in front
/// of. On a desktop that is this machine's, and Tauri opens it. On a server
/// there is nobody at the machine at all, so the honest answer is a refusal
/// naming where the sign-in has to happen instead.
///
/// A boxed function rather than a match on [`Deployment`], because the desktop
/// arm is the one line in this file that would drag `tauri` back into it, and
/// this module is the boundary that keeps the runtime host-agnostic.
pub type OpenUrl = Arc<dyn Fn(&str) -> Result<(), String> + Send + Sync>;

/// Where a sign-in's browser is sent back to, which is the other half of how
/// this host reaches a person.
///
/// A desktop lands the redirect on a loopback port it bound itself. A server
/// lands it on a route, at the origin the operator's browser reached the
/// workspace through, which nothing at boot can know: a box is behind a tunnel
/// whose name is the operator's. So it is either told (`GUACA_ORIGIN`) or
/// remembered from the last call that arrived, and a sign-in started before
/// either is refused with the sentence that says which to do.
pub enum Reach {
    Desktop,
    Served {
        /// `GUACA_ORIGIN`, when the operator set it. Wins over what was seen.
        fixed: Option<String>,
        /// Scheme and host of the last call, as the browser spelled them.
        seen: std::sync::Mutex<Option<String>>,
        callbacks: crate::oauth::Callbacks,
    },
}

impl Reach {
    pub fn served(fixed: Option<String>) -> Self {
        Reach::Served {
            fixed: fixed
                .map(|origin| origin.trim_end_matches('/').to_string())
                .filter(|o| !o.is_empty()),
            seen: std::sync::Mutex::new(None),
            callbacks: Default::default(),
        }
    }

    /// Remembers the origin a call arrived at. A desktop has nothing to note.
    pub fn note(&self, origin: String) {
        if let Reach::Served { seen, .. } = self {
            *seen.lock().unwrap_or_else(std::sync::PoisonError::into_inner) = Some(origin);
        }
    }

    /// The origin a browser would be sent back to, when there is one.
    pub fn origin(&self) -> Option<String> {
        match self {
            Reach::Desktop => None,
            Reach::Served { fixed, seen, .. } => fixed
                .clone()
                .or_else(|| seen.lock().unwrap_or_else(std::sync::PoisonError::into_inner).clone()),
        }
    }

    /// The callback map a served route delivers into. None on a desktop.
    pub fn callbacks(&self) -> Option<crate::oauth::Callbacks> {
        match self {
            Reach::Desktop => None,
            Reach::Served { callbacks, .. } => Some(callbacks.clone()),
        }
    }

    /// The landing for one sign-in, or the sentence for why there is none yet.
    pub fn landing(&self) -> Result<crate::oauth::Landing, CommandError> {
        match self {
            Reach::Desktop => Ok(crate::oauth::Landing::Loopback),
            Reach::Served { callbacks, .. } => match self.origin() {
                Some(origin) => {
                    Ok(crate::oauth::Landing::Served { origin, callbacks: callbacks.clone() })
                }
                None => Err(CommandError::new(
                    "config",
                    "this workspace does not know the address a browser reaches it at, so it \
                     cannot name where a sign-in should come back to. Open it in a browser \
                     first, or set GUACA_ORIGIN on the box",
                )),
            },
        }
    }
}

/// Hands the menu bar a presence read somewhere other than this runtime, or
/// `None` to put it back on this runtime. A desktop wires this to its tray;
/// a server has no strip and ignores it.
pub type MenubarFeed = Arc<dyn Fn(Option<crate::menubar::Presence>) + Send + Sync>;

pub struct AppState {
    pub runtime: Runtime,
    pub menubar: MenubarFeed,
    /// The workspace token, on a host that has one. What a screen ticket is
    /// derived from: a computer's live screen is reached through the daemon
    /// at an address noVNC has to be able to resolve its own files against,
    /// so the credential is a path segment, and it is a ticket for that one
    /// sandbox rather than the token for the whole workspace. `None` on a
    /// desktop, whose viewer is on loopback and needs nothing.
    pub secret: Option<Arc<str>>,
    /// Where this runtime is, which decides what the panels may offer.
    ///
    /// Read by the commands that would otherwise reach the operator's machine,
    /// and by `capabilities`, which is what the frontend gates on. Every
    /// refusal is drawn before anything is spent rather than at turn time.
    pub deployment: Deployment,
    /// Opens a URL in the operator's browser, or says why it cannot.
    pub open_url: OpenUrl,
    /// Where that browser comes back to.
    pub reach: Reach,
    pub config_path: PathBuf,
    /// Where a saved copy of an attachment lands. Resolved once at startup:
    /// the operating system's own downloads folder is the one place a person
    /// already knows to look.
    pub downloads: PathBuf,
    /// The ChatGPT sign-in. The same one the runtime makes calls with, so a
    /// sign-in completed here is usable by the next turn without a restart.
    pub subscription: Arc<Subscription>,
    /// The Guaca account, which is optional and which nothing else in the app
    /// depends on. An install that never signs in never reaches the service.
    pub account: Arc<Account>,
    /// OpenRouter's ranked model list, read while an agent's dialog is open and
    /// at no other time. No turn, prompt, tool or guard reads it, and an install
    /// that never opens that dialog never asks for it.
    pub catalog: Arc<Catalog>,
    /// The pages a transcript currently has framed. Not a store: see
    /// `frame_artifact`.
    pub artifacts: Artifacts,
    /// Where those are served from. Set once, at startup, after the OS has
    /// picked the port.
    pub artifact_port: Arc<std::sync::atomic::AtomicU16>,
}

/// A structured error the UI can render as more than a toast.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CommandError {
    /// Machine-readable discriminant, so the UI can react to a duplicate name
    /// differently from a disk failure.
    pub kind: &'static str,
    pub message: String,
}

impl CommandError {
    pub fn new(kind: &'static str, message: impl Into<String>) -> Self {
        Self { kind, message: message.into() }
    }
}

/// A capability this host does not have, as the panel reads it.
///
/// Its own kind rather than `validation`, because the two are different
/// problems: a validation failure is something the operator typed wrongly and
/// can retype, and this is something that is true of where the workspace runs.
/// A panel that can tell them apart can draw the second as a disabled control
/// with a reason rather than as a rejected form.
impl From<Absent> for CommandError {
    fn from(absent: Absent) -> Self {
        CommandError::new("notHere", absent.sentence())
    }
}

/// Refuses inference settings a server cannot honor, wherever they are written.
///
/// One function because there are three places: the app's settings, a group's
/// overrides, and the test button beside each. Three copies of this rule would
/// be three sentences for one fact, and the one an operator hit would depend on
/// which box they typed in.
///
/// Both checks are about the operator's own machine rather than about a feature
/// being unfinished. A `claude` signed in on a laptop cannot be reached from a
/// box, and neither can a model server on loopback.
fn honorable(
    here: Capabilities,
    provider: Option<config::Provider>,
    base_url: Option<&str>,
) -> Result<(), CommandError> {
    if provider == Some(config::Provider::Claude) {
        here.require(Absent::ClaudeProvider)?;
    }
    // Only when it is actually being set. A group that inherits the endpoint is
    // not making a claim about one, and refusing it would make every group edit
    // on a server fail over a field nobody touched.
    if let Some(url) = base_url.filter(|url| !url.trim().is_empty()) {
        if crate::domain::deployment::is_loopback(url) {
            here.require(Absent::LoopbackEndpoints)?;
        }
    }
    Ok(())
}

impl From<crate::db::StoreError> for CommandError {
    fn from(err: crate::db::StoreError) -> Self {
        use crate::db::StoreError;
        match err {
            StoreError::DuplicateName(_) => CommandError::new("duplicateName", err.to_string()),
            StoreError::AgentNotFound(_)
            | StoreError::GroupNotFound(_)
            | StoreError::ApprovalNotFound(_)
            | StoreError::ConnectorNotFound(_)
            | StoreError::RepositoryNotFound(_) => CommandError::new("notFound", err.to_string()),
            // Its own kind: a request answered twice, or answered after it
            // lapsed, is a stale button rather than anything being wrong. The
            // UI redraws it instead of showing a failure.
            StoreError::ApprovalSettled { .. } => {
                CommandError::new("alreadyAnswered", err.to_string())
            }
            // An operator naming a variable twice is a mistake they can fix,
            // not a disk failure, and the message already says which name.
            StoreError::DuplicateEnvVar(_)
            | StoreError::DuplicateRepository(_)
            | StoreError::AgentNotInGroupForRepository(_) => {
                CommandError::new("validation", err.to_string())
            }
            // Its own kind: the UI can offer to move the agents, which it
            // cannot do for a generic storage failure.
            StoreError::GroupNotEmpty { .. } | StoreError::CannotDeleteDefaultGroup => {
                CommandError::new("groupNotEmpty", err.to_string())
            }
            other => CommandError::new("storage", other.to_string()),
        }
    }
}

impl From<crate::runtime::RuntimeError> for CommandError {
    fn from(err: crate::runtime::RuntimeError) -> Self {
        use crate::runtime::RuntimeError;
        match err {
            RuntimeError::Store(inner) => inner.into(),
            RuntimeError::UnknownAgent(_) => CommandError::new("notFound", err.to_string()),
            RuntimeError::AgentTerminated(_) => CommandError::new("terminated", err.to_string()),
            RuntimeError::NothingToRetry => CommandError::new("notFound", err.to_string()),
            // A precondition the operator can fix from the rail, so it says so
            // rather than reading as something that broke.
            RuntimeError::NoRepository(_)
            | RuntimeError::RepositoryBusy { .. }
            | RuntimeError::NoWorkTree { .. } => CommandError::new("badRequest", err.to_string()),
            // A `shell` failure is answered to the model inside its turn and
            // never to the webview: no command here runs one. The arm exists
            // because the enum is one enum, and it says what it would say if a
            // command ever did: a directory that has moved is something the
            // operator fixes from the repository's panel.
            RuntimeError::Shell(_) => CommandError::new("badRequest", err.to_string()),
            // A job that ended between the panel drawing a button and the
            // operator pressing it, and two facts about a job that is running.
            // None of them is a failure: each says what the operator can do
            // instead, which for the first is nothing at all.
            RuntimeError::NoJobRunning
            | RuntimeError::JobStillStarting
            | RuntimeError::JobUnreachable(_) => CommandError::new("badRequest", err.to_string()),
            // All three are this side answering a request with the wrong shape
            // of answer, which is a defect here rather than something the
            // operator did. Reported as an ordinary failure so it lands in the
            // banner with the sentence that says which shape was expected.
            RuntimeError::NotAVerdict | RuntimeError::NotAQuestion | RuntimeError::EmptyAnswer => {
                CommandError::new("badRequest", err.to_string())
            }
        }
    }
}

impl From<AccountError> for CommandError {
    fn from(err: AccountError) -> Self {
        match err {
            // Its own kind: the UI redraws itself as signed out rather than
            // showing a failure for a sign-in that simply ended.
            AccountError::NotSignedIn | AccountError::Expired { .. } => {
                CommandError::new("signedOut", err.to_string())
            }
            other => CommandError::new("account", other.to_string()),
        }
    }
}

impl From<crate::domain::agent::DraftError> for CommandError {
    fn from(err: crate::domain::agent::DraftError) -> Self {
        CommandError::new("validation", err.to_string())
    }
}

impl From<crate::domain::group::GroupError> for CommandError {
    fn from(err: crate::domain::group::GroupError) -> Self {
        CommandError::new("validation", err.to_string())
    }
}

impl From<crate::domain::connector::ConnectorError> for CommandError {
    fn from(err: crate::domain::connector::ConnectorError) -> Self {
        CommandError::new("validation", err.to_string())
    }
}

impl From<crate::domain::repository::RepositoryError> for CommandError {
    fn from(err: crate::domain::repository::RepositoryError) -> Self {
        CommandError::new("validation", err.to_string())
    }
}

impl From<crate::repo::RepoError> for CommandError {
    fn from(err: crate::repo::RepoError) -> Self {
        // Every one of these is something the operator can fix in the dialog
        // they are already looking at: pick a different directory, run
        // `git init`, install git. Reported as validation so it lands beside
        // the field rather than in a banner about storage.
        CommandError::new("validation", err.to_string())
    }
}

impl From<crate::plugins::PluginError> for CommandError {
    fn from(err: crate::plugins::PluginError) -> Self {
        // Its own kind, because the UI can offer the one thing that fixes it:
        // open the browser again. Everything else here is a failure to report.
        match err {
            crate::plugins::PluginError::Signin(_) => {
                CommandError::new("pluginSignin", err.to_string())
            }
            other => CommandError::new("plugin", other.to_string()),
        }
    }
}

impl From<E2bError> for CommandError {
    fn from(err: E2bError) -> Self {
        match err {
            // Its own kind so the UI can offer to open settings rather than
            // showing a failure for something that was simply never set up.
            E2bError::NoKey => CommandError::new("computerUnconfigured", err.to_string()),
            other => CommandError::new("computer", other.to_string()),
        }
    }
}

impl From<KernelError> for CommandError {
    fn from(err: KernelError) -> Self {
        match err {
            // Its own kind, for the same reason the computer has one: never set
            // up is not a failure, and the UI can offer to open settings.
            KernelError::NoKey => CommandError::new("browserUnconfigured", err.to_string()),
            other => CommandError::new("browser", other.to_string()),
        }
    }
}

impl From<config::ConfigError> for CommandError {
    fn from(err: config::ConfigError) -> Self {
        CommandError::new("config", err.to_string())
    }
}

impl From<SigninError> for CommandError {
    fn from(err: SigninError) -> Self {
        match err {
            // Its own kind so the dialog can offer to start again rather than
            // showing a failure for a code the operator simply did not get to
            // in time. Nothing is wrong and nothing needs fixing.
            SigninError::TimedOut => CommandError::new("signinExpired", err.to_string()),
            other => CommandError::new("signin", other.to_string()),
        }
    }
}

impl From<CatalogError> for CommandError {
    fn from(err: CatalogError) -> Self {
        match err {
            // Its own kind because it is this app's defect, not OpenRouter's:
            // the webview asked for a use case the vendor does not rank, so the
            // two lists have drifted. Nothing an operator can do about it, and
            // nothing an operator should be shown a network failure for.
            CatalogError::Unsupported { .. } | CatalogError::Withdrawn(_) => {
                CommandError::new("useCase", err.to_string())
            }
            other => CommandError::new("catalog", other.to_string()),
        }
    }
}

pub type Reply<T> = Result<T, CommandError>;

// ---- computers -----------------------------------------------------------

/// The E2B client, or a clear reason there is not one.
fn computers(state: &AppState) -> Reply<E2bClient> {
    E2bClient::new(&state.runtime.config().e2b.api_key).ok_or_else(|| E2bError::NoKey.into())
}

fn agent_card(state: &AppState, id: AgentId) -> Reply<crate::domain::agent::AgentCard> {
    state
        .runtime
        .store()
        .get_agent(id)?
        .ok_or_else(|| CommandError::new("notFound", format!("no agent with id {id}")))
}

/// What an agent's computer is doing right now.
///
/// `None` means it has never been given one, which the UI shows as an offer
/// rather than as an error.
pub async fn agent_computer(state: &AppState, id: AgentId) -> Reply<Option<Computer>> {
    let card = agent_card(state, id)?;
    let (Some(sandbox), Some(envd)) = (card.sandbox_id, card.sandbox_envd_token) else {
        return Ok(None);
    };
    let client = computers(state)?;

    if client.state(&sandbox).await? == crate::e2b::SandboxState::Gone {
        // A reclaimed sandbox leaves a dangling id. Clearing it turns a dead
        // end into an offer to make a new one. A sleeping one is left alone:
        // it still holds the disk, and waking it is the operator's call.
        state.runtime.store().set_agent_sandbox(id, None)?;
        return Ok(None);
    }
    Ok(Some(state.screened(client.describe(&sandbox, &envd, state.runtime.viewer_port()).await?)))
}

/// Gives an agent a computer.
///
/// The decision and nothing else: no machine is made here, and an agent that
/// never needs one never costs anything. What changes is what its turns are
/// offered, which is the whole of what the operator is deciding.
pub async fn give_agent_computer(state: &AppState, id: AgentId) -> Reply<()> {
    state.runtime.store().set_has_computer(id, true)?;
    state.runtime.emit(UiEvent::AgentsChanged);
    Ok(())
}

/// Takes it back, and puts the machine to sleep if there is one.
///
/// Asleep rather than destroyed, and the disk is the reason: it holds whatever
/// the operator signed that machine in to, so giving the computer back later
/// has to find those sessions rather than a stranger. Destroying is its own
/// button and says what it does.
///
/// A sandbox that cannot be reached is not a failure here. The decision has
/// already been recorded by then, and the machine sleeps on its own timeout;
/// refusing would leave an agent holding a computer the operator has said it
/// may not have.
pub async fn take_agent_computer(state: &AppState, id: AgentId) -> Reply<()> {
    let card = agent_card(state, id)?;
    state.runtime.store().set_has_computer(id, false)?;

    if let Some(sandbox) = card.sandbox_id {
        match computers(state) {
            Ok(client) => {
                if let Err(err) = client.pause(&sandbox).await {
                    tracing::warn!(%err, %sandbox, "could not sleep a machine that was taken back");
                }
            }
            Err(err) => tracing::warn!(?err, "no client to sleep a machine that was taken back"),
        }
    }
    state.runtime.emit(UiEvent::AgentsChanged);
    Ok(())
}

/// Brings the desktop up, making or waking the machine as needed.
///
/// Refuses for an agent that has not been given a computer, which the panel
/// does not offer: this is the operator's own route to the same gate the
/// runtime uses, and one that granted by side effect would make the give
/// button decorative.
pub async fn start_agent_computer(state: &AppState, id: AgentId) -> Reply<Computer> {
    let card = agent_card(state, id)?;
    let (client, sandbox) = state.runtime.ensure_computer(&card).await?;

    client.start_desktop(&sandbox.id, &sandbox.envd_token).await?;
    let computer =
        client.describe(&sandbox.id, &sandbox.envd_token, state.runtime.viewer_port()).await?;
    state.runtime.emit(UiEvent::AgentsChanged);
    Ok(state.screened(computer))
}

/// Puts an agent's machine to sleep.
///
/// Not a delete: the disk is kept, so a browser that was signed in still is
/// when it wakes. This is what a bill-conscious operator wants, and what the
/// idle timeout does on its own.
pub async fn stop_agent_computer(state: &AppState, id: AgentId) -> Reply<Option<Computer>> {
    let card = agent_card(state, id)?;
    let (Some(sandbox), Some(envd)) = (card.sandbox_id, card.sandbox_envd_token) else {
        return Ok(None);
    };
    let client = computers(state)?;
    client.pause(&sandbox).await?;
    state.runtime.emit(UiEvent::AgentsChanged);
    Ok(Some(state.screened(client.describe(&sandbox, &envd, state.runtime.viewer_port()).await?)))
}

/// Destroys the sandbox and everything on its disk.
pub async fn delete_agent_computer(state: &AppState, id: AgentId) -> Reply<()> {
    let Some(sandbox) = agent_card(state, id)?.sandbox_id else {
        return Ok(());
    };
    computers(state)?.kill(&sandbox).await?;
    state.runtime.store().set_agent_sandbox(id, None)?;
    state.runtime.emit(UiEvent::AgentsChanged);
    Ok(())
}

// ---- browsers ------------------------------------------------------------

/// The Kernel client, or a clear reason there is not one.
fn browsers(state: &AppState) -> Reply<KernelClient> {
    KernelClient::new(&state.runtime.config().kernel.api_key)
        .ok_or_else(|| KernelError::NoKey.into())
}

/// What an agent's browser is doing right now.
///
/// `None` means it has never been given one, or the one it had has gone, which
/// the UI shows as an offer rather than as an error. Gone is the ordinary end of
/// every browser and costs nothing: the cookies went back to the agent's
/// profile, so the next one opens signed in to the same accounts.
pub async fn agent_browser(state: &AppState, id: AgentId) -> Reply<Option<Browser>> {
    let Some(browser) = agent_card(state, id)?.browser_id else {
        return Ok(None);
    };
    let client = browsers(state)?;

    match client.get(&browser).await? {
        Some(session) => Ok(Some(Browser::running(session))),
        None => {
            // A dangling id is a dead end in the pane. Clearing it turns that
            // back into an offer to open one.
            state.runtime.store().set_agent_browser(id, None)?;
            Ok(None)
        }
    }
}

/// Gives an agent a browser.
///
/// As with the computer: the decision alone. A browser is opened on first use,
/// or by the operator when they want to sign this agent in to something.
pub async fn give_agent_browser(state: &AppState, id: AgentId) -> Reply<()> {
    state.runtime.store().set_has_browser(id, true)?;
    state.runtime.emit(UiEvent::AgentsChanged);
    Ok(())
}

/// Whether that browser stops and asks before it acts in the operator's name.
///
/// Separate from giving and taking the browser back, because it answers a
/// different question and outlives every browser made under it. See
/// [`crate::domain::agent::Consent`], which is where the argument for deciding
/// this per agent lives.
pub fn set_agent_browser_consent(
    state: &AppState,
    id: AgentId,
    consent: Consent,
) -> Reply<()> {
    state.runtime.store().set_browser_consent(id, consent)?;
    state.runtime.emit(UiEvent::AgentsChanged);
    Ok(())
}

/// Takes it back, and closes the browser if one is open.
///
/// Closing is what writes the cookies back to the agent's profile, so this
/// keeps what the operator signed it in to exactly as the Close button does.
/// The profile outlives every browser made against it and is deleted with the
/// agent, so giving the browser back opens one signed in to the same accounts.
pub async fn take_agent_browser(state: &AppState, id: AgentId) -> Reply<()> {
    let card = agent_card(state, id)?;
    state.runtime.store().set_has_browser(id, false)?;

    if let Some(browser) = card.browser_id {
        match browsers(state) {
            Ok(client) => {
                if let Err(err) = client.delete(&browser).await {
                    tracing::warn!(%err, %browser, "could not close a browser that was taken back");
                } else {
                    state.runtime.store().set_agent_browser(id, None)?;
                }
            }
            Err(err) => tracing::warn!(?err, "no client to close a browser that was taken back"),
        }
    }
    state.runtime.emit(UiEvent::AgentsChanged);
    Ok(())
}

/// Opens an agent's browser, or hands back the one it has.
pub async fn start_agent_browser(state: &AppState, id: AgentId) -> Reply<Browser> {
    let card = agent_card(state, id)?;
    let (_, session) = state.runtime.ensure_browser(&card).await?;
    state.runtime.emit(UiEvent::AgentsChanged);
    Ok(Browser::running(session))
}

/// Ends an agent's browser, keeping what it is signed in to.
///
/// The counterpart of putting a machine to sleep, and the closest thing a
/// browser has: deleting is what writes the cookies back to the agent's
/// profile, so this is how an operator makes a sign-in they just performed
/// durable rather than waiting for the timeout to do it.
pub async fn stop_agent_browser(state: &AppState, id: AgentId) -> Reply<()> {
    let Some(browser) = agent_card(state, id)?.browser_id else {
        return Ok(());
    };
    browsers(state)?.delete(&browser).await?;
    state.runtime.store().set_agent_browser(id, None)?;
    state.runtime.emit(UiEvent::AgentsChanged);
    Ok(())
}

// ---- connectors ----------------------------------------------------------

/// Every account one crew can reach. Secrets are reported as set or not; the
/// values are not in this type and there is no command that returns them.
pub async fn group_connectors(state: &AppState, group_id: GroupId) -> Reply<Vec<Connector>> {
    Ok(state.runtime.store().group_connectors(group_id)?)
}

pub async fn create_connector(state: &AppState, draft: ConnectorDraft) -> Reply<Connector> {
    let clean = draft.validate()?;
    let connector = state.runtime.store().create_connector(&clean)?;
    // The roster every agent is shown includes what its peers can reach, so a
    // new account changes what the whole crew knows.
    state.runtime.emit(UiEvent::AgentsChanged);
    Ok(connector)
}

pub async fn delete_connector(state: &AppState, id: ConnectorId) -> Reply<()> {
    state.runtime.store().delete_connector(id)?;
    state.runtime.emit(UiEvent::AgentsChanged);
    Ok(())
}

// ---- repositories --------------------------------------------------------

/// The directories one crew has linked, and who in it may work in each.
///
/// No filesystem is touched here. A repository that has been moved or deleted
/// on disk since it was linked still comes back, because the panel is where the
/// operator fixes that and a list that silently dropped a row would leave them
/// nothing to fix.
pub async fn group_repositories(state: &AppState, group_id: GroupId) -> Reply<Vec<Repository>> {
    Ok(state.runtime.store().group_repositories(group_id)?)
}

/// What every linked repository is doing right now, by id.
///
/// One call for the rail rather than one per row, and every repository is asked
/// concurrently: the git half is local and instant, the `gh` half is a network
/// round trip, and asked in series a crew with four codebases would spend more
/// than a second before the first branch name appeared.
///
/// A repository that cannot be read is absent from the map rather than present
/// and empty. The directory may have been moved or unmounted since it was
/// linked, and a row saying `main, clean` about a path that is no longer there
/// is worse than a row saying nothing.
pub async fn repository_statuses(
    state: &AppState,
) -> Reply<std::collections::HashMap<RepositoryId, crate::repo::RepoStatus>> {
    let repositories = state.runtime.store().repositories()?;
    let asked = repositories.into_iter().map(|repository| async move {
        crate::repo::status(&repository.path).await.map(|status| (repository.id, status))
    });
    Ok(futures_util::future::join_all(asked).await.into_iter().flatten().collect())
}

/// Every repository in the workspace, with who may work in each.
///
/// One read for the whole rail. The crews column and the rail inside a crew are
/// drawn from one roster, and a call per crew would make the round trips the
/// number of crews.
pub async fn list_repositories(state: &AppState) -> Reply<Vec<Repository>> {
    Ok(state.runtime.store().repositories()?)
}

/// Links a directory to a crew, after checking that it is one.
///
/// The check is the point of the command being async: it runs git, and it is
/// the only moment anything asks the disk whether this path is real. Everything
/// after it works from the canonical path git agreed to, so two spellings of
/// one directory cannot become two repositories.
///
/// Nobody is given it here. Adding and handing out are two decisions, and the
/// second one is `set_repository_access`.
pub async fn create_repository(state: &AppState, draft: RepositoryDraft) -> Reply<Repository> {
    // Before the path is even cleaned. On a server there is no directory
    // for the operator to have picked, and verifying one would answer with
    // git's complaint about a path rather than with the reason.
    state.deployment.capabilities().require(Absent::LocalDirectories)?;

    let mut clean = draft.clean()?;
    clean.path = crate::repo::verify(&clean.path).await?;
    // Taken from the canonical path rather than the typed one, for the case
    // where they differ: a directory reached through a symlink would otherwise
    // be named for the link and drawn beside a path that says something else.
    if draft.name.trim().is_empty() {
        clean.name = clean.path.rsplit('/').next().unwrap_or(&clean.path).to_string();
    }

    let repository = state.runtime.store().create_repository(&clean)?;
    // The roster every agent is shown says what its peers can reach, and a
    // repository is now part of that.
    state.runtime.emit(UiEvent::AgentsChanged);
    Ok(repository)
}

/// Renames one, rewrites the line its agents read on every turn, changes which
/// program does the writing, or moves where that program works.
///
/// The path is not editable and is not a parameter. A different directory is a
/// different repository: editing the path in place would move every named
/// agent's boundary with nothing on screen saying so. The harness is the
/// opposite case and is editable for the same reason the note is: it says how
/// work happens in a directory the operator already chose, and the day it needs
/// changing is the day one of the two sign-ins stops paying.
///
/// [`RepositoryEdit`] rather than a [`RepositoryDraft`] with a stand-in path,
/// and that is a bug fix rather than tidying: the stand-in was `/`, which
/// cleans down to the empty string, so every call here was refused with *a
/// repository needs a directory; pick one to link*. Its doc comment is the
/// long version.
///
/// A job already running is not affected. It is a process that was started with
/// the old answer, and reaching into it would be a second way to stop a job that
/// `Runtime::start_job` does not have. That covers the bench too: switching a
/// repository to the linked directory while a job is writing in a worktree
/// leaves that job exactly where it is, and the next one starts in the new
/// place. Worktrees an agent is no longer using are left on disk rather than
/// removed here, because a switch back has to find its caches where it left
/// them, and `repo::release_bench` is what actually takes one away.
pub async fn update_repository(
    state: &AppState,
    id: RepositoryId,
    name: String,
    note: String,
    harness: Harness,
    gate: Gate,
    bench: Bench,
) -> Reply<Repository> {
    let clean = RepositoryEdit { name, note, harness, gate, bench }.clean()?;
    let repository = state.runtime.store().update_repository(
        id,
        &clean.name,
        &clean.note,
        clean.harness,
        clean.gate,
        clean.bench,
    )?;
    state.runtime.emit(UiEvent::AgentsChanged);
    Ok(repository)
}

/// One coding harness, as the panel that offers the choice needs it.
#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct HarnessOnMachine {
    pub harness: Harness,
    /// Whether the program is on this app's `PATH`.
    pub installed: bool,
    /// What it says its version is, or empty when it is not there.
    ///
    /// The program's own answer rather than a parse of it, so an operator
    /// comparing this panel with their terminal sees the same string.
    pub version: String,
    /// Whether a job on it can be reached while it runs.
    ///
    /// False for `pi`, which has no second interface, and for a Claude Code
    /// older than the one the bridge's contract was measured against. Neither
    /// stops a job: both run exactly as every job ran before the bridge
    /// existed, which is why this is a fact the panel states rather than a
    /// refusal. `coding::presence` is the argument.
    pub bridged: bool,
    /// How to get it if it is not. Sent from here rather than spelled in the
    /// webview, because it is the same string a refused job quotes at an agent,
    /// and two copies of an install command drift the day a vendor renames a
    /// package.
    pub install: &'static str,
    /// Why this host will not run it, when it will not.
    ///
    /// A row rather than an absence, for the reason every other capability here
    /// is drawn rather than hidden: a harness that silently vanishes from the
    /// list on a server is a panel that disagrees with the documentation and
    /// with the operator's own laptop, and nothing on screen explains it.
    /// `None` is the ordinary case and draws exactly what it always drew.
    pub withheld: Option<String>,
}

/// Which coding harnesses are on this machine, and how to get the ones that are
/// not.
///
/// Asked by the panel that offers the choice, so picking one the operator does
/// not have is answered at the moment they pick. Everything else about a job is
/// discovered when it runs; this one cannot be, because a job runs minutes after
/// the tool call that started it and its refusal reaches an agent rather than
/// the person who set the repository up.
///
/// Every harness comes back, installed or not, and they are asked concurrently:
/// each is a process spawn, and asked in series a panel waits once per harness.
pub async fn coding_harnesses(state: &AppState) -> Reply<Vec<HarnessOnMachine>> {
    let here = state.deployment.capabilities();
    let asked = Harness::ALL.map(|harness| async move {
        let withheld = match harness {
            Harness::Claude => here.require(Absent::ClaudeCodeHarness).err(),
            _ => None,
        };
        // Not probed when it is withheld. The probe is a process spawn whose
        // answer could not change anything, and "installed, and not offered"
        // is a sentence that invites an operator to go and fix the install.
        let (installed, version, bridged) = match withheld {
            Some(_) => (false, String::new(), false),
            None => match crate::coding::presence(harness).await {
                crate::coding::Presence::Missing => (false, String::new(), false),
                crate::coding::Presence::Installed { version, bridged } => (true, version, bridged),
            },
        };
        HarnessOnMachine {
            harness,
            installed,
            version,
            bridged,
            install: crate::coding::install(harness),
            withheld: withheld.map(|absent| absent.sentence().to_string()),
        }
    });
    Ok(futures_util::future::join_all(asked).await.into_iter().collect())
}

/// Sends a correction into a coding job that is already running.
///
/// The one thing an operator watching a job go the wrong way could not do. It
/// is staged rather than delivered: the job reads it at its next tool boundary,
/// or when it tries to finish, whichever comes first. `coding/bridge.rs` is
/// both halves.
///
/// Refused rather than swallowed when nothing takes it. A job that has just
/// ended, and a repository whose harness has no bridge, are two different
/// sentences and both are things the operator can act on: the second is why
/// the panel says which harness a repository runs.
pub async fn message_coding_job(state: &AppState, agent_id: AgentId, message: String) -> Reply<()> {
    state.runtime.message_job(agent_id, &message).map_err(Into::into)
}

/// Stops a coding job that is running, leaving whatever it has committed.
///
/// The process is killed. Nothing is reverted and nothing is cleaned up,
/// because the commits a job was told to make as it went are the operator's
/// checkpoints and throwing them away is not this button's decision to take.
/// The agent that started the job is told, on the same path it is told about
/// one that finished: an agent never told is an agent waiting forever.
pub async fn stop_coding_job(state: &AppState, agent_id: AgentId) -> Reply<()> {
    state.runtime.stop_job(agent_id).map_err(Into::into)
}

/// Unlinks a repository.
///
/// Nothing in the operator's own checkout is touched. The work trees this app
/// made for the agents that worked here do go, because each one is a
/// registration in that checkout and one left behind is an entry in their
/// `git worktree list` pointing into an app that has forgotten the directory.
pub async fn delete_repository(state: &AppState, id: RepositoryId) -> Reply<()> {
    state.runtime.unlink_repository(id).await?;
    state.runtime.emit(UiEvent::AgentsChanged);
    Ok(())
}

/// Puts one agent in a repository, or takes it out.
///
/// A move rather than a grant: an agent works in at most one, so `null` is how
/// it comes back out and there is no second call that takes one away. The rail
/// drops an agent onto a repository exactly as it drops one onto a crew, and
/// the two gestures mean the same kind of thing for the same reason.
pub async fn set_agent_repository(
    state: &AppState,
    id: AgentId,
    repository_id: Option<RepositoryId>,
) -> Reply<AgentCard> {
    let card = state.runtime.store().set_agent_repository(id, repository_id)?;
    state.runtime.emit(UiEvent::AgentsChanged);
    Ok(card)
}

// ---- plugins -------------------------------------------------------------

/// The servers Guaca knows how to sign in to. Static, and the same for every
/// group: what differs is which of them a crew has connected.
pub async fn plugin_catalog(_state: &AppState) -> Reply<Vec<PluginOffer>> {
    Ok(plugin::catalog())
}

/// What one crew has connected, and what each of those can do. No grant is on
/// this type and there is no command that returns one.
pub async fn group_plugins(state: &AppState, group_id: GroupId) -> Reply<Vec<Plugin>> {
    Ok(state.runtime.store().group_plugins(group_id)?)
}

/// Signs a group in to a plugin's server, opening the operator's browser if the
/// server asks for one.
///
/// Runs the whole flow inside one command, so the webview has no half-finished
/// sign-in to hold or to clean up. It can take minutes: the operator has to
/// authorize in a browser, and the command is what is waiting for them. Five
/// minutes and it gives up, which is also when the loopback socket closes.
pub async fn connect_plugin(
    state: &AppState,
    group_id: GroupId,
    kind: PluginKind,
    // Which of the account's authorized identities this crew should use.
    // Absent is the account's default, which is the right answer for an account
    // with one Google and what every already-connected plugin keeps doing.
    // Naming one is how two crews use two different mailboxes.
    connection: Option<String>,
) -> Reply<Plugin> {
    // Read here rather than inside the flow, so a machine that is not signed in
    // is told so before a browser opens. An account-backed plugin has no
    // sign-in of its own: see `PluginKind::account_backed`.
    let read = if kind.account_backed() { Some(state.account.access().await) } else { None };
    let connection = connection.unwrap_or_default();
    // An account that refused says so here rather than falling through to
    // `Discover`, which would open a browser at a vendor for a plugin whose
    // sign-in is the account's and report whatever came back as the problem.
    let credential = match &read {
        Some(read) => crate::plugins::Credential::Account(
            crate::plugins::Held::read(read, &connection).spend(&kind)?,
        ),
        None => crate::plugins::Credential::Discover,
    };

    // An account-backed plugin's server is the operator's own account, and
    // which identity it means is part of the address. The other kinds ignore
    // both and dial the vendor.
    let endpoint = if kind.account_backed() {
        crate::plugins::AccountUse::endpoint(state.account.origin(), &connection)
    } else {
        state.runtime.plugin_endpoint(&kind)
    };

    // A catalog server is dialled at the address this build ships, with nothing
    // on the request but the credential. Headers are the operator's answer to a
    // server nobody vouched for, and there is nothing here to answer.
    sign_in(state, group_id, &kind, &endpoint, credential, &Headers::none()).await
}

/// Adds a server the operator addressed themselves, and connects it.
///
/// Two fields, because two is what the catalog was supplying: a name, which
/// becomes the prefix every one of this server's tools is called by, and the
/// URL its MCP endpoint answers on. Everything after that is the flow the six
/// go through — the same era probe, the same sign-in, the same tool list, the
/// same per-agent and per-tool answers, and a grant that never reaches a model.
///
/// The key is optional and is the one thing the catalog never needs. A server
/// somebody wrote has often got no authorization server behind it at all, just
/// a token minted by hand, and asking that server to discover a sign-in is a
/// round trip whose only outcome is a 401 with nothing useful in it. Left out,
/// the server is asked what it wants exactly as a vendor's is.
///
/// The headers are the third thing, and they are not a third credential: they
/// are how the request reaches the server at all, so they go on every one of
/// them whichever of the other two paid for it. That is what makes an MCP
/// server behind Cloudflare Access work — the headers get past the gate, and
/// the 401 behind the gate still starts the sign-in.
///
/// Refused rather than silently replacing when the crew already has a server
/// under that name: `plugins_kind_unique` would take the collision as a
/// reconnection, which is right for the same address and wrong for a different
/// one — two tool lists under one prefix, with which one a call landed on
/// decided by row order.
pub async fn add_plugin(
    state: &AppState,
    group_id: GroupId,
    name: String,
    url: String,
    key: Option<String>,
    headers: Option<Vec<HeaderPair>>,
) -> Reply<Plugin> {
    let kind = PluginKind::custom(&name, &url)
        .map_err(|err| CommandError::new("validation", err.to_string()))?;

    let held = state.runtime.store().group_plugins(group_id)?;
    if let Some(clash) = held.iter().find(|held| held.kind.slug() == kind.slug()) {
        if clash.endpoint != kind.endpoint() {
            return Err(CommandError::new(
                "validation",
                format!(
                    "this group already has a server called {} at {}. Give this one another name, or \
                     disconnect that one first: two servers under one name would put two \
                     tool lists behind the same prefix.",
                    kind.slug(),
                    clash.endpoint
                ),
            ));
        }
    }

    let (key, headers) = presented(key, headers)?;
    let credential = match key.as_deref() {
        Some(key) => crate::plugins::Credential::Key(key),
        None => crate::plugins::Credential::Discover,
    };
    let endpoint = state.runtime.plugin_endpoint(&kind);
    sign_in(state, group_id, &kind, &endpoint, credential, &headers).await
}

/// What the operator gave, as the two things the connect path takes.
///
/// One function because the rule it enforces has to hold in both places that
/// take a key and headers, and because the rule is easy to state and easy to
/// get silently wrong: a key and an `authorization` header are the same slot on
/// the request. Both given, one would overwrite the other, and which one won
/// would depend on the order two loops happened to run in — so both given is
/// refused, in a sentence that says they are one thing rather than two.
///
/// The key comes back rather than the credential built from it, because the
/// credential borrows it: built here, it would name a string that goes out of
/// scope on the way back.
fn presented(
    key: Option<String>,
    headers: Option<Vec<HeaderPair>>,
) -> Result<(Option<String>, Headers), CommandError> {
    let key = key.map(|key| key.trim().to_string()).filter(|key| !key.is_empty());
    let headers = Headers::parse(&headers.unwrap_or_default())
        .map_err(|err| CommandError::new("validation", err.to_string()))?;
    if key.is_some() && headers.carries_authorization() {
        return Err(CommandError::new(
            "validation",
            "you gave a key and an `authorization` header, and they are the same slot on the \
             request. Keep the key and drop the header, or keep the header — which is how to \
             send a scheme other than `Bearer` — and clear the key.",
        ));
    }
    Ok((key, headers))
}

/// The half of connecting that is the same whatever the server is.
///
/// One function rather than two that agree, because the difference between a
/// catalog server and one the operator added is entirely in how the kind and
/// the credential were arrived at. After this line there is no difference at
/// all, and a second copy of it would be a second place for a crew's tools to
/// stop being refreshed or an event to stop being emitted.
async fn sign_in(
    state: &AppState,
    group_id: GroupId,
    kind: &PluginKind,
    endpoint: &str,
    credential: crate::plugins::Credential<'_>,
    headers: &Headers,
) -> Reply<Plugin> {
    // Asked before the flow starts, so a box that cannot name its own address
    // refuses here rather than after a browser has been opened.
    let landing = state.reach.landing()?;
    let plugin = crate::plugins::connect(
        state.runtime.store(),
        group_id,
        kind,
        endpoint,
        credential,
        headers,
        &landing,
        {
            // The flow takes a callback so `oauth.rs` does not have to know how
            // a browser is opened, and the host is what decides. On a desktop
            // this is the operator's own browser and the loopback redirect
            // lands back in this process. On a server the page is asked to
            // open it, and the redirect comes back through the served origin.
            let open = state.open_url.clone();
            move |url: &str| open(url)
        },
    )
    .await?;

    // The crew's tools changed, which changes what every agent in it is offered
    // on its next turn and what the roster says they can reach.
    state.runtime.emit(UiEvent::AgentsChanged);
    Ok(plugin)
}

/// Points a crew's plugin at a different authorized identity.
///
/// Kept apart from connecting, because it is not the same act: connecting reads
/// the server's tool list and replaces the row, and this moves an existing row
/// to another mailbox. Doing it through Connect would mean an operator who
/// wanted the other Google lost the per-tool switches they had set on this one.
///
/// The tool list is re-read, because two identities do not offer the same
/// tools: a grant that can read mail and not send it publishes fewer, and a row
/// left holding the old list would offer a model a tool that is no longer there.
pub async fn set_plugin_connection(
    state: &AppState,
    group_id: GroupId,
    kind: PluginKind,
    connection: String,
) -> Reply<Plugin> {
    if !kind.account_backed() {
        return Err(CommandError::new(
            "validation",
            format!("{} signs in per group and has no account identity to choose", kind.label()),
        ));
    }
    connect_plugin(state, group_id, kind, Some(connection)).await
}

/// Points a crew's custom server at a different address, or hands it a new key.
///
/// Reconnecting rather than editing a row, and that is the same act connecting
/// is: the tool list has to be re-read, because a server at a new address is
/// not the one that published the old list. What survives is what survives a
/// reconnection anywhere else — the row's id, who may spend it, and which of
/// its tools are whose — because the operator is fixing an address, not
/// deciding what the crew may do.
///
/// Sending headers replaces the whole set and sending none leaves them alone,
/// which is the rule a group's API key already has and for the same reason: a
/// value that cannot be read back is one a panel cannot re-send, so "absent"
/// has to mean keep or every reconnection would quietly drop it. Sending an
/// empty list is how they are removed, and it is a thing the operator did
/// rather than a thing they forgot.
///
/// The key is the other way round here, and that is this command's own history
/// rather than a second rule: absent means "ask the server", which is what the
/// box beside it says. It stays because a server that stopped needing a key is
/// otherwise unreachable from this panel.
pub async fn readdress_plugin(
    state: &AppState,
    group_id: GroupId,
    id: PluginId,
    url: String,
    key: Option<String>,
    headers: Option<Vec<HeaderPair>>,
) -> Reply<Plugin> {
    let held = state
        .runtime
        .store()
        .group_plugins(group_id)?
        .into_iter()
        .find(|held| held.id == id)
        .ok_or_else(|| CommandError::new("validation", "that plugin is not in this group"))?;
    if !held.custom {
        return Err(CommandError::new(
            "validation",
            format!(
                "{} is a server Guaca ships the address of, and where it lives is not an operator \
                 setting. Disconnect it if it is the wrong one.",
                held.name
            ),
        ));
    }

    // Rebuilt through the same constructor an added one goes through, so the
    // address is checked by the code that checked it the first time rather than
    // by a second copy of the rules here.
    let kind = PluginKind::custom(&held.name, &url)
        .map_err(|err| CommandError::new("validation", err.to_string()))?;
    let (key, headers) = match headers {
        Some(rows) => presented(key, Some(rows))?,
        // Read back off the row rather than left out of the write, so there is
        // one path into `save_plugin` and one meaning for what it is handed.
        None => {
            let stored = state
                .runtime
                .store()
                .plugin_dial(id)?
                .map(|dialed| dialed.headers)
                .unwrap_or_default();
            let (key, _) = presented(key, None)?;
            if key.is_some() && stored.carries_authorization() {
                return Err(CommandError::new(
                    "validation",
                    "this server already has an `authorization` header, and a key goes in the \
                     same slot. Replace its headers, or leave the key empty.",
                ));
            }
            (key, stored)
        }
    };
    let credential = match key.as_deref() {
        Some(key) => crate::plugins::Credential::Key(key),
        None => crate::plugins::Credential::Discover,
    };
    let endpoint = state.runtime.plugin_endpoint(&kind);
    sign_in(state, group_id, &kind, &endpoint, credential, &headers).await
}

/// Dials a server and says what it found, connecting nothing.
///
/// The button beside the address box. It runs the real path — the same probe,
/// both transports, the handshake, `tools/list` — with the credential and
/// headers the operator has typed but not yet saved, and writes nothing. What
/// comes back is either a report or the same sentence adding it would have
/// failed with, which is the point: an operator who has to press Add to find
/// out what is wrong is an operator connecting a server four times.
///
/// It stops one step short of `add_plugin` in exactly one place: a server that
/// wants a sign-in is reported as wanting one rather than sent to a browser.
/// The question here is whether this is the right address, and answering it
/// with a consent screen is a question nobody asked.
///
/// No group, because nothing is being connected to one, and no name, because a
/// name is what a server's tools are called by and this calls none of them.
pub async fn probe_server(
    _state: &AppState,
    url: String,
    key: Option<String>,
    headers: Option<Vec<HeaderPair>>,
) -> Reply<ServerReport> {
    // Through the same canonicalizer an added server goes through, so the
    // address that is tested is the address that would be stored. A test run
    // against `https://example.com/mcp/` that passes, followed by a sign-in
    // scoped to `https://example.com/mcp` that fails, is worse than no test.
    let endpoint = plugin::canonical_url(&url)
        .map_err(|err| CommandError::new("validation", err.to_string()))?;
    let (key, headers) = presented(key, headers)?;
    Ok(crate::plugins::inspect(true, &endpoint, key.as_deref(), &headers).await?)
}

/// The same question, asked of a plugin this crew already has.
///
/// Kept apart from `probe_server` because the credential is: this one spends
/// the grant in the store, which is the only way to answer "is our sign-in
/// still good", and that is the failure a crew notices as an agent reporting a
/// tool it cannot call. A stale grant is renewed first, because the next real
/// call would renew it too and a check that skipped it would report a working
/// plugin as broken.
pub async fn check_plugin(state: &AppState, id: PluginId) -> Reply<ServerReport> {
    let dialed = state
        .runtime
        .store()
        .plugin_dial(id)?
        .ok_or_else(|| CommandError::new("validation", "that plugin is not connected"))?;

    // The same two-line resolution `connect_plugin` does, and for the same
    // reason: an account-backed plugin's server is the operator's own account
    // and which identity it means is part of the address.
    let read = if dialed.kind.account_backed() { Some(state.account.access().await) } else { None };
    let endpoint = if dialed.kind.account_backed() {
        crate::plugins::AccountUse::endpoint(state.account.origin(), &dialed.connection)
    } else {
        state.runtime.plugin_endpoint(&dialed.kind)
    };
    // Copied off the row before it is handed over, because the check takes the
    // row and the identity travels with the token rather than with it.
    let connection = dialed.connection.clone();
    let account = match &read {
        Some(read) => crate::plugins::Held::read(read, &connection),
        // Not an account-backed kind, so nothing reads it.
        None => crate::plugins::Held::Absent,
    };

    Ok(crate::plugins::check(state.runtime.store(), id, dialed, &endpoint, account).await?)
}

/// Chooses which of a crew's agents may call one plugin.
///
/// The whole answer, every time: `everyone`, or the complete list of agents.
/// A merge would let a panel narrow a plugin by forgetting somebody, and this
/// one renders what it last read.
pub async fn set_plugin_access(
    state: &AppState,
    id: PluginId,
    access: PluginAccess,
) -> Reply<Plugin> {
    let plugin = state.runtime.store().set_plugin_access(id, &access)?;
    // Changes what every agent in the crew is offered on its next turn, and
    // what the roster says its peers can reach.
    state.runtime.emit(UiEvent::AgentsChanged);
    Ok(plugin)
}

/// Chooses which of a crew's agents may call one of a plugin's tools.
///
/// One tool per call, and the whole answer for it rather than a toggle, so that
/// two panels open on the same group cannot swap a decision between them. The
/// plugin comes back so the caller draws what was stored rather than what it
/// asked for.
pub async fn set_plugin_tool(
    state: &AppState,
    id: PluginId,
    tool: String,
    access: PluginAccess,
) -> Reply<Plugin> {
    let plugin = state.runtime.store().set_plugin_tool(id, &tool, &access)?;
    // Changes the tool definitions every agent in the crew is offered on its
    // next turn, and the line in each of their prompts that says what is off.
    state.runtime.emit(UiEvent::AgentsChanged);
    Ok(plugin)
}

/// Forgets a plugin, and the grant with it.
///
/// The grant is dropped locally rather than revoked at the vendor: not every
/// authorization server publishes a revocation endpoint, and an operator who
/// wants the authorization itself withdrawn has to do that where they granted
/// it. Said in the UI rather than assumed.
pub async fn disconnect_plugin(state: &AppState, id: PluginId) -> Reply<()> {
    state.runtime.store().delete_plugin(id)?;
    state.runtime.emit(UiEvent::AgentsChanged);
    Ok(())
}

/// What one agent's browser turns out to be signed in to, right now.
///
/// Detected rather than declared: the browser is holding the cookies, so Guaca
/// asks the machine instead of asking the operator to keep a list up to date.
/// Called when the operator opens an agent, and by the runtime after an agent
/// has been browsing, which between them covers both ways a session appears.
pub async fn scan_agent_signins(state: &AppState, id: AgentId) -> Reply<Vec<Signin>> {
    Ok(state.runtime.scan_signins(id).await?)
}

/// The last scan's result, without touching the machine.
///
/// Separate from the scan because the machine may be asleep, and what it was
/// signed in to yesterday is still the best answer available. Waking a sandbox
/// to redraw a list would also cost money on every render.
pub async fn agent_signins(state: &AppState, id: AgentId) -> Reply<Vec<Signin>> {
    Ok(state.runtime.store().agent_signins(id)?)
}

// ---- permission requests -------------------------------------------------

/// The most requests the desk is handed at once.
///
/// Well above what a workspace can have parked at one time, since a turn parks
/// on one request and an agent runs one turn: this is a bound on a query, not a
/// policy about how many the operator is shown.
const MAX_PENDING: u32 = 200;

/// What every recent request came to, keyed by id.
///
/// The requests themselves travel in the transcript, so this is only the half
/// that changes: whether the buttons on a request already in a channel are
/// still live, and what was decided if they are not.
pub async fn approval_states(state: &AppState) -> Reply<HashMap<ApprovalId, ApprovalState>> {
    Ok(state.runtime.store().approval_states(500)?)
}

/// Every request still waiting on the operator, oldest first.
///
/// Whole rather than as ids, because the caller is the desk and what it needs
/// is the wording: a queue that says only that something is pending is a queue
/// that cannot be answered without going and finding each channel, which is the
/// walk it exists to save. Same read the menu bar makes, and it is a read rather
/// than an accumulation for the same reason: a list assembled from events drifts
/// the moment one is missed, and what drifts is the count the operator is using
/// to decide whether anyone is waiting.
pub async fn pending_approvals(state: &AppState) -> Reply<Vec<Approval>> {
    Ok(state.runtime.store().pending_approvals(MAX_PENDING)?)
}

/// What this agent no longer has to ask about.
pub async fn agent_grants(state: &AppState, id: AgentId) -> Reply<Vec<ProtectedAction>> {
    Ok(state.runtime.store().standing_grants(id)?)
}

/// Takes one back. A permission that could only ever be given is a trap, and
/// "always" has to stay something the operator can change their mind about.
pub async fn revoke_grant(
    state: &AppState,
    id: AgentId,
    action: ProtectedAction,
) -> Reply<Vec<ProtectedAction>> {
    state.runtime.store().revoke_grant(id, action)?;
    Ok(state.runtime.store().standing_grants(id)?)
}

/// Answers one. Refused if it was already answered or has expired, which is
/// what the operator's second click on a stale widget is.
pub async fn decide_approval(
    state: &AppState,
    id: ApprovalId,
    decision: Decision,
) -> Reply<Approval> {
    Ok(state.runtime.decide_approval(id, decision)?)
}

/// Answers a question with what the operator picked or wrote.
///
/// Its own command rather than a fourth `Decision`, for the reason on that
/// enum: three tokens and arbitrary text are different things on a wire, and
/// only one of them can come from a menu item.
pub async fn answer_question(state: &AppState, id: ApprovalId, answer: String) -> Reply<Approval> {
    Ok(state.runtime.answer_question(id, &answer)?)
}

/// Everything an agent has said it cannot get past without the operator.
///
/// The other half of the desk's queue, read the same way and for the same
/// reason: what is on it is whatever the store says is open, so nothing can
/// appear there that is not a row somewhere.
///
/// Unbounded in practice by the same argument `MAX_PENDING` rests on, one step
/// stronger: an agent holds at most one open escalation, so this is bounded by
/// the size of the crew.
pub async fn open_escalations(state: &AppState) -> Reply<Vec<Escalation>> {
    Ok(state.runtime.store().open_escalations(MAX_PENDING)?)
}

/// Takes one off the desk. Not an answer: nothing is waiting on it.
pub async fn clear_escalation(state: &AppState, id: EscalationId) -> Reply<()> {
    state.runtime.clear_escalation(id)?;
    Ok(())
}

// ---- the calendar --------------------------------------------------------

/// How many occasions one read of the calendar hands back.
///
/// The view asks for a window of days, so this is a ceiling on a pathological
/// crew rather than a page size: a year of a busy workspace is a few hundred
/// rows, and the panel is a list the operator scrolls. Past it, what is cut is
/// the far end of the window, which is the part they were not looking at.
const MAX_OCCASIONS: u32 = 500;

/// What is on the calendar between two moments.
///
/// `groupId` absent is every crew at once, which is the operator's default and
/// the one read in the app that deliberately crosses the wall between crews. It
/// is theirs to cross: the wall stands between crews so an agent cannot move
/// another crew's meeting, not between the operator and a workspace they own.
pub fn calendar(
    state: &AppState,
    from: i64,
    until: i64,
    group_id: Option<GroupId>,
) -> Reply<Vec<Occasion>> {
    Ok(state.runtime.store().occasions(from, until, group_id, MAX_OCCASIONS)?)
}

/// What the operator can set on an occasion.
///
/// Deliberately the same shape the agent's own `calendar` tool takes, so one
/// written by hand and one written by an agent are the same row. The date
/// arrives as the string that was typed and is parsed here, which is what lets
/// the panel accept `2026-09-14` for a whole day and `2026-09-14 15:00` for a
/// time without a second field saying which it meant.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OccasionDraft {
    pub group_id: GroupId,
    pub title: String,
    #[serde(default)]
    pub detail: String,
    #[serde(default)]
    pub place: String,
    pub starts_at: String,
    pub minutes: Option<u32>,
}

impl OccasionDraft {
    /// The operator's own writes carry no agent, which is what `agentId: null`
    /// means everywhere it is read back.
    fn checked(&self) -> Result<occasion::Clean, CommandError> {
        let when = occasion::parse_when(&self.starts_at)
            .map_err(|e| CommandError::new("validation", e.to_string()))?;
        occasion::Clean::new(
            self.group_id,
            None,
            &self.title,
            &self.detail,
            &self.place,
            when,
            self.minutes,
        )
        .map_err(|e| CommandError::new("validation", e.to_string()))
    }
}

pub fn create_occasion(state: &AppState, draft: OccasionDraft) -> Reply<Occasion> {
    let clean = draft.checked()?;
    let occasion = state.runtime.store().create_occasion(&clean)?;
    // The crew's agents read this at the top of every turn, and the panel may
    // be open on another window of the same calendar.
    state.runtime.emit(UiEvent::CalendarChanged { group_id: occasion.group_id });
    Ok(occasion)
}

/// Rewrites one. The crew it is on is taken from the row rather than from the
/// draft: moving an occasion between crews is not an edit, and there is no
/// surface that offers it.
pub fn update_occasion(
    state: &AppState,
    id: OccasionId,
    draft: OccasionDraft,
) -> Reply<Occasion> {
    let existing = state
        .runtime
        .store()
        .any_occasion(id)?
        .ok_or_else(|| CommandError::new("notFound", format!("no occasion with id {id}")))?;

    let clean = OccasionDraft { group_id: existing.group_id, ..draft }.checked()?;
    let occasion = state
        .runtime
        .store()
        .update_occasion(id, existing.group_id, &clean)?
        .ok_or_else(|| CommandError::new("notFound", format!("no occasion with id {id}")))?;
    state.runtime.emit(UiEvent::CalendarChanged { group_id: occasion.group_id });
    Ok(occasion)
}

pub fn delete_occasion(state: &AppState, id: OccasionId) -> Reply<()> {
    // Read before the delete, because the event names the crew whose calendar
    // moved and afterward there is nothing left to ask.
    let Some(existing) = state.runtime.store().any_occasion(id)? else {
        return Ok(());
    };
    state.runtime.store().delete_occasion(id, existing.group_id)?;
    state.runtime.emit(UiEvent::CalendarChanged { group_id: existing.group_id });
    Ok(())
}

// ---- groups --------------------------------------------------------------

pub async fn list_groups(state: &AppState) -> Reply<Vec<Group>> {
    Ok(state.runtime.store().list_groups()?)
}

pub async fn create_group(state: &AppState, draft: GroupDraft) -> Reply<Group> {
    honorable(
        state.deployment.capabilities(),
        draft.inference.as_ref().and_then(|i| i.provider),
        draft.inference.as_ref().and_then(|i| i.base_url.as_deref()),
    )?;
    let clean = draft.validate()?;
    let group = state.runtime.store().create_group(&clean)?;
    state.runtime.emit(UiEvent::AgentsChanged);
    Ok(group)
}

pub async fn update_group(state: &AppState, id: GroupId, draft: GroupDraft) -> Reply<Group> {
    honorable(
        state.deployment.capabilities(),
        draft.inference.as_ref().and_then(|i| i.provider),
        draft.inference.as_ref().and_then(|i| i.base_url.as_deref()),
    )?;
    let clean = draft.validate()?;
    let group = state.runtime.store().update_group(id, &clean)?;
    state.runtime.emit(UiEvent::AgentsChanged);
    Ok(group)
}

/// Verifies a group's endpoint and key without involving one of its agents.
///
/// The group's own answer to `test_connection`, and it has to be a separate
/// command rather than that one with an id: a group's settings are layered over
/// the app's, so what is worth testing is the resolution rather than either
/// half. Takes what is on screen, exactly as the app's does, and starts from the
/// stored key so a test run without retyping it tests the key that is actually
/// there. `id` is absent for a group that has not been created yet.
pub async fn test_group_connection(
    state: &AppState,
    id: Option<GroupId>,
    draft: GroupDraft,
) -> Reply<String> {
    let clean = draft.validate()?;
    let mut resolved = match id {
        Some(id) => state.runtime.store().group_inference(id)?,
        None => GroupInference::default(),
    };
    if let Some(overrides) = clean.inference {
        resolved.overrides = overrides;
    }
    if let Some(key) = clean.api_key {
        resolved.api_key = key;
    }

    let mut config = state.runtime.config();
    config.inference = resolved.apply(&config.inference);
    state
        .runtime
        .probe(&config)
        .await
        .map_err(|err| CommandError::new("inference", err.to_string()))
}

/// Deletes an empty group. Refused while it still holds agents; see
/// `Store::delete_group` for why they are not relocated.
pub async fn delete_group(state: &AppState, id: GroupId) -> Reply<()> {
    state.runtime.store().delete_group(id)?;
    state.runtime.emit(UiEvent::AgentsChanged);
    Ok(())
}

/// Deletes a group and the crew inside it.
///
/// The other half of `delete_group`, which refuses while anyone is still in
/// there. An operator winding a crew down had to delete each agent by hand and
/// then the group, and stopping halfway left a group with three agents in it
/// and no reason to keep any of them.
///
/// Every agent goes the whole way, and none of them lands in the compost: its
/// computer killed, its browser and profile destroyed, its memory, schedule,
/// sign-ins and standing permission gone. What each of them said stays
/// readable, because a disband is a delete at the scale of a crew and not a
/// different rule about history.
///
/// The compost is a hold on one agent, and a disband takes the place it would
/// come back to. `delete_group` files whoever is left under the default group,
/// so a composted crew would be offered a restore into a crew that no longer
/// exists, holding a sign-in belonging to a group whose credentials went with
/// it. The confirmation names the count for that reason: this is the
/// irreversible one, and it says so.
///
/// Refused before anything irreversible. `group_for_removal` asks the one
/// question that does not depend on the crew: killing four computers and then
/// discovering the group itself cannot go would leave the operator with neither
/// the crew nor the deletion.
///
/// Past that point a failure stops where it is and is reported. Retrying is
/// safe and picks up where it left off, because the crew is read fresh and the
/// agents already retired are no longer in it.
pub async fn disband_group(state: &AppState, id: GroupId) -> Reply<()> {
    state.runtime.store().group_for_removal(id)?;

    let outcome = async {
        for card in state.runtime.store().group_crew(id)? {
            state.runtime.purge_agent(&card).await?;
        }
        state.runtime.store().delete_group(id)?;
        Ok(())
    }
    .await;

    // Emitted either way. A disband that stopped part-way has still deleted
    // whoever it reached, and a rail left drawing them is rows the operator can
    // click on to open channels belonging to agents that are gone.
    state.runtime.emit(UiEvent::AgentsChanged);
    outcome
}

// ---- agents --------------------------------------------------------------

pub async fn list_agents(state: &AppState) -> Reply<Vec<AgentCard>> {
    Ok(state.runtime.store().list_agents()?)
}

pub async fn create_agent(state: &AppState, draft: AgentDraft) -> Reply<AgentCard> {
    let clean = draft.validate()?;
    let card = state.runtime.store().create_agent(&clean)?;
    state.runtime.start_agent(card.id);
    state.runtime.emit(UiEvent::AgentsChanged);
    Ok(card)
}

pub async fn update_agent(state: &AppState, id: AgentId, draft: AgentDraft) -> Reply<AgentCard> {
    let clean = draft.validate()?;
    let card = state.runtime.store().update_agent(id, &clean)?;
    state.runtime.emit(UiEvent::AgentsChanged);
    Ok(card)
}

/// Deletes an agent, into the compost.
///
/// It leaves the rail and the directory immediately and can never be messaged
/// again, and what it already said stays readable in the other agents'
/// channels: hard-deleting would punch holes in transcripts that had nothing to
/// do with this agent.
///
/// What is new is that nothing it held privately is destroyed yet. For thirty
/// days its memory, working notes, schedule, sign-ins and permissions sit
/// exactly where they were and the operator can pull it back out;
/// `Runtime::sweep_compost` is what eventually spends the other half.
pub async fn delete_agent(state: &AppState, id: AgentId) -> Reply<()> {
    let card = agent_card(state, id)?;
    state.runtime.discard_agent(&card).await?;
    state.runtime.emit(UiEvent::AgentsChanged);
    Ok(())
}

/// Pulls an agent back out of the compost, paused.
///
/// Refused for an agent whose thirty days are up, which is the same refusal as
/// for one nobody deleted: both are rows with no stamp on them, and neither has
/// anything left to bring back.
pub async fn restore_agent(state: &AppState, id: AgentId) -> Reply<AgentCard> {
    let card = agent_card(state, id)?;
    let restored = state.runtime.restore_agent(&card)?;
    state.runtime.emit(UiEvent::AgentsChanged);
    Ok(restored)
}

/// Ends the wait early: the thirty days, now.
///
/// The same act the sweep performs, on the operator's own timing, and the only
/// thing in this app that destroys an agent's memory on a click. What it leaves
/// is what the sweep leaves: a terminated row with no stamp, whose transcript
/// still reads.
pub async fn purge_agent(state: &AppState, id: AgentId) -> Reply<()> {
    let card = agent_card(state, id)?;
    state.runtime.purge_agent(&card).await?;
    state.runtime.emit(UiEvent::AgentsChanged);
    Ok(())
}

pub async fn set_agent_paused(state: &AppState, id: AgentId, paused: bool) -> Reply<AgentCard> {
    let target = if paused { Lifecycle::Paused } else { Lifecycle::Active };
    let card = state.runtime.store().set_lifecycle(id, target)?;
    if paused {
        state.runtime.pause_agent(id);
    } else {
        state.runtime.start_agent(card.id);
        state.runtime.resume_agent(id);
    }
    state.runtime.emit(UiEvent::AgentsChanged);
    Ok(card)
}

/// Keeps an agent at the top of the rail.
///
/// Nothing about the agent changes: it is discoverable, addressable and billed
/// exactly as before, and no peer is told. The card version deliberately does
/// not move, because nothing a peer reads has.
pub async fn set_agent_pinned(state: &AppState, id: AgentId, pinned: bool) -> Reply<AgentCard> {
    let card = state.runtime.store().set_agent_pinned(id, pinned)?;
    state.runtime.emit(UiEvent::AgentsChanged);
    Ok(card)
}

/// Puts an agent where the operator dropped it: which group, and which place.
///
/// One call rather than two, because a drag is one gesture that can be both,
/// and two writes leave a state where the agent has arrived in the group but
/// not in the place it was dropped.
///
/// `before` is the row it lands in front of; `None` is the end of the group.
/// Nothing about the agent itself changes, so the card version does not move
/// and no peer is told: an agent's group is enforced on every turn from a fresh
/// read, so it is in its new crew's directory from the next message onward.
pub async fn move_agent(
    state: &AppState,
    id: AgentId,
    group_id: GroupId,
    before: Option<AgentId>,
) -> Reply<AgentCard> {
    let card = state.runtime.store().move_agent(id, group_id, before)?;
    state.runtime.emit(UiEvent::AgentsChanged);
    Ok(card)
}

/// Makes a second agent from the same card.
///
/// The card and nothing else: a copy starts with the look, the model, the
/// skills and the instructions, and with no computer, no memory, no schedule,
/// no accounts and no transcript. Those are not part of what the operator
/// wrote, they are what one agent went and did, and a second agent that
/// inherited a sandbox would be two agents holding one machine.
pub async fn duplicate_agent(state: &AppState, id: AgentId) -> Reply<AgentCard> {
    let original = agent_card(state, id)?;
    // Only the group it is being copied into, and only agents that still hold
    // their name: a terminated agent frees it.
    let taken: Vec<String> = state
        .runtime
        .store()
        .list_agents()?
        .into_iter()
        .filter(|c| c.group_id == original.group_id && c.lifecycle != Lifecycle::Terminated)
        .map(|c| c.name)
        .collect();

    let draft = AgentDraft {
        group_id: Some(original.group_id),
        name: copy_name(&original.name, &taken),
        avatar: original.avatar,
        color: original.color,
        model: original.model,
        system_prompt: original.system_prompt,
        skills: original.skills,
    };
    let card = state.runtime.store().create_agent(&draft.validate()?)?;
    state.runtime.start_agent(card.id);
    state.runtime.emit(UiEvent::AgentsChanged);
    Ok(card)
}

/// Hires a set of preconfigured agents into one group.
///
/// One command rather than one `create_agent` per agent, and the reason is not
/// only round trips. Every create emits `AgentsChanged`, and the rail answers
/// each one by re-reading the whole roster, so a crew of six hired one at a
/// time redraws the sidebar six times while it is being filled in. Worse, a
/// draft rejected halfway leaves the operator with three agents and an error
/// about a fourth, and nothing that says which three.
///
/// So the batch is resolved and validated in full before anything is written:
/// the group has to exist, every name is settled against the roster and against
/// the rest of the batch by `hire_names`, and every draft has to pass
/// `validate`. The rows then go in as one transaction, because a name taken by
/// another window between the check and the write is a failure validation
/// cannot see. A hire either happens or does not.
///
/// `group_id` is required and overrides whatever the drafts carry. The whole
/// point of the cafeteria is that a crew lands somewhere the operator chose,
/// and a batch that could scatter across groups is a batch that can arrive
/// half outside the wall its agents were picked to sit behind.
pub async fn hire_agents(
    state: &AppState,
    group_id: GroupId,
    drafts: Vec<AgentDraft>,
) -> Reply<Vec<AgentCard>> {
    if drafts.is_empty() {
        return Ok(Vec::new());
    }

    let store = state.runtime.store();
    // Checked rather than left to the foreign key, which would surface as an
    // opaque storage failure on a group the operator picked from a list. The
    // realistic way to get here is a group deleted while the cafeteria was open.
    if !store.list_groups()?.iter().any(|g| g.id == group_id) {
        return Err(CommandError::new(
            "notFound",
            "that group no longer exists. Close the cafeteria and pick another one.",
        ));
    }

    // Only the group being hired into, and only agents that still hold their
    // name: a terminated agent frees it. Same rule `duplicate_agent` uses.
    let taken: Vec<String> = store
        .list_agents()?
        .into_iter()
        .filter(|c| c.group_id == group_id && c.lifecycle != Lifecycle::Terminated)
        .map(|c| c.name)
        .collect();

    let wanted: Vec<String> = drafts.iter().map(|d| d.name.clone()).collect();
    let clean = hire_names(&wanted, &taken)
        .into_iter()
        .zip(drafts)
        .map(|(name, draft)| AgentDraft { group_id: Some(group_id), name, ..draft }.validate())
        .collect::<Result<Vec<_>, _>>()?;

    let hired = store.create_agents(&clean)?;
    // Actors start only once the rows are committed. Starting them inside the
    // write would leave a live agent behind for a hire that rolled back.
    for card in &hired {
        state.runtime.start_agent(card.id);
    }
    state.runtime.emit(UiEvent::AgentsChanged);
    Ok(hired)
}

pub async fn agent_activity(state: &AppState) -> Reply<HashMap<AgentId, Activity>> {
    Ok(state.runtime.activity_snapshot())
}

/// Newest message timestamp per agent, used to order the sidebar by who spoke
/// most recently. Live updates come from message events; this seeds them.
/// An agent's memory: a small markdown file it maintains for itself.
pub async fn agent_memory(state: &AppState, id: AgentId) -> Reply<String> {
    Ok(state.runtime.workspace().read(id))
}

/// Lets the operator seed or correct an agent's memory by hand.
pub async fn set_agent_memory(state: &AppState, id: AgentId, content: String) -> Reply<String> {
    let card = state
        .runtime
        .store()
        .get_agent(id)?
        .ok_or_else(|| CommandError::new("notFound", format!("no agent with id {id}")))?;
    state
        .runtime
        .workspace()
        .write(id, &card.name, &content)
        .map_err(|err| CommandError::new("storage", err.to_string()))?;
    Ok(state.runtime.workspace().read(id))
}

/// What an agent is in the middle of: the other half of what it carries.
pub async fn agent_working_notes(state: &AppState, id: AgentId) -> Reply<Vec<WorkingNote>> {
    Ok(state.runtime.store().working_notes(id)?)
}

/// Drops every note an agent holds.
///
/// The operator's only write here, and deliberately the blunt one. Editing a
/// single note would make the list a document two parties maintain, which is
/// the shape memory already has and the reason it needed the held-draft dance
/// in the panel. This list is the agent's own account of its work; the operator
/// either believes it or says the work is done.
pub async fn clear_agent_working_notes(state: &AppState, id: AgentId) -> Reply<()> {
    state.runtime.store().clear_working_notes(id)?;
    Ok(())
}

pub async fn agent_last_active(state: &AppState) -> Reply<HashMap<AgentId, i64>> {
    Ok(state.runtime.store().last_activity()?)
}

// ---- messages ------------------------------------------------------------

/// The most of a transcript any one read will put in the webview.
const MAX_CHANNEL_WINDOW: u32 = 1000;

/// A channel's newest messages.
///
/// `through` widens the window to reach one particular message, which is what
/// opening a search result needs: a hit from last month is not in the newest
/// three hundred, and a jump that lands somewhere else is a jump that failed.
/// A message that has since been cleared falls back to the plain newest window
/// rather than to an empty channel.
pub async fn channel_messages(
    state: &AppState,
    channel_id: AgentId,
    limit: Option<u32>,
    through: Option<MessageId>,
) -> Reply<Vec<Envelope>> {
    let store = state.runtime.store();
    if let Some(id) = through {
        let reaching = store.channel_messages_through(channel_id, id, MAX_CHANNEL_WINDOW)?;
        if !reaching.is_empty() {
            return Ok(reaching);
        }
    }
    Ok(store.channel_messages(channel_id, limit.unwrap_or(300).min(MAX_CHANNEL_WINDOW))?)
}

/// What two agents said to each other, for the thread opened off a channel.
///
/// Neither agent's channel holds it: a send is filed under the recipient and
/// the answer under the sender, so this is read from the messages themselves.
pub async fn pair_messages(
    state: &AppState,
    a: AgentId,
    b: AgentId,
    limit: Option<u32>,
) -> Reply<Vec<Envelope>> {
    Ok(state.runtime.store().pair_messages(a, b, limit.unwrap_or(200).min(1000))?)
}

/// One crew's conversation, for the flow board in its settings.
pub async fn conversation_flow(
    state: &AppState,
    group: GroupId,
    limit: Option<u32>,
) -> Reply<Vec<Envelope>> {
    Ok(state.runtime.store().conversation_flow(group, limit.unwrap_or(400).min(2000))?)
}

/// What the transcript has to say about a query.
///
/// Agents and groups are deliberately not here. The webview is already holding
/// both to draw the rail, so matching them there costs no round trip and stays
/// right while the operator types. What it is not holding is the transcript,
/// and shipping that across IPC to search it in the renderer would copy the
/// database once per keystroke.
pub async fn search(state: &AppState, query: String, limit: Option<u32>) -> Reply<SearchHits> {
    Ok(state.runtime.store().search(query.trim(), limit.unwrap_or(20).min(100))?)
}

/// What became of a drop.
#[derive(Debug, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Staged {
    pub attached: Vec<Attachment>,
    /// One line per file that could not be taken, saying which and why.
    pub refused: Vec<String>,
}

/// Takes what the operator dropped on the window into the store, there and then.
///
/// On the drop rather than on the send, for two reasons they feel. A file too
/// big to send is refused while they are still holding it, instead of failing a
/// message they have since written; and a picture that is already stored has an
/// address, so it can be shown back to them before it goes. What is staged and
/// never sent is the same leftover as a file whose message was deleted, and the
/// store has always kept those.
///
/// `paths` are on the operator's own disk, never bytes: this side reads them,
/// so a document never crosses IPC and never sits in the renderer's memory.
pub async fn stage_files(state: &AppState, paths: Vec<String>) -> Reply<Staged> {
    state.deployment.capabilities().require(Absent::LocalFiles)?;

    let mut staged = Staged::default();
    for path in &paths {
        match state.runtime.files().take(std::path::Path::new(path)) {
            Ok(file) => staged.attached.push(file),
            // One file out of five failing does not refuse the other four. The
            // operator picked all of them deliberately, and the one that cannot
            // go is named so they know which it was.
            Err(err) => staged.refused.push(err.to_string()),
        }
    }
    Ok(staged)
}

/// A ticket for one sandbox's screen, or `None` where the screen needs none.
///
/// `sha256(secret ":" sandbox)`, so it can be checked without being stored and
/// leaks nothing but the right to watch that one screen through this daemon.
/// The same function checks it at the route.
pub fn screen_ticket(secret: &str, sandbox: &str) -> String {
    use sha2::Digest;
    format!("{:x}", sha2::Sha256::digest(format!("{secret}:{sandbox}").as_bytes()))
}

/// The route a hosted page reaches a computer's screen on. One string, read
/// by the address below and by the daemon's router.
pub const SCREEN_ROUTE: &str = "/v1/screen";

impl AppState {
    /// Points a computer's screen at the daemon rather than at loopback, on a
    /// host that has one.
    ///
    /// The viewer stays on loopback and keeps the sandbox tokens; the daemon
    /// relays to it. The address is relative, because only the page knows
    /// which origin it reached the daemon at, and the page resolves it.
    fn screened(&self, mut computer: crate::e2b::Computer) -> crate::e2b::Computer {
        if let (Some(secret), Some(url)) = (&self.secret, computer.vnc_url.as_deref()) {
            let viewer =
                format!("http://{}:{}", crate::e2b::VIEWER_HOST, self.runtime.viewer_port());
            if let Some(rest) = url.strip_prefix(&viewer) {
                let ticket = screen_ticket(secret, &computer.sandbox_id);
                computer.vnc_url = Some(format!("{SCREEN_ROUTE}/{ticket}{rest}"));
            }
        }
        computer
    }
}

/// Sends files from this machine's disk to a workspace somewhere else.
///
/// The desktop app can show a box's workspace, and a file dropped on that
/// window is still a path on this disk that the box has never seen. So the
/// same read `stage_files` does happens here, and the bytes go to the box's
/// upload route instead of into this machine's store. One file at a time,
/// with the box's own sentence for each one it refuses.
pub async fn forward_files(
    state: &AppState,
    origin: String,
    token: String,
    paths: Vec<String>,
) -> Reply<Staged> {
    state.deployment.capabilities().require(Absent::LocalFiles)?;

    let http = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(120))
        .build()
        .map_err(|err| CommandError::new("file", err.to_string()))?;
    let origin = origin.trim_end_matches('/');
    let mut staged = Staged::default();
    for path in &paths {
        let path = std::path::Path::new(path);
        let name = path.file_name().and_then(|n| n.to_str()).unwrap_or_default().to_string();
        let bytes = match std::fs::read(path) {
            Ok(bytes) => bytes,
            Err(err) => {
                staged.refused.push(format!("{}: {err}", path.display()));
                continue;
            }
        };
        let sent = http
            .post(format!("{origin}/v1/upload?name={}", crate::oauth::encode(&name)))
            .bearer_auth(&token)
            .body(bytes)
            .send()
            .await;
        let answered: serde_json::Value = match sent {
            Ok(response) => response.json().await.unwrap_or_default(),
            Err(err) => {
                staged.refused.push(format!("{name}: could not reach the workspace ({err})"));
                continue;
            }
        };
        if let Some(refused) = answered.get("err").and_then(|e| e.get("message")) {
            staged.refused.push(format!("{name}: {}", refused.as_str().unwrap_or("refused")));
            continue;
        }
        match serde_json::from_value::<Attachment>(answered["ok"].clone()) {
            Ok(file) => staged.attached.push(file),
            Err(_) => staged
                .refused
                .push(format!("{name}: the workspace answered nothing this app could read")),
        }
    }
    Ok(staged)
}

/// Hands the menu bar what the window is looking at, when that is not here.
///
/// The strip follows the window. A window showing a box's workspace already
/// holds that workspace's roster, activity, requests and spend, and this is
/// the one way those numbers reach the corner of the screen; `None` puts the
/// strip back on this machine's own runtime, and is what a window sends the
/// moment it is showing that again.
pub async fn report_presence(
    state: &AppState,
    presence: Option<crate::menubar::Presence>,
) -> Reply<()> {
    (state.menubar)(presence);
    Ok(())
}

/// Stops every conversation in this workspace. Says how many were running.
///
/// The menu bar's own row does this against the local runtime directly; this
/// is the same act for a window that is showing a box, which has to ask the
/// box.
pub async fn stop_everything(state: &AppState) -> Reply<usize> {
    Ok(state.runtime.stop_everything())
}

/// A file the operator has already dropped, as the webview refers to it.
///
/// Two fields and no more: which bytes, and what to call them. Everything else
/// about an attachment is worked out on this side.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FileRef {
    pub digest: String,
    pub name: String,
}

/// Sends the operator's message, with anything they attached to it.
///
/// The files are already in the store by now: `stage_files` put them there when
/// they were dropped. This resolves each one again rather than trusting what
/// came back over IPC, so a message carries a file the operator actually has.
pub async fn send_message(
    state: &AppState,
    agent_id: AgentId,
    text: String,
    files: Option<Vec<FileRef>>,
) -> Reply<RunId> {
    let trimmed = text.trim();
    let files = files.unwrap_or_default();
    // A file on its own is a message. "Here, read this" with the document
    // attached is the most natural way to send one, and rejecting it as empty
    // would be the app arguing with the operator.
    if trimmed.is_empty() && files.is_empty() {
        return Err(CommandError::new("validation", "message must not be empty"));
    }

    let mut attached = Vec::new();
    for file in &files {
        match state.runtime.files().reference(&file.digest, &file.name) {
            Ok(file) => attached.push(file),
            // Named, because the operator attached this file deliberately and a
            // message that quietly went without it is worse than an error.
            Err(err) => return Err(CommandError::new("file", err.to_string())),
        }
    }
    Ok(state.runtime.send_from_human_with(agent_id, trimmed, attached)?)
}

/// Puts a copy of a stored file where a person can get at it, and says where.
///
/// The downloads folder, not a save dialog: the operator asked for the file,
/// not for a conversation about where to put it. The path goes back so the app
/// can say where to look, since a copy that lands somewhere unannounced is a
/// copy they have to go and find.
pub async fn save_file(state: &AppState, digest: String, name: String) -> Reply<String> {
    state.deployment.capabilities().require(Absent::LocalFiles)?;

    let saved = state
        .runtime
        .files()
        .save_copy(&digest, &name, &state.downloads)
        .map_err(|err| CommandError::new("file", err.to_string()))?;
    Ok(saved.display().to_string())
}

/// Where a page an agent wrote can be framed.
///
/// The renderer hands over the document and gets back an address on the
/// artifact server, which is a loopback origin of its own. It has to be a round
/// trip rather than a `srcdoc`: a frame given its markup inline inherits the
/// app's own content policy, and the app's policy forbids script, so the page
/// would draw and quietly do nothing. What the page is then allowed to do is
/// `artifact.rs`'s argument, and none of it is negotiable from here.
///
/// The document is not persisted and this is not a store. The message that
/// carried it is the record; this is a copy held while a transcript is drawing
/// one, and an id that has been evicted is registered again by the next draw.
pub async fn frame_artifact(state: &AppState, html: String) -> Reply<ArtifactAddress> {
    let id = state
        .artifacts
        .keep(&html)
        .map_err(|err| CommandError::new("artifact", err.to_string()))?;
    Ok(ArtifactAddress { port: state.artifact_port.load(std::sync::atomic::Ordering::SeqCst), id })
}

/// Where the renderer should point a frame.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ArtifactAddress {
    pub port: u16,
    pub id: String,
}

/// Sends a failed turn's message again, as a new run.
///
/// Offered on the notice the failure left behind, so the operator retries the
/// thing that broke rather than retyping what they asked for.
pub async fn retry_turn(
    state: &AppState,
    agent_id: AgentId,
    message_id: MessageId,
) -> Reply<RunId> {
    Ok(state.runtime.retry_turn(agent_id, message_id)?)
}

/// Stops a conversation and everything it set off.
///
/// A run rather than an agent: what the operator wants to end reached however
/// many agents it reached, and stopping only the one they happen to be looking
/// at would leave the rest of it running on their bill.
///
/// False when the run had already finished, which is the ordinary outcome of a
/// stop that arrives a moment too late. It is not an error and there is nothing
/// for the operator to do about it: the answer they were waiting for is already
/// on screen.
pub async fn stop_run(state: &AppState, run_id: RunId) -> Reply<bool> {
    Ok(state.runtime.stop_run(run_id))
}

/// Empties one agent's channel, and touches nothing else it knows.
///
/// An agent carries two kinds of state and this is one of them. The channel is
/// what its turns read back as conversation, and it is where a crew's habits
/// accumulate: a coordinator that spent a day inventing assignment numbers has
/// a day of that in front of it on every turn, and it will keep going. Its
/// memory is the other, a file it wrote deliberately and would have to write
/// again, and it is a markdown file in the workspace folder that nothing here
/// reaches.
///
/// So this is the operator's way to give an agent a fresh start without taking
/// away what it learned, and the wording on the menu item says so, because an
/// operator who cannot tell the two apart does not use it.
///
/// One channel, not a conversation. A message this agent sent a peer is filed
/// in the *peer's* channel, so clearing here leaves what it told them where
/// they can still read it, and takes away only what this agent would read
/// back. That asymmetry is the point rather than a limitation: resetting one
/// confused agent must not erase a colleague's record of what it asked for.
pub async fn clear_channel(state: &AppState, channel_id: AgentId) -> Reply<usize> {
    Ok(state.runtime.store().delete_channel_messages(channel_id)?)
}

// ---- routines ------------------------------------------------------------

/// An agent's own schedule, as the operator sees it.
pub async fn agent_routines(state: &AppState, id: AgentId) -> Reply<Vec<Routine>> {
    Ok(state.runtime.store().agent_routines(id)?)
}

/// What an operator can set on a routine.
///
/// Deliberately the same shape the agent's own `schedule` tool takes, so a
/// routine an operator writes and one an agent writes are the same thing. An
/// absent `inSecs` on an edit leaves the next firing where it was: correcting
/// a typo should not move the schedule.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RoutineDraft {
    #[serde(default)]
    pub name: String,
    pub what: String,
    /// The stored trigger form: `daily`, `weekdays`, `every:3600`, `once`.
    pub trigger: String,
    pub in_secs: Option<u32>,
    /// Whether a firing that lands while the agent is working is dropped.
    ///
    /// Defaulted rather than required, because a routine that has to happen
    /// even if it has to wait is the ordinary case and the one an older caller
    /// means by saying nothing.
    #[serde(default)]
    pub skip_if_working: bool,
}

impl RoutineDraft {
    /// Refuses a trigger this build does not understand rather than storing it
    /// and finding out at the next tick. A row nothing can parse is a schedule
    /// that silently never fires.
    fn checked(&self) -> Result<Trigger, CommandError> {
        let trigger = Trigger::parse(&self.trigger).ok_or_else(|| {
            CommandError::new("validation", format!("no trigger called {:?}", self.trigger))
        })?;
        routine::validate(&self.name, &self.what, &trigger, self.in_secs, self.skip_if_working)
            .map_err(|e| CommandError::new("validation", e.to_string()))?;
        Ok(trigger)
    }
}

pub async fn create_routine(
    state: &AppState,
    agent_id: AgentId,
    draft: RoutineDraft,
) -> Reply<Routine> {
    let trigger = draft.checked()?;
    let first = trigger.first_run(now_ms(), draft.in_secs);
    let routine = state.runtime.store().create_routine(
        agent_id,
        &draft.name,
        &draft.what,
        trigger,
        first,
        draft.skip_if_working,
    )?;
    state.runtime.emit(UiEvent::RoutinesChanged { agent_id });
    Ok(routine)
}

pub async fn update_routine(
    state: &AppState,
    id: RoutineId,
    draft: RoutineDraft,
) -> Reply<Routine> {
    let trigger = draft.checked()?;

    let existing = state
        .runtime
        .store()
        .get_routine(id)?
        .ok_or_else(|| CommandError::new("notFound", format!("no routine with id {id}")))?;
    let next = routine::next_slot_for(&trigger, &existing, draft.in_secs);

    let routine = state.runtime.store().update_routine(
        id,
        &draft.name,
        &draft.what,
        trigger,
        next,
        draft.skip_if_working,
    )?;
    state.runtime.emit(UiEvent::RoutinesChanged { agent_id: routine.agent_id });
    Ok(routine)
}

/// Turns a routine off, or back on.
///
/// Not an edit to what it says, so it goes through its own command and leaves
/// the next firing where it was. A routine switched back on after its slot has
/// passed is overdue, and the scheduler fires an overdue slot once.
pub async fn set_routine_active(state: &AppState, id: RoutineId, active: bool) -> Reply<Routine> {
    let routine = state.runtime.store().set_routine_active(id, active)?;
    state.runtime.emit(UiEvent::RoutinesChanged { agent_id: routine.agent_id });
    Ok(routine)
}

/// Fires a routine now, leaving its schedule alone.
///
/// The same delivery the scheduler makes, so what comes back from the button
/// is what will happen on Tuesday. Works on a routine that is switched off:
/// trying one out before turning it on is the point.
pub async fn test_routine(state: &AppState, id: RoutineId) -> Reply<RunId> {
    let routine = state
        .runtime
        .store()
        .get_routine(id)?
        .ok_or_else(|| CommandError::new("notFound", format!("no routine with id {id}")))?;
    Ok(state.runtime.test_routine(&routine)?)
}

/// Where an event is posted to fire a routine, and the secret the post carries.
///
/// The one secret this surface hands to the webview, and the exception is
/// argued rather than slipped in. The API key is kept from the webview because
/// nothing on that side ever needs it: every call it pays for is made from
/// here. This secret is the opposite case. Its only use is to be copied out of
/// the app into whatever the operator wires to post here, and the routine's
/// panel is where they are standing when they need it. What it unlocks is the
/// same thing the Test run button does, on a loopback port, from a process
/// that could read the config file anyway.
///
/// A port of zero is a receiver that is not up, and the panel says so rather
/// than printing an address nothing answers.
pub fn webhook_address(state: &AppState) -> Reply<WebhookAddress> {
    Ok(WebhookAddress {
        port: state.runtime.webhook_port(),
        secret: state.runtime.config().webhook.secret,
    })
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WebhookAddress {
    pub port: u16,
    pub secret: String,
}

/// What a routine has done lately, newest first.
pub async fn routine_runs(state: &AppState, id: RoutineId) -> Reply<Vec<RoutineRun>> {
    // Enough to answer "is this thing working" without turning the panel into
    // a log viewer. The transcript is where a firing is actually read.
    Ok(state.runtime.store().routine_runs(id, 20)?)
}

pub async fn delete_routine(state: &AppState, id: RoutineId) -> Reply<()> {
    // Read before the delete, because the event names the agent whose schedule
    // changed and afterward there is nothing left to ask.
    let whose = state.runtime.store().get_routine(id)?.map(|routine| routine.agent_id);
    state.runtime.store().delete_routine(id)?;
    if let Some(agent_id) = whose {
        state.runtime.emit(UiEvent::RoutinesChanged { agent_id });
    }
    Ok(())
}

/// What each of the given runs cost.
///
/// Asked for by the activity view, which knows which runs it is drawing. The
/// alternative, joining usage onto every message, would send the same totals
/// back once per message in the run.
pub async fn usage_for_runs(state: &AppState, runs: Vec<RunId>) -> Reply<Vec<RunUsage>> {
    Ok(state
        .runtime
        .store()
        .usage_by_run(&runs)?
        .into_iter()
        .map(|(run_id, tokens)| RunUsage { run_id, tokens })
        .collect())
}

/// What every group has spent, ever.
///
/// Cheap enough to ask for on load and after a run settles: it is one grouped
/// sum over a local table. The live numbers between those points come from
/// events, so this is a correction rather than a poll.
pub async fn usage_summary(state: &AppState) -> Reply<Vec<GroupUsage>> {
    Ok(state
        .runtime
        .store()
        .usage_by_group()?
        .into_iter()
        .map(|(group_id, tokens)| GroupUsage { group_id, tokens })
        .collect())
}

/// Empties every channel in a group. The crew stays; what it said does not.
pub async fn clear_group(state: &AppState, group_id: GroupId) -> Reply<GroupReset> {
    let store = state.runtime.store();
    let agents = store.group_agent_ids(group_id)?;

    let reset = GroupReset {
        messages: store.delete_group_messages(group_id)?,
        routines: store.delete_group_routines(group_id)?,
        calls: store.delete_group_usage(group_id)?,
        // A memory is a file, not a row. An agent whose transcript, schedule
        // and spend are gone but which still opens tomorrow believing what it
        // wrote last week has not started fresh.
        notes: agents
            .iter()
            .filter(|id| !state.runtime.workspace().read(**id).trim().is_empty())
            .inspect(|id| state.runtime.workspace().remove(**id))
            .count(),
        // And the other store, for the same reason from the other end: what an
        // agent is in the middle of is the half that was true about a
        // conversation this reset just deleted.
        working_notes: store.delete_group_working_notes(group_id)?,
    };

    // Named so open channels can drop what they are holding. `AgentsChanged`
    // refetches the roster, which is not what went stale here: the operator
    // had to click away and back to see an empty transcript.
    state.runtime.emit(UiEvent::ChannelsCleared { agents });
    state.runtime.emit(UiEvent::AgentsChanged);
    Ok(reset)
}

/// What a reset actually took, so the operator is told rather than reassured.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GroupReset {
    pub messages: usize,
    pub routines: usize,
    /// Memories wiped. Named for the file each one is, which is `notes`.
    pub notes: usize,
    /// Working notes dropped, which is the other store and a row count.
    pub working_notes: usize,
    pub calls: usize,
}

// ---- settings ------------------------------------------------------------

/// Absent fields are left alone. `apiKey: ""` clears the key; omitting it
/// keeps the existing one, which is what lets the UI show a redacted value
/// without ever round-tripping the secret.
#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct SettingsPatch {
    pub operator_name: Option<String>,
    pub provider: Option<config::Provider>,
    pub base_url: Option<String>,
    pub api_key: Option<String>,
    pub default_model: Option<String>,
    pub subscription_model: Option<String>,
    pub request_timeout_secs: Option<u64>,
    pub limits: Option<GuardLimits>,
    pub e2b_api_key: Option<String>,
    pub computer_idle_minutes: Option<u32>,
    pub kernel_api_key: Option<String>,
    pub browser_idle_minutes: Option<u32>,
    pub browser_stealth: Option<bool>,
}

/// What this workspace can do, which is a property of where it runs.
///
/// Read once when the window opens and never again: a deployment cannot change
/// under a running app, so this is a constant that happens to be delivered over
/// IPC. Every panel that could offer something impossible reads it, and the
/// refusals in the commands themselves are the second half of the same rule:
/// a model names tools it was never offered, and a webview on a stale bundle
/// calls commands its panels no longer draw.
pub async fn capabilities(state: &AppState) -> Reply<Capabilities> {
    Ok(state.deployment.capabilities())
}

pub async fn get_settings(state: &AppState) -> Reply<RedactedConfig> {
    Ok(state.runtime.config().redacted())
}

// ---- the Guaca account ---------------------------------------------------

/// Whether an account is signed in, and which service it is.
///
/// The origin is on it because in development it is not `guaca.bot`, and an
/// operator who cannot see which service they linked cannot tell the two apart.
pub async fn account_status(state: &AppState) -> Reply<crate::account::Status> {
    Ok(state.account.status())
}

/// The whole sign-in, in one call.
///
/// One command rather than the subscription's two, because there is no code for
/// the operator to carry: the browser opens, they say yes, and the answer comes
/// back to a port this process is already listening on. Nothing needs drawing
/// in between, so nothing needs a second round trip to draw it.
///
/// Parks for up to five minutes on purpose. An operator who closes the dialog
/// abandons it, and what is left behind is a closed socket.
pub async fn sign_in_account(state: &AppState) -> Reply<crate::account::Status> {
    let account = state.account.clone();
    // The system browser, not the webview. The sign-in belongs to a session
    // this app has no business holding, and the whole argument for a loopback
    // redirect is that the browser is the operator's own.
    let open = state.open_url.clone();
    let landing = state.reach.landing()?;
    Ok(account.sign_in(|url| open(url), &landing).await?)
}

/// What the account holds, asked of the service rather than remembered.
///
/// The answer changes when the operator authorizes something in a browser, not
/// when this app does anything, so a cached copy would be a list of
/// capabilities an agent is told it has and does not.
pub async fn account_connectors(state: &AppState) -> Reply<Connectors> {
    let account = state.account.clone();
    Ok(account.connectors().await?)
}

/// Forgets the sign-in on this machine.
pub async fn sign_out_account(state: &AppState) -> Reply<crate::account::Status> {
    state.account.sign_out()?;
    Ok(state.account.status())
}

// ---- the ChatGPT sign-in -------------------------------------------------

/// Whether a subscription is signed in, and which account it is.
pub async fn subscription_status(state: &AppState) -> Reply<Status> {
    Ok(state.subscription.status())
}

/// Asks for a code the operator carries to a browser.
///
/// Two commands rather than one because the two halves take wildly different
/// amounts of time. This returns in a round trip and the dialog draws the code
/// immediately; the next one waits for a person.
pub async fn begin_subscription_signin(state: &AppState) -> Reply<DeviceCode> {
    Ok(state.subscription.begin().await?)
}

/// Waits for the code to be entered, then stores the sign-in.
///
/// Parks for as long as fifteen minutes on purpose. A dialog that has to poll
/// would need a third command and a place to keep the half-finished sign-in
/// between calls; awaiting one call keeps the whole flow in one place, and an
/// operator who closes the dialog abandons it with nothing left behind.
pub async fn complete_subscription_signin(state: &AppState, code: DeviceCode) -> Reply<Status> {
    Ok(state.subscription.complete(&code).await?)
}

/// Forgets the sign-in, and moves off it if it was the one in use.
///
/// Both halves, because leaving the provider pointed at a subscription that has
/// just been signed out gives every agent the same refusal on its next turn,
/// and the operator's next action would have been to switch it back by hand.
pub async fn sign_out_subscription(state: &AppState) -> Reply<RedactedConfig> {
    state.subscription.sign_out()?;

    let mut config: AppConfig = state.runtime.config();
    if config.inference.provider == config::Provider::Chatgpt {
        config.inference.provider = config::Provider::Compatible;
        // The endpoint and key were never cleared when the subscription was
        // chosen, so going back lands on whatever was configured before it.
        config::save(&state.config_path, &config)?;
        state.runtime.set_config(config.clone());
    }
    Ok(config.redacted())
}

/// Applies a patch to a config in memory. Shared by saving and testing, so a
/// tested configuration and a saved one can never diverge.
fn apply_patch(
    here: Capabilities,
    config: &mut AppConfig,
    patch: SettingsPatch,
) -> Result<(), CommandError> {
    honorable(here, patch.provider, patch.base_url.as_deref())?;
    if let Some(name) = patch.operator_name {
        config.operator_name = name.trim().to_string();
    }
    if let Some(provider) = patch.provider {
        config.inference.provider = provider;
    }
    if let Some(base_url) = patch.base_url {
        config.inference.base_url = config::normalize_base_url(&base_url)?;
    }
    if let Some(key) = patch.e2b_api_key {
        config.e2b.api_key = key.trim().to_string();
    }
    if let Some(minutes) = patch.computer_idle_minutes {
        // A machine that sleeps after zero minutes can never be used, and one
        // that never sleeps is a bill nobody chose.
        config.e2b.idle_minutes = minutes.clamp(1, 24 * 60);
    }
    if let Some(key) = patch.kernel_api_key {
        config.kernel.api_key = key.trim().to_string();
    }
    if let Some(minutes) = patch.browser_idle_minutes {
        // Wider at the top than the machine's, because a browser on standby
        // costs nothing and the provider allows three days. Wider at the bottom
        // is not possible: ten seconds is its floor, which is a fifth of a
        // minute, so one minute is as short as this can offer.
        config.kernel.idle_minutes = minutes.clamp(1, 72 * 60);
    }
    if let Some(stealth) = patch.browser_stealth {
        config.kernel.stealth = stealth;
    }
    if let Some(api_key) = patch.api_key {
        config.inference.api_key = api_key.trim().to_string();
    }
    if let Some(model) = patch.default_model {
        let trimmed = model.trim();
        if trimmed.is_empty() {
            return Err(CommandError::new("validation", "default model must not be blank"));
        }
        config.inference.default_model = trimmed.to_string();
    }
    if let Some(model) = patch.subscription_model {
        let trimmed = model.trim();
        if trimmed.is_empty() {
            return Err(CommandError::new("validation", "subscription model must not be blank"));
        }
        config.inference.subscription_model = trimmed.to_string();
    }
    if let Some(timeout) = patch.request_timeout_secs {
        config.inference.request_timeout_secs = timeout.clamp(5, 900);
    }
    if let Some(limits) = patch.limits {
        config.limits = limits.sanitized();
    }
    Ok(())
}

pub async fn update_settings(state: &AppState, patch: SettingsPatch) -> Reply<RedactedConfig> {
    let mut config: AppConfig = state.runtime.config();
    apply_patch(state.deployment.capabilities(), &mut config, patch)?;
    config::save(&state.config_path, &config)?;
    state.runtime.set_config(config.clone());
    Ok(config.redacted())
}

/// Verifies the endpoint and key without involving an agent.
///
/// Takes the settings currently on screen rather than the saved ones, and does
/// not persist them. Testing the saved config while the operator is looking at
/// an unsaved key reports "no API key configured" for a key they can see in
/// front of them, which reads as a bug in the app.
pub async fn test_connection(state: &AppState, patch: Option<SettingsPatch>) -> Reply<String> {
    let mut config = state.runtime.config();
    if let Some(patch) = patch {
        apply_patch(state.deployment.capabilities(), &mut config, patch)?;
    }
    state
        .runtime
        .probe(&config)
        .await
        .map_err(|err| CommandError::new("inference", err.to_string()))
}

/// The models OpenRouter sees doing one kind of work, most capable first.
///
/// The one command that reads a catalog rather than this install's own state,
/// and it is still not the frontend performing network access: the host is a
/// constant here and the use case is checked against a published set before a
/// request is spent, so the webview names a use case rather than a URL.
///
/// Offered beside an agent's model field, so it is asked for while a dialog is
/// open and at no other time. Nothing is blocked on it and no turn reads it.
pub async fn ranked_models(state: &AppState, category: String) -> Reply<Vec<RankedModel>> {
    Ok(state.catalog.ranked(&category).await?)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_duplicate_name_is_distinguishable_from_a_disk_failure() {
        let dup: CommandError = crate::db::StoreError::DuplicateName("Manager".into()).into();
        assert_eq!(dup.kind, "duplicateName");

        let corrupt: CommandError = crate::db::StoreError::Corrupt("bad row".into()).into();
        assert_eq!(corrupt.kind, "storage");
    }

    #[test]
    fn a_settings_patch_with_no_fields_deserializes_to_all_none() {
        let patch: SettingsPatch = serde_json::from_str("{}").unwrap();
        assert!(patch.base_url.is_none());
        assert!(patch.api_key.is_none(), "an absent key must not be read as a request to clear it");
    }

    #[test]
    fn a_settings_patch_accepts_camel_case_from_the_frontend() {
        let patch: SettingsPatch =
            serde_json::from_str(r#"{"baseUrl":"https://x/v1","requestTimeoutSecs":60}"#).unwrap();
        assert_eq!(patch.base_url.as_deref(), Some("https://x/v1"));
        assert_eq!(patch.request_timeout_secs, Some(60));
    }

    #[test]
    fn a_patch_leaves_absent_fields_alone() {
        let mut config = AppConfig::default();
        config.inference.api_key = "stored-key".into();
        config.inference.default_model = "stored/model".into();

        apply_patch(
            Deployment::Desktop.capabilities(),
            &mut config,
            SettingsPatch { base_url: Some("https://x/v1".into()), ..Default::default() },
        )
        .unwrap();

        assert_eq!(config.inference.base_url, "https://x/v1");
        assert_eq!(config.inference.api_key, "stored-key", "an absent key must not be cleared");
        assert_eq!(config.inference.default_model, "stored/model");
    }

    #[test]
    fn a_patch_can_supply_a_key_that_was_never_saved() {
        // The Test connection path: the operator has typed a key but not saved.
        let mut config = AppConfig::default();
        assert!(!config.inference.is_ready(), "no key stored yet");

        apply_patch(
            Deployment::Desktop.capabilities(),
            &mut config,
            SettingsPatch { api_key: Some("  sk-typed  ".into()), ..Default::default() },
        )
        .unwrap();

        assert_eq!(config.inference.api_key, "sk-typed", "whitespace is trimmed");
        assert!(config.inference.is_ready(), "the typed key must be usable without saving");
    }

    #[test]
    fn a_patch_rejects_a_blank_model_rather_than_storing_one() {
        let mut config = AppConfig::default();
        let err = apply_patch(
            Deployment::Desktop.capabilities(),
            &mut config,
            SettingsPatch { default_model: Some("   ".into()), ..Default::default() },
        )
        .unwrap_err();
        assert_eq!(err.kind, "validation");
    }

    #[test]
    fn command_errors_serialize_with_a_kind_the_ui_can_branch_on() {
        let json = serde_json::to_value(CommandError::new("validation", "bad")).unwrap();
        assert_eq!(json["kind"], "validation");
        assert_eq!(json["message"], "bad");
    }
}
