mod catalog;

pub use catalog::{Catalog, RankedModel};

use crate::domain::decision::WorkDecision;
use crate::domain::ids::DecisionId;
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;

use serde::{Deserialize, Serialize};

use crate::account::{Account, AccountError, Connectors};
use crate::artifact::Artifacts;
use crate::config::{self, AppConfig, RedactedConfig};
use crate::domain::agent::{copy_name, hire_names, AgentCard, AgentDraft, Consent, Lifecycle};
use crate::domain::approval::{Approval, ApprovalState, Decision, ProtectedAction};
use crate::domain::artifact::{self as artifact_domain, Actor, Artifact};
use crate::domain::attachment::Attachment;
use crate::domain::connector::{Connector, ConnectorDraft};
use crate::domain::deployment::{Absent, Capabilities, Deployment};
use crate::domain::envelope::Envelope;
use crate::domain::escalation::Escalation;
use crate::domain::group::{Group, GroupDraft, GroupInference};
use crate::domain::ids::{
    AgentId, ApprovalId, ArtifactId, ConnectorId, EscalationId, GroupId, MessageId, OccasionId,
    PluginId, RoutineId, RunId,
};
use crate::domain::now_ms;
use crate::domain::occasion::{self, Occasion};
use crate::domain::plugin::{
    self, HeaderPair, Headers, Plugin, PluginAccess, PluginKind, PluginOffer, ServerReport,
};
use crate::domain::routine::{self, Routine, RoutineRun, Trigger, UpcomingRun};
use crate::domain::search::SearchHits;
use crate::domain::signin::Signin;
use crate::domain::skill::{Scope as SkillScope, Skill};
use crate::domain::usage::{GroupUsage, RunUsage};
use crate::domain::worknote::WorkingNote;
use crate::e2b::{Computer, E2bClient, E2bError};
use crate::kernel::{Browser, KernelClient, KernelError};
use crate::runtime::events::{Activity, UiEvent};
use crate::runtime::Runtime;
use crate::subscription::{DeviceCode, SigninError, Status, Subscription};
use catalog::CatalogError;

pub type OpenUrl = Arc<dyn Fn(&str) -> Result<(), String> + Send + Sync>;

