//! The IPC surface.
//!
//! Everything the webview can ask for, and nothing else. Note what is absent:
//! there is no command that returns the API key, and no command that performs
//! network access on the frontend's behalf with a caller-supplied URL. The
//! webview never holds a credential.

use std::collections::HashMap;
use std::path::PathBuf;

use serde::{Deserialize, Serialize};
use tauri::State;

use crate::config::{self, AppConfig, RedactedConfig};
use crate::domain::agent::{AgentCard, AgentDraft, Lifecycle};
use crate::domain::envelope::Envelope;
use crate::domain::group::{Group, GroupDraft};
use crate::domain::ids::{AgentId, GroupId, RunId};
use crate::e2b::{Computer, E2bClient, E2bError};
use crate::runtime::events::{Activity, UiEvent};
use crate::runtime::guard::GuardLimits;
use crate::runtime::Runtime;

pub struct AppState {
    pub runtime: Runtime,
    pub config_path: PathBuf,
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
    fn new(kind: &'static str, message: impl Into<String>) -> Self {
        Self { kind, message: message.into() }
    }
}

impl From<crate::db::StoreError> for CommandError {
    fn from(err: crate::db::StoreError) -> Self {
        use crate::db::StoreError;
        match err {
            StoreError::DuplicateName(_) => CommandError::new("duplicateName", err.to_string()),
            StoreError::AgentNotFound(_) | StoreError::GroupNotFound(_) => {
                CommandError::new("notFound", err.to_string())
            }
            // Its own kind: the UI can offer to move the agents, which it
            // cannot do for a generic storage failure.
            StoreError::GroupNotEmpty { .. } => CommandError::new("groupNotEmpty", err.to_string()),
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

impl From<config::ConfigError> for CommandError {
    fn from(err: config::ConfigError) -> Self {
        CommandError::new("config", err.to_string())
    }
}

type Reply<T> = Result<T, CommandError>;

// ---- computers -----------------------------------------------------------

/// The E2B client, or a clear reason there is not one.
fn computers(state: &State<'_, AppState>) -> Reply<E2bClient> {
    E2bClient::new(&state.runtime.config().e2b.api_key).ok_or_else(|| E2bError::NoKey.into())
}

fn agent_card(state: &State<'_, AppState>, id: AgentId) -> Reply<crate::domain::agent::AgentCard> {
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
#[tauri::command]
pub async fn agent_computer(state: State<'_, AppState>, id: AgentId) -> Reply<Option<Computer>> {
    let card = agent_card(&state, id)?;
    let (Some(sandbox), Some(envd)) = (card.sandbox_id, card.sandbox_envd_token) else {
        return Ok(None);
    };
    let client = computers(&state)?;

    if !client.is_alive(&sandbox).await? {
        // E2B reclaims idle sandboxes, and a reclaimed one leaves a dangling
        // id. Clearing it turns a dead end into an offer to make a new one.
        state.runtime.store().set_agent_sandbox(id, None)?;
        return Ok(None);
    }
    Ok(Some(client.describe(&sandbox, &envd, state.runtime.viewer_port()).await?))
}

/// Gives an agent a computer, or brings the desktop up on the one it has.
#[tauri::command]
pub async fn start_agent_computer(state: State<'_, AppState>, id: AgentId) -> Reply<Computer> {
    let card = agent_card(&state, id)?;
    let (client, sandbox) = state.runtime.ensure_computer(&card).await?;

    client.start_desktop(&sandbox.id, &sandbox.envd_token).await?;
    let computer =
        client.describe(&sandbox.id, &sandbox.envd_token, state.runtime.viewer_port()).await?;
    state.runtime.emit(UiEvent::AgentsChanged);
    Ok(computer)
}

/// Destroys the sandbox and everything on its disk.
#[tauri::command]
pub async fn delete_agent_computer(state: State<'_, AppState>, id: AgentId) -> Reply<()> {
    let Some(sandbox) = agent_card(&state, id)?.sandbox_id else {
        return Ok(());
    };
    computers(&state)?.kill(&sandbox).await?;
    state.runtime.store().set_agent_sandbox(id, None)?;
    state.runtime.emit(UiEvent::AgentsChanged);
    Ok(())
}

// ---- groups --------------------------------------------------------------

#[tauri::command]
pub fn list_groups(state: State<'_, AppState>) -> Reply<Vec<Group>> {
    Ok(state.runtime.store().list_groups()?)
}

#[tauri::command]
pub fn create_group(state: State<'_, AppState>, draft: GroupDraft) -> Reply<Group> {
    let clean = draft.validate()?;
    let group = state.runtime.store().create_group(&clean)?;
    state.runtime.emit(UiEvent::AgentsChanged);
    Ok(group)
}

#[tauri::command]
pub fn update_group(state: State<'_, AppState>, id: GroupId, draft: GroupDraft) -> Reply<Group> {
    let clean = draft.validate()?;
    let group = state.runtime.store().update_group(id, &clean)?;
    state.runtime.emit(UiEvent::AgentsChanged);
    Ok(group)
}

/// Deletes an empty group. Refused while it still holds agents; see
/// `Store::delete_group` for why they are not relocated.
#[tauri::command]
pub fn delete_group(state: State<'_, AppState>, id: GroupId) -> Reply<()> {
    state.runtime.store().delete_group(id)?;
    state.runtime.emit(UiEvent::AgentsChanged);
    Ok(())
}

// ---- agents --------------------------------------------------------------

#[tauri::command]
pub fn list_agents(state: State<'_, AppState>) -> Reply<Vec<AgentCard>> {
    Ok(state.runtime.store().list_agents()?)
}

#[tauri::command]
pub fn create_agent(state: State<'_, AppState>, draft: AgentDraft) -> Reply<AgentCard> {
    let clean = draft.validate()?;
    let card = state.runtime.store().create_agent(&clean)?;
    state.runtime.start_agent(card.id);
    state.runtime.emit(UiEvent::AgentsChanged);
    Ok(card)
}

#[tauri::command]
pub fn update_agent(
    state: State<'_, AppState>,
    id: AgentId,
    draft: AgentDraft,
) -> Reply<AgentCard> {
    let clean = draft.validate()?;
    let card = state.runtime.store().update_agent(id, &clean)?;
    state.runtime.emit(UiEvent::AgentsChanged);
    Ok(card)
}

/// Deletes an agent.
///
/// A soft delete: the agent leaves the sidebar and the directory immediately
/// and can never be messaged again, but what it already said stays readable in
/// the other agents' channels. Hard-deleting would punch holes in transcripts
/// that had nothing to do with this agent.
#[tauri::command]
pub fn delete_agent(state: State<'_, AppState>, id: AgentId) -> Reply<()> {
    state.runtime.store().set_lifecycle(id, Lifecycle::Terminated)?;
    state.runtime.stop_agent(id);
    // The transcript survives a deletion, but the agent's private notes are
    // its own and go with it.
    state.runtime.workspace().remove(id);
    state.runtime.emit(UiEvent::AgentsChanged);
    Ok(())
}

#[tauri::command]
pub fn set_agent_paused(state: State<'_, AppState>, id: AgentId, paused: bool) -> Reply<AgentCard> {
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

#[tauri::command]
pub fn agent_activity(state: State<'_, AppState>) -> Reply<HashMap<AgentId, Activity>> {
    Ok(state.runtime.activity_snapshot())
}

/// Newest message timestamp per agent, used to order the sidebar by who spoke
/// most recently. Live updates come from message events; this seeds them.
/// An agent's notes: a small markdown file it maintains for itself.
#[tauri::command]
pub fn agent_notes(state: State<'_, AppState>, id: AgentId) -> Reply<String> {
    Ok(state.runtime.workspace().read(id))
}

/// Lets the operator seed or correct an agent's notes by hand.
#[tauri::command]
pub fn set_agent_notes(state: State<'_, AppState>, id: AgentId, content: String) -> Reply<String> {
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

#[tauri::command]
pub fn agent_last_active(state: State<'_, AppState>) -> Reply<HashMap<AgentId, i64>> {
    Ok(state.runtime.store().last_activity()?)
}

// ---- messages ------------------------------------------------------------

#[tauri::command]
pub fn channel_messages(
    state: State<'_, AppState>,
    channel_id: AgentId,
    limit: Option<u32>,
) -> Reply<Vec<Envelope>> {
    Ok(state.runtime.store().channel_messages(channel_id, limit.unwrap_or(300).min(1000))?)
}

/// The whole conversation, for the flow board.
#[tauri::command]
pub fn conversation_flow(state: State<'_, AppState>, limit: Option<u32>) -> Reply<Vec<Envelope>> {
    Ok(state.runtime.store().conversation_flow(limit.unwrap_or(400).min(2000))?)
}

#[tauri::command]
pub fn send_message(state: State<'_, AppState>, agent_id: AgentId, text: String) -> Reply<RunId> {
    let trimmed = text.trim();
    if trimmed.is_empty() {
        return Err(CommandError::new("validation", "message must not be empty"));
    }
    Ok(state.runtime.send_from_human(agent_id, trimmed)?)
}

#[tauri::command]
pub fn clear_channel(state: State<'_, AppState>, channel_id: AgentId) -> Reply<usize> {
    Ok(state.runtime.store().delete_channel_messages(channel_id)?)
}

// ---- settings ------------------------------------------------------------

/// Absent fields are left alone. `apiKey: ""` clears the key; omitting it
/// keeps the existing one, which is what lets the UI show a redacted value
/// without ever round-tripping the secret.
#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct SettingsPatch {
    pub base_url: Option<String>,
    pub api_key: Option<String>,
    pub default_model: Option<String>,
    pub request_timeout_secs: Option<u64>,
    pub limits: Option<GuardLimits>,
    pub e2b_api_key: Option<String>,
}

#[tauri::command]
pub fn get_settings(state: State<'_, AppState>) -> Reply<RedactedConfig> {
    Ok(state.runtime.config().redacted())
}

/// Applies a patch to a config in memory. Shared by saving and testing, so a
/// tested configuration and a saved one can never diverge.
fn apply_patch(config: &mut AppConfig, patch: SettingsPatch) -> Result<(), CommandError> {
    if let Some(base_url) = patch.base_url {
        config.inference.base_url = config::normalize_base_url(&base_url)?;
    }
    if let Some(key) = patch.e2b_api_key {
        config.e2b.api_key = key.trim().to_string();
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
    if let Some(timeout) = patch.request_timeout_secs {
        config.inference.request_timeout_secs = timeout.clamp(5, 900);
    }
    if let Some(limits) = patch.limits {
        config.limits = limits.sanitized();
    }
    Ok(())
}

#[tauri::command]
pub fn update_settings(state: State<'_, AppState>, patch: SettingsPatch) -> Reply<RedactedConfig> {
    let mut config: AppConfig = state.runtime.config();
    apply_patch(&mut config, patch)?;
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
#[tauri::command]
pub async fn test_connection(
    state: State<'_, AppState>,
    patch: Option<SettingsPatch>,
) -> Reply<String> {
    let mut config = state.runtime.config();
    if let Some(patch) = patch {
        apply_patch(&mut config, patch)?;
    }
    state
        .runtime
        .probe(&config)
        .await
        .map_err(|err| CommandError::new("inference", err.to_string()))
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