pub enum Reach {
    Desktop,
    Served {
        fixed: Option<String>,
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

    pub fn note(&self, origin: String) {
        if let Reach::Served { seen, .. } = self {
            *seen.lock().unwrap_or_else(std::sync::PoisonError::into_inner) = Some(origin);
        }
    }

    pub fn origin(&self) -> Option<String> {
        match self {
            Reach::Desktop => None,
            Reach::Served { fixed, seen, .. } => fixed
                .clone()
                .or_else(|| seen.lock().unwrap_or_else(std::sync::PoisonError::into_inner).clone()),
        }
    }

    pub fn callbacks(&self) -> Option<crate::oauth::Callbacks> {
        match self {
            Reach::Desktop => None,
            Reach::Served { callbacks, .. } => Some(callbacks.clone()),
        }
    }

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

pub type MenubarFeed = Arc<dyn Fn(Option<crate::menubar::Presence>) + Send + Sync>;

pub struct AppState {
    pub runtime: Runtime,
    pub menubar: MenubarFeed,
    pub secret: Option<Arc<str>>,
    pub deployment: Deployment,
    pub open_url: OpenUrl,
    pub reach: Reach,
    pub config_path: PathBuf,
    pub downloads: PathBuf,
    pub subscription: Arc<Subscription>,
    pub claude_signin: crate::claude_signin::Signin,
    pub account: Arc<Account>,
    pub catalog: Arc<Catalog>,
    pub artifacts: Artifacts,
    pub artifact_port: Arc<std::sync::atomic::AtomicU16>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CommandError {
    pub kind: &'static str,
    pub message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
}

impl CommandError {
    pub fn new(kind: &'static str, message: impl Into<String>) -> Self {
        Self { kind, message: message.into(), detail: None }
    }

    #[must_use]
    pub fn with_detail(mut self, detail: impl Into<String>) -> Self {
        self.detail = Some(detail.into());
        self
    }
}

impl From<Absent> for CommandError {
    fn from(absent: Absent) -> Self {
        CommandError::new("notHere", absent.sentence())
    }
}

fn honorable(
    here: Capabilities,
    provider: Option<config::Provider>,
    base_url: Option<&str>,
) -> Result<(), CommandError> {
    if provider == Some(config::Provider::Claude) {
        here.require(Absent::ClaudeProvider)?;
    }
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
            | StoreError::ConnectorNotFound(_) => CommandError::new("notFound", err.to_string()),
            StoreError::ApprovalSettled { .. } => {
                CommandError::new("alreadyAnswered", err.to_string())
            }
            StoreError::DuplicateEnvVar(_) => CommandError::new("validation", err.to_string()),
            StoreError::GroupNotEmpty { .. } | StoreError::CannotDeleteLastGroup => {
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
            RuntimeError::NotAVerdict | RuntimeError::NotAQuestion | RuntimeError::EmptyAnswer => {
                CommandError::new("badRequest", err.to_string())
            }
        }
    }
}

impl From<AccountError> for CommandError {
    fn from(err: AccountError) -> Self {
        match err {
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

impl From<crate::notebook::NotebookError> for CommandError {
    fn from(err: crate::notebook::NotebookError) -> Self {
        use crate::notebook::NotebookError;
        let kind = match &err {
            NotebookError::NotFound(_) => "notFound",
            NotebookError::Io { .. } => "storage",
            _ => "validation",
        };
        CommandError::new(kind, err.to_string())
    }
}

impl From<crate::skills_sh::DirectoryError> for CommandError {
    fn from(err: crate::skills_sh::DirectoryError) -> Self {
        use crate::skills_sh::DirectoryError;
        let kind = match err {
            DirectoryError::Store(inner) => return inner.into(),
            DirectoryError::Missing(_) => "notFound",
            DirectoryError::Changed => "changed",
            DirectoryError::NotAddable(_)
            | DirectoryError::ShortQuery
            | DirectoryError::Invalid(_) => "validation",
            DirectoryError::Unreachable(_)
            | DirectoryError::Busy
            | DirectoryError::Status(_)
            | DirectoryError::Unexpected(_) => "directory",
        };
        CommandError::new(kind, err.to_string())
    }
}

impl From<crate::skills::SkillsError> for CommandError {
    fn from(err: crate::skills::SkillsError) -> Self {
        use crate::skills::SkillsError;
        let kind = match &err {
            SkillsError::NotFound(_) | SkillsError::NoFile { .. } => "notFound",
            SkillsError::ReadOnly(_) | SkillsError::NotYours { .. } => "readOnly",
            SkillsError::Taken(_) => "validation",
            SkillsError::Invalid(_) => "validation",
            SkillsError::Io { .. } => "storage",
        };
        CommandError::new(kind, err.to_string())
    }
}

impl From<crate::plugins::PluginError> for CommandError {
    fn from(err: crate::plugins::PluginError) -> Self {
        match err {
            crate::plugins::PluginError::Signin(_) => {
                CommandError::new("pluginSignin", err.to_string())
            }
            crate::plugins::PluginError::Unreachable { says, detail } => {
                CommandError::new("plugin", says).with_detail(detail)
            }
            other => CommandError::new("plugin", other.to_string()),
        }
    }
}

impl From<E2bError> for CommandError {
    fn from(err: E2bError) -> Self {
        match err {
            E2bError::NoKey => CommandError::new("computerUnconfigured", err.to_string()),
            other => CommandError::new("computer", other.to_string()),
        }
    }
}

impl From<KernelError> for CommandError {
    fn from(err: KernelError) -> Self {
        match err {
            KernelError::NoKey => CommandError::new("browserUnconfigured", err.to_string()),
            other => CommandError::new("browser", other.to_string()),
        }
    }
}

impl From<config::ConfigError> for CommandError {
    fn from(err: config::ConfigError) -> Self {
        let kind = match err {
            config::ConfigError::Blank { .. } | config::ConfigError::Quick(_) => "validation",
            _ => "config",
        };
        CommandError::new(kind, err.to_string())
    }
}

impl From<SigninError> for CommandError {
    fn from(err: SigninError) -> Self {
        match err {
            SigninError::TimedOut => CommandError::new("signinExpired", err.to_string()),
            other => CommandError::new("signin", other.to_string()),
        }
    }
}

impl From<CatalogError> for CommandError {
    fn from(err: CatalogError) -> Self {
        match err {
            CatalogError::Unsupported { .. } | CatalogError::Withdrawn(_) => {
                CommandError::new("useCase", err.to_string())
            }
            other => CommandError::new("catalog", other.to_string()),
        }
    }
}

pub type Reply<T> = Result<T, CommandError>;

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

pub async fn agent_computer(state: &AppState, id: AgentId) -> Reply<Option<Computer>> {
    let card = agent_card(state, id)?;
    let (Some(sandbox), Some(envd)) = (card.sandbox_id, card.sandbox_envd_token) else {
        return Ok(None);
    };
    let client = computers(state)?;

    if client.state(&sandbox).await? == crate::e2b::SandboxState::Gone {
        state.runtime.store().clear_agent_sandbox_if(id, &sandbox)?;
        return Ok(None);
    }
    Ok(Some(state.screened(client.describe(&sandbox, &envd, state.runtime.viewer_port()).await?)))
}

pub async fn give_agent_computer(state: &AppState, id: AgentId) -> Reply<()> {
    state.runtime.store().set_has_computer(id, true)?;
    state.runtime.emit(UiEvent::AgentsChanged);
    Ok(())
}

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

pub async fn start_agent_computer(state: &AppState, id: AgentId) -> Reply<Computer> {
    let card = agent_card(state, id)?;
    let (client, sandbox) = state.runtime.ensure_computer(&card).await?;

    client.start_desktop(&sandbox.id, &sandbox.envd_token).await?;
    let computer =
        client.describe(&sandbox.id, &sandbox.envd_token, state.runtime.viewer_port()).await?;
    state.runtime.emit(UiEvent::AgentsChanged);
    Ok(state.screened(computer))
}

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

pub async fn delete_agent_computer(state: &AppState, id: AgentId) -> Reply<()> {
    let Some(sandbox) = agent_card(state, id)?.sandbox_id else {
        return Ok(());
    };
    computers(state)?.kill(&sandbox).await?;
    state.runtime.store().clear_agent_sandbox_if(id, &sandbox)?;
    state.runtime.emit(UiEvent::AgentsChanged);
    Ok(())
}

fn browsers(state: &AppState) -> Reply<KernelClient> {
    KernelClient::new(&state.runtime.config().kernel.api_key)
        .ok_or_else(|| KernelError::NoKey.into())
}

pub async fn agent_browser(state: &AppState, id: AgentId) -> Reply<Option<Browser>> {
    let Some(browser) = agent_card(state, id)?.browser_id else {
        return Ok(None);
    };
    let client = browsers(state)?;

    match client.get(&browser).await? {
        Some(session) => Ok(Some(Browser::running(session))),
        None => {
            state.runtime.store().clear_agent_browser_if(id, &browser)?;
            Ok(None)
        }
    }
}

pub async fn give_agent_browser(state: &AppState, id: AgentId) -> Reply<()> {
    state.runtime.store().set_has_browser(id, true)?;
    state.runtime.emit(UiEvent::AgentsChanged);
    Ok(())
}

pub async fn set_agent_browser_consent(
    state: &AppState,
    id: AgentId,
    consent: Consent,
) -> Reply<()> {
    state.runtime.store().set_browser_consent(id, consent)?;
    state.runtime.emit(UiEvent::AgentsChanged);
    Ok(())
}

pub async fn set_agent_errands(state: &AppState, id: AgentId, given: bool) -> Reply<()> {
    state.runtime.store().set_runs_errands(id, given)?;
    state.runtime.emit(UiEvent::AgentsChanged);
    Ok(())
}

pub async fn take_agent_browser(state: &AppState, id: AgentId) -> Reply<()> {
    let card = agent_card(state, id)?;
    state.runtime.store().set_has_browser(id, false)?;

    if let Some(browser) = card.browser_id {
        match browsers(state) {
            Ok(client) => {
                if let Err(err) = client.delete(&browser).await {
                    tracing::warn!(%err, %browser, "could not close a browser that was taken back");
                } else {
                    state.runtime.store().clear_agent_browser_if(id, &browser)?;
                }
            }
            Err(err) => tracing::warn!(?err, "no client to close a browser that was taken back"),
        }
    }
    state.runtime.emit(UiEvent::AgentsChanged);
    Ok(())
}

pub async fn start_agent_browser(state: &AppState, id: AgentId) -> Reply<Browser> {
    let card = agent_card(state, id)?;
    let (_, session) = state.runtime.ensure_browser(&card).await?;
    state.runtime.emit(UiEvent::AgentsChanged);
    Ok(Browser::running(session))
}

pub async fn stop_agent_browser(state: &AppState, id: AgentId) -> Reply<()> {
    let Some(browser) = agent_card(state, id)?.browser_id else {
        return Ok(());
    };
    browsers(state)?.delete(&browser).await?;
    state.runtime.store().clear_agent_browser_if(id, &browser)?;
    state.runtime.emit(UiEvent::AgentsChanged);
    Ok(())
}

pub async fn group_connectors(state: &AppState, group_id: GroupId) -> Reply<Vec<Connector>> {
    Ok(state.runtime.store().group_connectors(group_id)?)
}

pub async fn create_connector(state: &AppState, draft: ConnectorDraft) -> Reply<Connector> {
    let clean = draft.validate()?;
    let connector = state.runtime.store().create_connector(&clean)?;
    state.runtime.emit(UiEvent::AgentsChanged);
    Ok(connector)
}

pub async fn update_connector(
    state: &AppState,
    id: ConnectorId,
    agents: Vec<AgentId>,
    secret: Option<String>,
) -> Reply<()> {
    if let Some(value) = &secret {
        crate::domain::connector::validate_secret(value)?;
    }
    state.runtime.store().update_connector(id, &agents, secret.as_deref().map(str::trim))?;
    state.runtime.emit(UiEvent::AgentsChanged);
    Ok(())
}

pub async fn delete_connector(state: &AppState, id: ConnectorId) -> Reply<()> {
    state.runtime.store().delete_connector(id)?;
    state.runtime.emit(UiEvent::AgentsChanged);
    Ok(())
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ClaudeProgram {
    pub installed: bool,
    pub version: String,
    pub install: String,
}

pub async fn claude_program(_state: &AppState) -> Reply<ClaudeProgram> {
    let version = crate::llm::claude::presence().await;
    Ok(ClaudeProgram {
        installed: version.is_some(),
        version: version.unwrap_or_default(),
        install: crate::llm::claude::INSTALL.to_string(),
    })
}

pub async fn claude_signin_status(_state: &AppState) -> Reply<crate::claude_signin::Status> {
    Ok(crate::claude_signin::Signin::status(crate::llm::claude::PROGRAM).await)
}

pub async fn begin_claude_signin(state: &AppState) -> Reply<crate::claude_signin::Started> {
    state
        .claude_signin
        .begin(crate::llm::claude::PROGRAM)
        .await
        .map_err(|message| CommandError::new("claudeSignin", message))
}

pub async fn complete_claude_signin(
    state: &AppState,
    id: String,
    code: String,
) -> Reply<crate::claude_signin::Status> {
    state
        .claude_signin
        .complete(&id, code)
        .await
        .map_err(|message| CommandError::new("claudeSignin", message))
}

pub async fn cancel_claude_signin(state: &AppState, id: String) -> Reply<()> {
    state.claude_signin.cancel(&id).await;
    Ok(())
}

pub async fn existing_signins(state: &AppState) -> Reply<Vec<crate::pickup::Pickup>> {
    if state.deployment.is_server() {
        return Ok(Vec::new());
    }
    Ok(crate::pickup::find().await)
}

pub async fn plugin_catalog(_state: &AppState) -> Reply<Vec<PluginOffer>> {
    Ok(plugin::catalog())
}

pub async fn group_plugins(state: &AppState, group_id: GroupId) -> Reply<Vec<Plugin>> {
    Ok(state.runtime.store().group_plugins(group_id)?)
}

pub async fn connect_plugin(
    state: &AppState,
    group_id: GroupId,
    kind: PluginKind,
    connection: Option<String>,
) -> Reply<Plugin> {
    let read = if kind.account_backed() { Some(state.account.access().await) } else { None };
    let connection = connection.unwrap_or_default();
    let credential = match &read {
        Some(read) => crate::plugins::Credential::Account(
            crate::plugins::Held::read(read, &connection).spend(&kind)?,
        ),
        None => crate::plugins::Credential::Discover,
    };

    let endpoint = if kind.account_backed() {
        crate::plugins::AccountUse::endpoint(state.account.origin(), &connection)
    } else {
        state.runtime.plugin_endpoint(&kind)
    };

    sign_in(state, group_id, &kind, &endpoint, credential, &Headers::none()).await
}

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

    let (key, headers) = presented(key, headers, kind.is_stdio())?;
    let credential = match key.as_deref() {
        Some(key) => crate::plugins::Credential::Key(key),
        None => crate::plugins::Credential::Discover,
    };
    let endpoint = state.runtime.plugin_endpoint(&kind);
    sign_in(state, group_id, &kind, &endpoint, credential, &headers).await
}

fn presented(
    key: Option<String>,
    headers: Option<Vec<HeaderPair>>,
    stdio: bool,
) -> Result<(Option<String>, Headers), CommandError> {
    let key = key.map(|key| key.trim().to_string()).filter(|key| !key.is_empty());
    let rows = headers.unwrap_or_default();
    if stdio {
        if key.is_some() {
            return Err(CommandError::new(
                "validation",
                "a server run on the host takes its key as an environment variable, such as \
                 GITHUB_TOKEN. Put it there and clear the key.",
            ));
        }
        let env = Headers::parse_env(&rows)
            .map_err(|err| CommandError::new("validation", err.to_string()))?;
        return Ok((None, env));
    }
    let headers =
        Headers::parse(&rows).map_err(|err| CommandError::new("validation", err.to_string()))?;
    if key.is_some() && headers.carries_authorization() {
        return Err(CommandError::new(
            "validation",
            "you gave a key and an `authorization` header, and they are the same slot on the \
             request. Keep the key and drop the header, or keep the header (which is how to \
             send a scheme other than `Bearer`) and clear the key.",
        ));
    }
    Ok((key, headers))
}

async fn sign_in(
    state: &AppState,
    group_id: GroupId,
    kind: &PluginKind,
    endpoint: &str,
    credential: crate::plugins::Credential<'_>,
    headers: &Headers,
) -> Reply<Plugin> {
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
            let open = state.open_url.clone();
            move |url: &str| open(url)
        },
    )
    .await
    .map_err(crate::plugins::said_to_operator)?;

    state.runtime.emit(UiEvent::AgentsChanged);
    Ok(plugin)
}

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
        .ok_or_else(|| CommandError::new("validation", "that connector is not in this group"))?;
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

    let kind = PluginKind::custom(&held.name, &url)
        .map_err(|err| CommandError::new("validation", err.to_string()))?;
    let (key, headers) = match headers {
        Some(rows) => presented(key, Some(rows), kind.is_stdio())?,
        None => {
            let stored = state
                .runtime
                .store()
                .plugin_dial(id)?
                .map(|dialed| dialed.headers)
                .filter(|stored| stored.is_env() == kind.is_stdio())
                .unwrap_or_default();
            let (key, _) = presented(key, None, kind.is_stdio())?;
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

pub async fn probe_server(
    _state: &AppState,
    url: String,
    key: Option<String>,
    headers: Option<Vec<HeaderPair>>,
) -> Reply<ServerReport> {
    let endpoint = plugin::canonical_address(&url)
        .map_err(|err| CommandError::new("validation", err.to_string()))?;
    let stdio = endpoint.starts_with(plugin::STDIO);
    let (key, headers) = presented(key, headers, stdio)?;
    Ok(crate::plugins::inspect(true, &endpoint, key.as_deref(), &headers).await?)
}

pub async fn check_plugin(state: &AppState, id: PluginId) -> Reply<ServerReport> {
    let dialed = state
        .runtime
        .store()
        .plugin_dial(id)?
        .ok_or_else(|| CommandError::new("validation", "that connector is not connected"))?;

    let read = if dialed.kind.account_backed() { Some(state.account.access().await) } else { None };
    let endpoint = if dialed.kind.account_backed() {
        crate::plugins::AccountUse::endpoint(state.account.origin(), &dialed.connection)
    } else {
        state.runtime.plugin_endpoint(&dialed.kind)
    };
    let connection = dialed.connection.clone();
    let account = match &read {
        Some(read) => crate::plugins::Held::read(read, &connection),
        None => crate::plugins::Held::Absent,
    };

    Ok(crate::plugins::check(state.runtime.store(), id, dialed, &endpoint, account).await?)
}

pub async fn set_plugin_access(
    state: &AppState,
    id: PluginId,
    access: PluginAccess,
) -> Reply<Plugin> {
    let plugin = state.runtime.store().set_plugin_access(id, &access)?;
    state.runtime.emit(UiEvent::AgentsChanged);
    Ok(plugin)
}

pub async fn set_plugin_tool(
    state: &AppState,
    id: PluginId,
    tool: String,
    access: PluginAccess,
) -> Reply<Plugin> {
    let plugin = state.runtime.store().set_plugin_tool(id, &tool, &access)?;
    state.runtime.emit(UiEvent::AgentsChanged);
    Ok(plugin)
}

pub async fn disconnect_plugin(state: &AppState, id: PluginId) -> Reply<()> {
    state.runtime.store().delete_plugin(id)?;
    state.runtime.emit(UiEvent::AgentsChanged);
    Ok(())
}

pub async fn scan_agent_signins(state: &AppState, id: AgentId) -> Reply<Vec<Signin>> {
    Ok(state.runtime.scan_signins(id).await?)
}

pub async fn agent_signins(state: &AppState, id: AgentId) -> Reply<Vec<Signin>> {
    Ok(state.runtime.store().agent_signins(id)?)
}

const MAX_PENDING: u32 = 200;

pub async fn approval_states(state: &AppState) -> Reply<HashMap<ApprovalId, ApprovalState>> {
    Ok(state.runtime.store().approval_states(500)?)
}

pub async fn approval_answers(state: &AppState) -> Reply<HashMap<ApprovalId, String>> {
    Ok(state.runtime.store().approval_answers(500)?)
}

pub async fn pending_approvals(state: &AppState) -> Reply<Vec<Approval>> {
    let window = state.runtime.approval_window();
    Ok(state
        .runtime
        .store()
        .pending_approvals(MAX_PENDING)?
        .into_iter()
        .map(|approval| approval.lapsing_after(window))
        .collect())
}

pub async fn agent_grants(state: &AppState, id: AgentId) -> Reply<Vec<ProtectedAction>> {
    Ok(state.runtime.store().standing_grants(id)?)
}

pub async fn list_permission_rules(
    state: &AppState,
    group_id: Option<GroupId>,
) -> Reply<Vec<crate::domain::policy::Rule>> {
    Ok(match group_id {
        Some(group) => state.runtime.store().policies_for(group)?,
        None => state.runtime.store().list_policies(None)?,
    })
}

pub async fn save_permission_rule(
    state: &AppState,
    id: Option<String>,
    version: Option<u64>,
    rule: crate::domain::policy::RuleDraft,
) -> Reply<crate::domain::policy::Rule> {
    let store = state.runtime.store();
    let saved = match (id, version) {
        (None, None) => store.create_policy(rule)?,
        (Some(id), Some(version)) => store.update_policy(&id, rule.group_id, version, rule)?,
        _ => return Err(CommandError::new("invalid", "A rule edit needs its id and version")),
    };
    Ok(saved)
}

pub async fn delete_permission_rule(
    state: &AppState,
    id: String,
    group_id: Option<GroupId>,
    version: u64,
) -> Reply<()> {
    if !state.runtime.store().delete_policy(&id, group_id, version)? {
        return Err(CommandError::new("conflict", "The rule is missing or has changed"));
    }
    Ok(())
}

pub async fn revoke_grant(
    state: &AppState,
    id: AgentId,
    action: ProtectedAction,
) -> Reply<Vec<ProtectedAction>> {
    state.runtime.store().revoke_grant(id, action)?;
    Ok(state.runtime.store().standing_grants(id)?)
}

pub async fn decide_approval(
    state: &AppState,
    id: ApprovalId,
    decision: Decision,
) -> Reply<Approval> {
    Ok(state.runtime.decide_approval(id, decision)?)
}

pub async fn answer_question(state: &AppState, id: ApprovalId, answer: String) -> Reply<Approval> {
    Ok(state.runtime.answer_question(id, &answer)?)
}

pub async fn open_escalations(state: &AppState) -> Reply<Vec<Escalation>> {
    Ok(state.runtime.store().open_escalations(MAX_PENDING)?)
}

pub async fn clear_escalation(state: &AppState, id: EscalationId) -> Reply<()> {
    state.runtime.clear_escalation(id)?;
    Ok(())
}

const MAX_OCCASIONS: u32 = 500;

pub async fn calendar(
    state: &AppState,
    from: i64,
    until: i64,
    group_id: Option<GroupId>,
) -> Reply<Vec<Occasion>> {
    Ok(state.runtime.store().occasions(from, until, group_id, MAX_OCCASIONS)?)
}

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

pub async fn create_occasion(state: &AppState, draft: OccasionDraft) -> Reply<Occasion> {
    let clean = draft.checked()?;
    let occasion = state.runtime.store().create_occasion(&clean)?;
    state.runtime.emit(UiEvent::CalendarChanged { group_id: occasion.group_id });
    Ok(occasion)
}

pub async fn update_occasion(
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

pub async fn delete_occasion(state: &AppState, id: OccasionId) -> Reply<()> {
    let Some(existing) = state.runtime.store().any_occasion(id)? else {
        return Ok(());
    };
    state.runtime.store().delete_occasion(id, existing.group_id)?;
    state.runtime.emit(UiEvent::CalendarChanged { group_id: existing.group_id });
    Ok(())
}

const MAX_ARTIFACTS: u32 = 500;

pub async fn artifacts(state: &AppState, group_id: Option<GroupId>) -> Reply<Vec<Artifact>> {
    Ok(state.runtime.store().artifacts(group_id, MAX_ARTIFACTS)?)
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ArtifactDetail {
    pub artifact: Artifact,
    pub log: Vec<artifact_domain::Entry>,
}

fn no_artifact(id: ArtifactId) -> CommandError {
    CommandError::new(
        "notFound",
        format!("There is no artifact with id {id} any more. Somebody may have deleted it."),
    )
}

pub async fn artifact_detail(state: &AppState, id: ArtifactId) -> Reply<ArtifactDetail> {
    let store = state.runtime.store();
    let artifact = store.any_artifact(id)?.ok_or_else(|| no_artifact(id))?;
    Ok(ArtifactDetail { artifact, log: store.artifact_log(id)? })
}

pub async fn artifact_page(
    state: &AppState,
    id: ArtifactId,
    version: Option<u32>,
) -> Reply<String> {
    state.runtime.store().artifact_page(id, version)?.ok_or_else(|| match version {
        Some(version) => CommandError::new(
            "notFound",
            format!("This artifact has no version {version}. Pick one from its history."),
        ),
        None => no_artifact(id),
    })
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CondensedView {
    pub artifact: Artifact,
    pub page: Option<String>,
}

pub async fn artifact_condensed(state: &AppState, id: ArtifactId) -> Reply<CondensedView> {
    let store = state.runtime.store();
    let artifact = store.any_artifact(id)?.ok_or_else(|| no_artifact(id))?;
    Ok(CondensedView { artifact, page: store.artifact_condensed(id)? })
}

pub async fn pin_artifact(
    state: &AppState,
    id: ArtifactId,
    width: crate::domain::widget::Width,
) -> Reply<RedactedConfig> {
    use crate::domain::widget::{self, Widget};
    let artifact = state.runtime.store().any_artifact(id)?.ok_or_else(|| no_artifact(id))?;
    if !artifact.condensed {
        return Err(CommandError::new(
            "validation",
            "This page has no condensed view, which is what the status bar draws. Ask its owner \
             for one.",
        ));
    }
    let config = state.runtime.change_config(|config| {
        if config.widgets.iter().any(|pinned| pinned.artifact_id == id) {
            widget::resize(&mut config.widgets, id, width)
        } else {
            Widget::new(id, width, None, "the operator")
                .and_then(|made| widget::pin(&mut config.widgets, made))
        }
        .map_err(config::ConfigError::from)
    })?;
    Ok(config.redacted())
}

pub async fn unpin_artifact(state: &AppState, id: ArtifactId) -> Reply<RedactedConfig> {
    let config = state.runtime.change_config(|config| {
        crate::domain::widget::unpin(&mut config.widgets, id)
            .map(|_| ())
            .map_err(config::ConfigError::from)
    })?;
    Ok(config.redacted())
}

pub async fn artifact_data(
    state: &AppState,
    id: ArtifactId,
    version: Option<u32>,
) -> Reply<Vec<artifact_domain::Read>> {
    state.runtime.read_artifact(id, version).await?.ok_or_else(|| no_artifact(id))
}

pub async fn allow_artifact_sources(_state: &AppState, _id: ArtifactId) -> Reply<Artifact> {
    Err(CommandError::new("clientOutdated", "Update Guaca on this device to approve this page."))
}

pub async fn approve_artifact_sources(
    state: &AppState,
    id: ArtifactId,
    version: u32,
    owner_id: Option<AgentId>,
    sources: Vec<artifact_domain::Source>,
) -> Reply<Artifact> {
    let store = state.runtime.store();
    let matched = store
        .approve_artifact_sources(id, version, owner_id, &sources)?
        .ok_or_else(|| no_artifact(id))?;
    if !matched {
        return Err(CommandError::new(
            "conflict",
            "This page changed. Review its reads before allowing them.",
        ));
    }
    let allowed = store.any_artifact(id)?.ok_or_else(|| no_artifact(id))?;
    state.runtime.emit(UiEvent::ArtifactsChanged { group_id: allowed.group_id });
    Ok(allowed)
}

pub async fn restore_artifact(state: &AppState, id: ArtifactId, version: u32) -> Reply<Artifact> {
    let restored = state
        .runtime
        .store()
        .restore_artifact(id, version, &Actor::Operator)?
        .ok_or_else(|| {
            CommandError::new(
                "notFound",
                format!("This artifact has no version {version} to put back. Pick one from its history."),
            )
        })?;
    state.runtime.emit(UiEvent::ArtifactsChanged { group_id: restored.group_id });
    Ok(restored)
}

pub async fn hand_artifact(state: &AppState, id: ArtifactId, agent_id: AgentId) -> Reply<Artifact> {
    let handed = state.runtime.store().hand_artifact(id, agent_id).map_err(|err| match err {
        crate::db::StoreError::AgentNotInGroup(_) => CommandError::new(
            "validation",
            "That agent is not in this artifact's group. Pick one of the group's own agents.",
        ),
        other => other.into(),
    })?;
    let handed = handed.ok_or_else(|| no_artifact(id))?;
    state.runtime.emit(UiEvent::ArtifactsChanged { group_id: handed.group_id });
    Ok(handed)
}

pub async fn delete_artifact(state: &AppState, id: ArtifactId) -> Reply<()> {
    let Some(existing) = state.runtime.store().any_artifact(id)? else {
        return Ok(());
    };
    state.runtime.store().delete_artifact(id)?;
    state.runtime.unpin_missing();
    state.runtime.emit(UiEvent::ArtifactsChanged { group_id: existing.group_id });
    Ok(())
}

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

pub async fn move_group(
    state: &AppState,
    id: GroupId,
    before: Option<GroupId>,
) -> Reply<Vec<Group>> {
    let groups = state.runtime.store().move_group(id, before)?;
    state.runtime.emit(UiEvent::AgentsChanged);
    Ok(groups)
}

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

pub async fn delete_group(state: &AppState, id: GroupId) -> Reply<()> {
    state.runtime.store().delete_group(id)?;
    state.runtime.emit(UiEvent::AgentsChanged);
    Ok(())
}

pub async fn disband_group(state: &AppState, id: GroupId) -> Reply<()> {
    state.runtime.store().group_for_removal(id)?;

    let outcome = async {
        for card in state.runtime.store().group_crew(id)? {
            state.runtime.purge_agent(&card).await?;
        }
        state.runtime.store().delete_group(id)?;
        state.runtime.skills().forget_crew(id);
        Ok(())
    }
    .await;

    state.runtime.unpin_missing();
    state.runtime.emit(UiEvent::AgentsChanged);
    outcome
}

pub async fn builtin_tools(_state: &AppState) -> Reply<Vec<crate::llm::tools::ToolSummary>> {
    Ok(crate::llm::tools::catalog())
}

pub async fn add_quick_action(
    state: &AppState,
    label: String,
    does: crate::domain::quick::Does,
) -> Reply<RedactedConfig> {
    quick_targets_exist(state, &does)?;
    let action = crate::domain::quick::QuickAction::new(&label, does, "the operator")
        .map_err(config::ConfigError::from)?;
    let config = state.runtime.change_config(|config| {
        crate::domain::quick::add(&mut config.quick_actions, action)
            .map_err(config::ConfigError::from)?;
        Ok::<_, config::ConfigError>(())
    })?;
    Ok(config.redacted())
}

pub async fn remove_quick_action(state: &AppState, id: String) -> Reply<RedactedConfig> {
    let config = state.runtime.change_config(|config| {
        crate::domain::quick::remove(&mut config.quick_actions, &id)
            .map_err(config::ConfigError::from)?;
        Ok::<_, config::ConfigError>(())
    })?;
    Ok(config.redacted())
}

fn quick_targets_exist(
    state: &AppState,
    does: &crate::domain::quick::Does,
) -> Result<(), CommandError> {
    use crate::domain::quick::{Does, Place};
    let store = state.runtime.store();
    let missing = match does {
        Does::Message { agent_id, .. } | Does::Open { place: Place::Channel { agent_id } } => {
            store.get_agent(*agent_id)?.is_none()
        }
        Does::Open { place: Place::CrewSettings { group_id } } => {
            store.get_group(*group_id)?.is_none()
        }
        Does::Open { .. } => false,
    };
    if missing {
        return Err(CommandError::new("notFound", "that agent or group no longer exists"));
    }
    Ok(())
}

pub async fn report_view(
    state: &AppState,
    view: crate::domain::view::OperatorView,
    focused: Option<bool>,
    client: Option<String>,
) -> Reply<()> {
    state.runtime.report_window(view, focused.unwrap_or(true), client);
    Ok(())
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SkillDraft {
    pub name: String,
    pub description: String,
    pub body: String,
    #[serde(default)]
    pub previous: Option<String>,
}

fn skill_scope(state: &AppState, scope: SkillScope) -> Result<SkillScope, CommandError> {
    if let SkillScope::Crew { group_id } = scope {
        state
            .runtime
            .store()
            .get_group(group_id)?
            .ok_or_else(|| CommandError::new("notFound", "that group no longer exists"))?;
    }
    Ok(scope)
}

pub async fn list_skills(state: &AppState, scope: SkillScope) -> Reply<Vec<Skill>> {
    let scope = skill_scope(state, scope)?;
    Ok(state.runtime.skills().in_scope(scope))
}

pub async fn read_skill(state: &AppState, scope: SkillScope, name: String) -> Reply<Skill> {
    let scope = skill_scope(state, scope)?;
    Ok(state.runtime.skills().load(scope, &name)?)
}

pub async fn save_skill(state: &AppState, scope: SkillScope, draft: SkillDraft) -> Reply<Skill> {
    let scope = skill_scope(state, scope)?;
    let clean = crate::domain::skill::Clean::new(&draft.name, &draft.description, &draft.body)
        .map_err(crate::skills::SkillsError::from)?;
    let skills = state.runtime.skills();
    let previous = draft.previous.filter(|previous| *previous != clean.name);
    let moved = match &previous {
        Some(previous) => skills.rename(scope, previous, &clean.name)?,
        None => false,
    };
    let written = skills.write(scope, &clean)?;
    if let Some(previous) = previous.filter(|_| !moved) {
        skills.delete(scope, &previous)?;
    }
    state.runtime.emit(UiEvent::SkillsChanged { scope });
    Ok(written)
}

pub async fn delete_skill(state: &AppState, scope: SkillScope, name: String) -> Reply<bool> {
    let scope = skill_scope(state, scope)?;
    let deleted = state.runtime.skills().delete(scope, &name)?;
    if deleted {
        state.runtime.emit(UiEvent::SkillsChanged { scope });
    }
    Ok(deleted)
}

pub async fn skill_directory(
    _state: &AppState,
    board: crate::skills_sh::Board,
    page: u32,
) -> Reply<crate::skills_sh::Page> {
    Ok(crate::skills_sh::Directory::new().board(board, page).await?)
}

pub async fn search_skill_directory(
    _state: &AppState,
    query: String,
) -> Reply<Vec<crate::skills_sh::Listing>> {
    Ok(crate::skills_sh::Directory::new().search(&query).await?)
}

pub async fn preview_directory_skill(
    _state: &AppState,
    id: String,
) -> Reply<crate::skills_sh::Preview> {
    Ok(crate::skills_sh::Directory::new().preview(&id).await?)
}

pub async fn add_directory_skill(
    state: &AppState,
    scope: SkillScope,
    id: String,
    hash: String,
) -> Reply<Skill> {
    let scope = skill_scope(state, scope)?;
    let directory = crate::skills_sh::Directory::new();
    let added = directory.add(state.runtime.skills(), scope, &id, &hash).await?;
    state.runtime.emit(UiEvent::SkillsChanged { scope });
    Ok(added)
}

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

pub async fn delete_agent(state: &AppState, id: AgentId) -> Reply<()> {
    let card = agent_card(state, id)?;
    state.runtime.discard_agent(&card).await?;
    state.runtime.emit(UiEvent::AgentsChanged);
    Ok(())
}

pub async fn restore_agent(state: &AppState, id: AgentId) -> Reply<AgentCard> {
    let card = agent_card(state, id)?;
    let restored = state.runtime.restore_agent(&card)?;
    state.runtime.emit(UiEvent::AgentsChanged);
    Ok(restored)
}

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

pub async fn set_agent_pinned(state: &AppState, id: AgentId, pinned: bool) -> Reply<AgentCard> {
    let card = state.runtime.store().set_agent_pinned(id, pinned)?;
    state.runtime.emit(UiEvent::AgentsChanged);
    Ok(card)
}

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

pub async fn duplicate_agent(state: &AppState, id: AgentId) -> Reply<AgentCard> {
    let original = agent_card(state, id)?;
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
        subscription_model: Some(original.subscription_model),
        reasoning_effort: original.reasoning_effort,
        system_prompt: original.system_prompt,
        skills: original.skills,
    };
    let card = state.runtime.store().create_agent(&draft.validate()?)?;
    state.runtime.start_agent(card.id);
    state.runtime.emit(UiEvent::AgentsChanged);
    Ok(card)
}

pub async fn hire_agents(
    state: &AppState,
    group_id: GroupId,
    drafts: Vec<AgentDraft>,
    idempotency_key: Option<String>,
) -> Reply<Vec<AgentCard>> {
    if drafts.is_empty() {
        return Ok(Vec::new());
    }

    let store = state.runtime.store();
    let fingerprint = idempotency_key.as_ref().map(|_| {
        use sha2::{Digest, Sha256};
        format!("{:x}", Sha256::digest(format!("{group_id}:{drafts:?}").as_bytes()))
    });
    if let Some(key) = &idempotency_key {
        if key.is_empty() || key.len() > 128 || drafts.len() != 1 {
            return Err(CommandError::new(
                "validation",
                "A hire key requires one agent and at most 128 characters.",
            ));
        }
    }
    if !store.list_groups()?.iter().any(|g| g.id == group_id) {
        return Err(CommandError::new(
            "notFound",
            "that group no longer exists. Close the cafeteria and pick another one.",
        ));
    }

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

    let hired = if let Some(key) = idempotency_key {
        let (card, _) =
            store.create_onboarding_agent(&key, fingerprint.as_deref().unwrap(), &clean[0])?;
        vec![card]
    } else {
        store.create_agents(&clean)?
    };
    for card in &hired {
        if card.lifecycle != Lifecycle::Terminated {
            state.runtime.start_agent(card.id);
        }
    }
    state.runtime.emit(UiEvent::AgentsChanged);
    Ok(hired)
}

pub async fn agent_activity(state: &AppState) -> Reply<HashMap<AgentId, Activity>> {
    Ok(state.runtime.activity_snapshot())
}

pub async fn agent_memory(state: &AppState, id: AgentId) -> Reply<String> {
    Ok(state.runtime.workspace().read(id))
}

pub async fn agent_memory_versioned(
    state: &AppState,
    id: AgentId,
) -> Reply<crate::workspace::MemorySnapshot> {
    Ok(state.runtime.workspace().read_versioned(id))
}

pub async fn set_agent_memory(
    state: &AppState,
    id: AgentId,
    content: String,
    expected: Option<String>,
) -> Reply<String> {
    let card = state
        .runtime
        .store()
        .get_agent(id)?
        .ok_or_else(|| CommandError::new("notFound", format!("no agent with id {id}")))?;
    let result = state
        .runtime
        .workspace()
        .write_versioned(id, &card.name, &content, expected.as_deref())
        .map_err(|err| CommandError::new("storage", err.to_string()))?;
    match result {
        crate::workspace::WriteResult::Stored(stored) => Ok(stored.content),
        crate::workspace::WriteResult::Conflict { current, draft } => {
            let version = crate::workspace::version(&current);
            Err(CommandError::new("memoryConflict", "Memory changed while you were editing it.")
                .with_detail(
                    serde_json::json!({ "current": current, "draft": draft, "version": version })
                        .to_string(),
                ))
        }
    }
}

pub async fn agent_notebook(state: &AppState, id: AgentId) -> Reply<Vec<crate::notebook::Entry>> {
    Ok(state.runtime.notebooks().list(id))
}

pub async fn read_notebook(state: &AppState, id: AgentId, path: String) -> Reply<String> {
    Ok(state.runtime.notebooks().read(id, &path)?)
}

pub async fn delete_notebook_file(state: &AppState, id: AgentId, path: String) -> Reply<bool> {
    let deleted = state.runtime.notebooks().delete(id, &path)?;
    if deleted {
        state.runtime.emit(UiEvent::NotebookChanged { agent_id: id });
    }
    Ok(deleted)
}

pub async fn agent_working_notes(state: &AppState, id: AgentId) -> Reply<Vec<WorkingNote>> {
    Ok(state.runtime.store().working_notes(id)?)
}

pub async fn clear_agent_working_notes(state: &AppState, id: AgentId) -> Reply<()> {
    state.runtime.store().clear_working_notes(id)?;
    Ok(())
}

pub async fn agent_last_active(state: &AppState) -> Reply<HashMap<AgentId, i64>> {
    Ok(state.runtime.store().last_activity()?)
}

const MAX_CHANNEL_WINDOW: u32 = 1000;

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

pub async fn pair_messages(
    state: &AppState,
    a: AgentId,
    b: AgentId,
    limit: Option<u32>,
) -> Reply<Vec<Envelope>> {
    Ok(state.runtime.store().pair_messages(a, b, limit.unwrap_or(200).min(1000))?)
}

pub async fn conversation_flow(
    state: &AppState,
    group: GroupId,
    limit: Option<u32>,
) -> Reply<Vec<Envelope>> {
    Ok(state.runtime.store().conversation_flow(group, limit.unwrap_or(400).min(2000))?)
}

pub async fn search(state: &AppState, query: String, limit: Option<u32>) -> Reply<SearchHits> {
    Ok(state.runtime.store().search(query.trim(), limit.unwrap_or(20).min(100))?)
}

#[derive(Debug, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Staged {
    pub attached: Vec<Attachment>,
    pub refused: Vec<String>,
}

pub fn screen_ticket(secret: &str, sandbox: &str) -> String {
    use sha2::Digest;
    format!("{:x}", sha2::Sha256::digest(format!("{secret}:{sandbox}").as_bytes()))
}

pub const SCREEN_ROUTE: &str = "/v1/screen";

impl AppState {
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

pub async fn forward_files(
    state: &AppState,
    origin: String,
    token: String,
    paths: Vec<String>,
) -> Reply<Staged> {
    state.deployment.capabilities().require(Absent::LocalFiles)?;

    upload_local_files(origin, token, paths).await
}

pub async fn upload_local_files(
    origin: String,
    token: String,
    paths: Vec<String>,
) -> Reply<Staged> {
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

pub async fn report_presence(
    state: &AppState,
    presence: Option<crate::menubar::Presence>,
) -> Reply<()> {
    (state.menubar)(presence);
    Ok(())
}

pub async fn stop_everything(state: &AppState) -> Reply<usize> {
    Ok(state.runtime.stop_everything())
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FileRef {
    pub digest: String,
    pub name: String,
}

#[derive(Debug, Serialize)]
#[serde(untagged)]
pub enum SentMessage {
    Legacy(RunId),
    Keyed {
        #[serde(rename = "messageId")]
        message_id: MessageId,
        #[serde(rename = "runId")]
        run_id: RunId,
    },
}

pub async fn send_message(
    state: &AppState,
    agent_id: AgentId,
    text: String,
    files: Option<Vec<FileRef>>,
    idempotency_key: Option<String>,
) -> Reply<SentMessage> {
    let trimmed = text.trim();
    let files = files.unwrap_or_default();
    if trimmed.is_empty() && files.is_empty() {
        return Err(CommandError::new("validation", "message must not be empty"));
    }

    let fingerprint = if let Some(key) = &idempotency_key {
        if key.is_empty() || key.len() > 128 {
            return Err(CommandError::new(
                "validation",
                "A message key must have between 1 and 128 characters.",
            ));
        }
        use sha2::{Digest, Sha256};
        let fingerprint =
            format!("{:x}", Sha256::digest(format!("{agent_id}:{trimmed}:{files:?}").as_bytes()));
        if let Some((message_id, run_id)) =
            state.runtime.store().operator_send_receipt(key, &fingerprint)?
        {
            return Ok(SentMessage::Keyed { message_id, run_id });
        }
        Some(fingerprint)
    } else {
        None
    };

    let mut attached = Vec::new();
    for file in &files {
        match state.runtime.files().reference(&file.digest, &file.name) {
            Ok(file) => attached.push(file),
            Err(err) => return Err(CommandError::new("file", err.to_string())),
        }
    }
    match idempotency_key {
        Some(key) => {
            let (message_id, run_id) = state.runtime.send_from_human_keyed(
                agent_id,
                trimmed,
                attached,
                &key,
                fingerprint.as_deref().unwrap(),
            )?;
            Ok(SentMessage::Keyed { message_id, run_id })
        }
        None => Ok(SentMessage::Legacy(
            state.runtime.send_from_human_with(agent_id, trimmed, attached)?,
        )),
    }
}

pub async fn save_file(state: &AppState, digest: String, name: String) -> Reply<String> {
    state.deployment.capabilities().require(Absent::LocalFiles)?;

    let saved = state
        .runtime
        .files()
        .save_copy(&digest, &name, &state.downloads)
        .map_err(|err| CommandError::new("file", err.to_string()))?;
    Ok(saved.display().to_string())
}

pub async fn frame_artifact(state: &AppState, html: String) -> Reply<ArtifactAddress> {
    let id = state
        .artifacts
        .keep(&html)
        .map_err(|err| CommandError::new("artifact", err.to_string()))?;
    let ticket = state.secret.as_deref().map(|secret| artifact_ticket(secret, &id));
    Ok(ArtifactAddress {
        port: state.artifact_port.load(std::sync::atomic::Ordering::SeqCst),
        id,
        ticket,
    })
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ArtifactAddress {
    pub port: u16,
    pub id: String,
    pub ticket: Option<String>,
}

pub fn artifact_ticket(secret: &str, id: &str) -> String {
    screen_ticket(secret, &format!("artifact:{id}"))
}

pub async fn retry_turn(
    state: &AppState,
    agent_id: AgentId,
    message_id: MessageId,
) -> Reply<RunId> {
    Ok(state.runtime.retry_turn(agent_id, message_id)?)
}

pub async fn stop_run(state: &AppState, run_id: RunId) -> Reply<bool> {
    Ok(state.runtime.stop_run(run_id))
}

pub async fn clear_channel(state: &AppState, channel_id: AgentId) -> Reply<usize> {
    Ok(state.runtime.store().delete_channel_messages(channel_id)?)
}

pub async fn agent_routines(state: &AppState, id: AgentId) -> Reply<Vec<Routine>> {
    Ok(state.runtime.store().agent_routines(id)?)
}

pub async fn upcoming_runs(state: &AppState, until: i64) -> Reply<Vec<UpcomingRun>> {
    Ok(state.runtime.store().upcoming_runs(until)?)
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RoutineDraft {
    #[serde(default)]
    pub name: String,
    pub what: String,
    pub trigger: String,
    pub in_secs: Option<u32>,
    #[serde(default)]
    pub skip_if_working: bool,
}

impl RoutineDraft {
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

pub async fn set_routine_active(state: &AppState, id: RoutineId, active: bool) -> Reply<Routine> {
    let routine = state.runtime.store().set_routine_active(id, active)?;
    state.runtime.emit(UiEvent::RoutinesChanged { agent_id: routine.agent_id });
    Ok(routine)
}

pub async fn test_routine(state: &AppState, id: RoutineId) -> Reply<RunId> {
    let routine = state
        .runtime
        .store()
        .get_routine(id)?
        .ok_or_else(|| CommandError::new("notFound", format!("no routine with id {id}")))?;
    Ok(state.runtime.test_routine(&routine)?)
}

pub async fn webhook_address(state: &AppState) -> Reply<WebhookAddress> {
    Ok(WebhookAddress {
        url: state.reach.origin().map(|origin| format!("{}/events", origin.trim_end_matches('/'))),
        secret: state.runtime.config().webhook.secret,
    })
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WebhookAddress {
    pub url: Option<String>,
    pub secret: String,
}

pub async fn routine_runs(state: &AppState, id: RoutineId) -> Reply<Vec<RoutineRun>> {
    Ok(state.runtime.store().routine_runs(id, 20)?)
}

pub async fn delete_routine(state: &AppState, id: RoutineId) -> Reply<()> {
    let whose = state.runtime.store().get_routine(id)?.map(|routine| routine.agent_id);
    state.runtime.store().delete_routine(id)?;
    if let Some(agent_id) = whose {
        state.runtime.emit(UiEvent::RoutinesChanged { agent_id });
    }
    Ok(())
}

pub async fn usage_for_runs(state: &AppState, runs: Vec<RunId>) -> Reply<Vec<RunUsage>> {
    Ok(state
        .runtime
        .store()
        .usage_by_run(&runs)?
        .into_iter()
        .map(|(run_id, tokens)| RunUsage { run_id, tokens })
        .collect())
}

pub async fn usage_summary(state: &AppState) -> Reply<Vec<GroupUsage>> {
    Ok(state
        .runtime
        .store()
        .usage_by_group()?
        .into_iter()
        .map(|(group_id, tokens)| GroupUsage { group_id, tokens })
        .collect())
}

pub async fn clear_group(state: &AppState, group_id: GroupId) -> Reply<GroupReset> {
    let store = state.runtime.store();
    let agents = store.group_agent_ids(group_id)?;

    let reset = GroupReset {
        messages: store.delete_group_messages(group_id)?,
        routines: store.delete_group_routines(group_id)?,
        calls: store.delete_group_usage(group_id)?,
        notes: agents
            .iter()
            .filter(|id| !state.runtime.workspace().read(**id).trim().is_empty())
            .inspect(|id| state.runtime.workspace().remove(**id))
            .count(),
        working_notes: store.delete_group_working_notes(group_id)?,
    };

    state.runtime.emit(UiEvent::ChannelsCleared { agents });
    state.runtime.emit(UiEvent::AgentsChanged);
    Ok(reset)
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GroupReset {
    pub messages: usize,
    pub routines: usize,
    pub notes: usize,
    pub working_notes: usize,
    pub calls: usize,
}

pub use crate::config::SettingsPatch;

pub async fn capabilities(state: &AppState) -> Reply<Capabilities> {
    Ok(state.deployment.capabilities())
}

pub async fn get_settings(state: &AppState) -> Reply<RedactedConfig> {
    Ok(state.runtime.config().redacted())
}

pub async fn account_status(state: &AppState) -> Reply<crate::account::Status> {
    Ok(state.account.checked_status().await)
}

pub async fn sign_in_account(state: &AppState) -> Reply<crate::account::Status> {
    let account = state.account.clone();
    let open = state.open_url.clone();
    if account.has_broker() {
        return Ok(account.open_broker(|url| open(url)).await?);
    }
    let landing = state.reach.landing()?;
    Ok(account.sign_in(|url| open(url), &landing).await?)
}

pub async fn account_connectors(state: &AppState) -> Reply<Connectors> {
    let account = state.account.clone();
    Ok(account.connectors().await?)
}

pub async fn sign_out_account(state: &AppState) -> Reply<crate::account::Status> {
    state.account.sign_out()?;
    Ok(state.account.status())
}

pub async fn subscription_status(state: &AppState) -> Reply<Status> {
    Ok(state.subscription.status())
}

pub async fn subscription_models(
    state: &AppState,
) -> Reply<Vec<crate::llm::codex::SubscriptionModel>> {
    crate::llm::codex::models(&state.subscription).await.map_err(|err| {
        tracing::warn!(%err, "could not load ChatGPT model catalog");
        CommandError::new("subscriptionModels", err.to_string())
    })
}

pub async fn begin_subscription_signin(state: &AppState) -> Reply<DeviceCode> {
    Ok(state.subscription.begin().await?)
}

pub async fn complete_subscription_signin(state: &AppState, code: DeviceCode) -> Reply<Status> {
    Ok(state.subscription.complete(&code).await?)
}

pub async fn sign_out_subscription(state: &AppState) -> Reply<RedactedConfig> {
    state.subscription.sign_out()?;

    if state.runtime.config().inference.provider != config::Provider::Chatgpt {
        return Ok(state.runtime.config().redacted());
    }
    let config = state.runtime.change_config(|config| {
        if config.inference.provider == config::Provider::Chatgpt {
            config.inference.provider = config::Provider::Compatible;
        }
        Ok::<_, CommandError>(())
    })?;
    Ok(config.redacted())
}

fn apply_patch(
    here: Capabilities,
    config: &mut AppConfig,
    patch: SettingsPatch,
) -> Result<(), CommandError> {
    honorable(here, patch.provider, patch.base_url.as_deref())?;
    Ok(config.apply(patch)?)
}

pub async fn update_settings(state: &AppState, patch: SettingsPatch) -> Reply<RedactedConfig> {
    let here = state.deployment.capabilities();
    let config = state.runtime.change_config(|config| apply_patch(here, config, patch))?;
    Ok(config.redacted())
}

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

pub async fn ranked_models(state: &AppState, category: String) -> Reply<Vec<RankedModel>> {
    Ok(state.catalog.ranked(&category).await?)
}

pub async fn export_group(state: &AppState, id: GroupId) -> Reply<crate::transfer::Archive> {
    let runtime = state.runtime.clone();
    tokio::task::spawn_blocking(move || {
        let mut conn = runtime.store().conn()?;
        crate::transfer::export(&mut conn, id, runtime.workspace(), runtime.files())
            .map_err(|e| CommandError::new("export", e))
    })
    .await
    .map_err(|e| CommandError::new("export", e.to_string()))?
}

pub async fn import_group(
    state: &AppState,
    archive: crate::transfer::Archive,
    name: String,
) -> Reply<Group> {
    let runtime = state.runtime.clone();
    tokio::task::spawn_blocking(move || {
        let id = crate::transfer::import(
            runtime.store(),
            archive,
            name,
            runtime.workspace(),
            runtime.files(),
        )
        .map_err(|e| CommandError::new("import", e))?;
        for agent in runtime.store().list_agents()?.into_iter().filter(|a| {
            a.group_id == id && a.lifecycle != crate::domain::agent::Lifecycle::Terminated
        }) {
            runtime.start_agent(agent.id);
        }
        runtime.emit(UiEvent::AgentsChanged);
        runtime
            .store()
            .get_group(id)?
            .ok_or_else(|| CommandError::new("import", "The imported group was not found."))
    })
    .await
    .map_err(|e| CommandError::new("import", e.to_string()))?
}

pub async fn group_reconnect(
    state: &AppState,
    id: GroupId,
) -> Reply<Vec<crate::transfer::Reconnect>> {
    crate::transfer::reconnect(state.runtime.store(), id)
        .map_err(|e| CommandError::new("import", e))
}

pub async fn list_decisions(state: &AppState) -> Reply<Vec<WorkDecision>> {
    Ok(state.runtime.store().decisions(None)?)
}

pub async fn list_outcomes(
    state: &AppState,
    since: Option<i64>,
    group_id: Option<GroupId>,
    unseen_only: Option<bool>,
    limit: u32,
) -> Reply<Vec<crate::domain::outcome::Outcome>> {
    Ok(state.runtime.store().outcomes(since, group_id, unseen_only.unwrap_or(false), limit)?)
}

pub async fn mark_outcomes_seen(
    state: &AppState,
    run_ids: Option<Vec<RunId>>,
    before: Option<i64>,
) -> Reply<Vec<RunId>> {
    if run_ids.is_none() && before.is_none() {
        return Err(CommandError::new(
            "validation",
            "Name the outcomes to mark seen, or the time to mark them up to.",
        ));
    }
    Ok(state.runtime.mark_outcomes_seen(run_ids.as_deref(), before)?)
}

pub async fn pending_deliveries(
    state: &AppState,
    client: Option<String>,
) -> Reply<Vec<crate::domain::outbox::Delivery>> {
    let page = crate::domain::view::window_name(client);
    Ok(state.runtime.pending_deliveries(page.as_deref())?)
}

pub async fn claim_delivery(state: &AppState, id: String, client: Option<String>) -> Reply<bool> {
    let page = crate::domain::view::window_name(client);
    Ok(state.runtime.claim_delivery(&id, page.as_deref())?)
}

pub async fn release_delivery(state: &AppState, id: String, client: String) -> Reply<bool> {
    let Some(page) = crate::domain::view::window_name(Some(client)) else {
        return Ok(false);
    };
    Ok(state.runtime.release_delivery(&id, &page)?)
}

pub async fn push_report(state: &AppState) -> Reply<crate::push::Report> {
    Ok(state.runtime.push_report()?)
}

pub async fn test_push(state: &AppState) -> Reply<String> {
    state.runtime.test_push().await.map_err(|why| CommandError::new("delivery", why))
}
pub async fn answer_decision(
    state: &AppState,
    id: DecisionId,
    answer: String,
    updated_at: i64,
) -> Reply<WorkDecision> {
    Ok(state.runtime.answer_decision(id, &answer, false, Some(updated_at))?)
}
pub async fn resume_decision(state: &AppState, id: DecisionId) -> Reply<WorkDecision> {
    Ok(state.runtime.answer_decision(id, "", true, None)?)
}
pub async fn snooze_decision(state: &AppState, id: DecisionId, until: i64) -> Reply<()> {
    Ok(state.runtime.snooze_decision(id, until)?)
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
