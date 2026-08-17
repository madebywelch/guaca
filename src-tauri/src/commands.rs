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

use crate::computer::{Computer, ComputerError};
use crate::config::{self, AppConfig, RedactedConfig};
use crate::domain::agent::{copy_name, AgentCard, AgentDraft, Lifecycle};
use crate::domain::approval::{Approval, ApprovalState, Decision, ProtectedAction};
use crate::domain::computer::ProviderChoice;
use crate::domain::connector::{Connector, ConnectorDraft};
use crate::domain::envelope::Envelope;
use crate::domain::group::{Group, GroupDraft};
use crate::domain::ids::{AgentId, ApprovalId, ConnectorId, GroupId, MessageId, RoutineId, RunId};
use crate::domain::now_ms;
use crate::domain::routine::{self, Routine, RoutineRun, Trigger};
use crate::domain::search::SearchHits;
use crate::domain::signin::Signin;
use crate::domain::usage::{GroupUsage, RunUsage};
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
            StoreError::AgentNotFound(_)
            | StoreError::ComputerNotFound(_)
            | StoreError::GroupNotFound(_)
            | StoreError::ApprovalNotFound(_)
            | StoreError::ConnectorNotFound(_) => CommandError::new("notFound", err.to_string()),
            // Its own kind: a request answered twice, or answered after it
            // lapsed, is a stale button rather than anything being wrong. The
            // UI redraws it instead of showing a failure.
            StoreError::ApprovalSettled { .. } => {
                CommandError::new("alreadyAnswered", err.to_string())
            }
            // An operator naming a variable twice is a mistake they can fix,
            // not a disk failure, and the message already says which name.
            StoreError::DuplicateEnvVar(_) => CommandError::new("validation", err.to_string()),
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
            RuntimeError::Computer(inner) => inner.into(),
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

impl From<ComputerError> for CommandError {
    fn from(err: ComputerError) -> Self {
        // Its own kind so the UI can offer to open settings rather than
        // showing a failure for something that was simply never set up.
        let code = match err {
            ComputerError::Unconfigured(_) => "computerUnconfigured",
            _ => "computer",
        };
        CommandError::new(code, err.to_string())
    }
}

impl From<config::ConfigError> for CommandError {
    fn from(err: config::ConfigError) -> Self {
        CommandError::new("config", err.to_string())
    }
}

type Reply<T> = Result<T, CommandError>;

// ---- computers -----------------------------------------------------------

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
    Ok(state.runtime.computers().describe(id).await?)
}

/// Gives an agent a computer, or brings the desktop up on the one it has.
#[tauri::command]
pub async fn start_agent_computer(state: State<'_, AppState>, id: AgentId) -> Reply<Computer> {
    let card = agent_card(&state, id)?;
    let machine = state.runtime.ensure_computer(&card).await?;

    machine.start_desktop().await.map_err(ComputerError::from)?;
    // Asked again rather than assumed: what the pane draws is where the desktop
    // is actually serving, and that is only true once it answers.
    let computer = state.runtime.computers().describe(id).await?.ok_or_else(|| {
        CommandError::new("computer", "the computer was started but cannot be found; try again")
    })?;
    state.runtime.emit(UiEvent::AgentsChanged);
    Ok(computer)
}

/// Puts an agent's machine to sleep.
///
/// Not a delete: the disk is kept, so a browser that was signed in still is
/// when it wakes. This is what a bill-conscious operator wants, and what the
/// idle timeout does on its own.
#[tauri::command]
pub async fn stop_agent_computer(
    state: State<'_, AppState>,
    id: AgentId,
) -> Reply<Option<Computer>> {
    let shown = state.runtime.computers().sleep(id).await?;
    state.runtime.emit(UiEvent::AgentsChanged);
    Ok(shown)
}

/// Destroys the machine and everything on its disk.
#[tauri::command]
pub async fn delete_agent_computer(state: State<'_, AppState>, id: AgentId) -> Reply<()> {
    state.runtime.computers().destroy(id).await?;
    state.runtime.emit(UiEvent::AgentsChanged);
    Ok(())
}

// ---- connectors ----------------------------------------------------------

/// Every account one crew can reach. Secrets are reported as set or not; the
/// values are not in this type and there is no command that returns them.
#[tauri::command]
pub fn group_connectors(state: State<'_, AppState>, group_id: GroupId) -> Reply<Vec<Connector>> {
    Ok(state.runtime.store().group_connectors(group_id)?)
}

#[tauri::command]
pub fn create_connector(state: State<'_, AppState>, draft: ConnectorDraft) -> Reply<Connector> {
    let clean = draft.validate()?;
    let connector = state.runtime.store().create_connector(&clean)?;
    // The roster every agent is shown includes what its peers can reach, so a
    // new account changes what the whole crew knows.
    state.runtime.emit(UiEvent::AgentsChanged);
    Ok(connector)
}

#[tauri::command]
pub fn delete_connector(state: State<'_, AppState>, id: ConnectorId) -> Reply<()> {
    state.runtime.store().delete_connector(id)?;
    state.runtime.emit(UiEvent::AgentsChanged);
    Ok(())
}

/// What one agent's browser turns out to be signed in to, right now.
///
/// Detected rather than declared: the browser is holding the cookies, so Guaca
/// asks the machine instead of asking the operator to keep a list up to date.
/// Called when the operator opens an agent, and by the runtime after an agent
/// has been browsing, which between them covers both ways a session appears.
#[tauri::command]
pub async fn scan_agent_signins(state: State<'_, AppState>, id: AgentId) -> Reply<Vec<Signin>> {
    Ok(state.runtime.scan_signins(id).await?)
}

/// The last scan's result, without touching the machine.
///
/// Separate from the scan because the machine may be asleep, and what it was
/// signed in to yesterday is still the best answer available. Waking a sandbox
/// to redraw a list would also cost money on every render.
#[tauri::command]
pub fn agent_signins(state: State<'_, AppState>, id: AgentId) -> Reply<Vec<Signin>> {
    Ok(state.runtime.store().agent_signins(id)?)
}

// ---- permission requests -------------------------------------------------

/// What every recent request came to, keyed by id.
///
/// The requests themselves travel in the transcript, so this is only the half
/// that changes: whether the buttons on a request already in a channel are
/// still live, and what was decided if they are not.
#[tauri::command]
pub fn approval_states(state: State<'_, AppState>) -> Reply<HashMap<ApprovalId, ApprovalState>> {
    Ok(state.runtime.store().approval_states(500)?)
}

/// What this agent no longer has to ask about.
#[tauri::command]
pub fn agent_grants(state: State<'_, AppState>, id: AgentId) -> Reply<Vec<ProtectedAction>> {
    Ok(state.runtime.store().standing_grants(id)?)
}

/// Takes one back. A permission that could only ever be given is a trap, and
/// "always" has to stay something the operator can change their mind about.
#[tauri::command]
pub fn revoke_grant(
    state: State<'_, AppState>,
    id: AgentId,
    action: ProtectedAction,
) -> Reply<Vec<ProtectedAction>> {
    state.runtime.store().revoke_grant(id, action)?;
    Ok(state.runtime.store().standing_grants(id)?)
}

/// Answers one. Refused if it was already answered or has expired, which is
/// what the operator's second click on a stale widget is.
#[tauri::command]
pub fn decide_approval(
    state: State<'_, AppState>,
    id: ApprovalId,
    decision: Decision,
) -> Reply<Approval> {
    Ok(state.runtime.decide_approval(id, decision)?)
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
pub async fn delete_agent(state: State<'_, AppState>, id: AgentId) -> Reply<()> {
    // Looked up first, so a bad id reads as "no such agent" rather than as a
    // deletion that quietly did nothing.
    agent_card(&state, id)?;
    // The machine goes next. A deleted agent cannot be asked to tidy up after
    // itself, and a machine nobody holds a reference to keeps billing. A
    // failure here is written down and retried at startup rather than left to
    // block the deletion.
    state.runtime.computers().release(id).await;

    state.runtime.store().set_lifecycle(id, Lifecycle::Terminated)?;
    state.runtime.stop_agent(id);
    // The transcript survives a deletion, but the agent's private memory is
    // its own and goes with it.
    state.runtime.workspace().remove(id);
    // Its schedule goes too, or it would keep coming due for an agent that can
    // no longer act on it.
    let _ = state.runtime.store().delete_agent_routines(id);
    // And what its browser was signed in to, which was cookies on the disk
    // destroyed above. Left behind, the roster would keep telling the crew to
    // ask this agent for an account nothing can reach any more.
    let _ = state.runtime.store().delete_agent_signins(id);
    // Permission the operator gave this agent dies with it. A name is free to
    // reuse the moment an agent is deleted, and whoever takes it next must not
    // inherit a standing grant given to somebody else.
    let _ = state.runtime.store().delete_agent_approvals(id);
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

/// Keeps an agent at the top of the rail.
///
/// Nothing about the agent changes: it is discoverable, addressable and billed
/// exactly as before, and no peer is told. The card version deliberately does
/// not move, because nothing a peer reads has.
#[tauri::command]
pub fn set_agent_pinned(state: State<'_, AppState>, id: AgentId, pinned: bool) -> Reply<AgentCard> {
    let card = state.runtime.store().set_agent_pinned(id, pinned)?;
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
#[tauri::command]
pub fn duplicate_agent(state: State<'_, AppState>, id: AgentId) -> Reply<AgentCard> {
    let original = agent_card(&state, id)?;
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

#[tauri::command]
pub fn agent_activity(state: State<'_, AppState>) -> Reply<HashMap<AgentId, Activity>> {
    Ok(state.runtime.activity_snapshot())
}

/// Newest message timestamp per agent, used to order the sidebar by who spoke
/// most recently. Live updates come from message events; this seeds them.
/// An agent's memory: a small markdown file it maintains for itself.
#[tauri::command]
pub fn agent_notes(state: State<'_, AppState>, id: AgentId) -> Reply<String> {
    Ok(state.runtime.workspace().read(id))
}

/// Lets the operator seed or correct an agent's memory by hand.
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

/// The most of a transcript any one read will put in the webview.
const MAX_CHANNEL_WINDOW: u32 = 1000;

/// A channel's newest messages.
///
/// `through` widens the window to reach one particular message, which is what
/// opening a search result needs: a hit from last month is not in the newest
/// three hundred, and a jump that lands somewhere else is a jump that failed.
/// A message that has since been cleared falls back to the plain newest window
/// rather than to an empty channel.
#[tauri::command]
pub fn channel_messages(
    state: State<'_, AppState>,
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
#[tauri::command]
pub fn pair_messages(
    state: State<'_, AppState>,
    a: AgentId,
    b: AgentId,
    limit: Option<u32>,
) -> Reply<Vec<Envelope>> {
    Ok(state.runtime.store().pair_messages(a, b, limit.unwrap_or(200).min(1000))?)
}

/// The whole conversation, for the flow board.
#[tauri::command]
pub fn conversation_flow(state: State<'_, AppState>, limit: Option<u32>) -> Reply<Vec<Envelope>> {
    Ok(state.runtime.store().conversation_flow(limit.unwrap_or(400).min(2000))?)
}

/// What the transcript has to say about a query.
///
/// Agents and groups are deliberately not here. The webview is already holding
/// both to draw the rail, so matching them there costs no round trip and stays
/// right while the operator types. What it is not holding is the transcript,
/// and shipping that across IPC to search it in the renderer would copy the
/// database once per keystroke.
#[tauri::command]
pub fn search(state: State<'_, AppState>, query: String, limit: Option<u32>) -> Reply<SearchHits> {
    Ok(state.runtime.store().search(query.trim(), limit.unwrap_or(20).min(100))?)
}

/// Sends the operator's message, with anything they dropped on the window.
///
/// `files` are paths on the operator's own disk, never bytes: the webview hands
/// over what was dropped and this side reads it, so a document never crosses
/// IPC and never sits in the renderer's memory.
#[tauri::command]
pub fn send_message(
    state: State<'_, AppState>,
    agent_id: AgentId,
    text: String,
    files: Option<Vec<String>>,
) -> Reply<RunId> {
    let trimmed = text.trim();
    let paths = files.unwrap_or_default();
    // A file on its own is a message. "Here, read this" with the document
    // attached is the most natural way to send one, and rejecting it as empty
    // would be the app arguing with the operator.
    if trimmed.is_empty() && paths.is_empty() {
        return Err(CommandError::new("validation", "message must not be empty"));
    }

    let mut attached = Vec::new();
    for path in &paths {
        match state.runtime.files().take(std::path::Path::new(path)) {
            Ok(file) => attached.push(file),
            // Named, because the operator picked this file deliberately and a
            // message that quietly went without it is worse than an error.
            Err(err) => return Err(CommandError::new("file", err.to_string())),
        }
    }
    Ok(state.runtime.send_from_human_with(agent_id, trimmed, attached)?)
}

/// Sends a failed turn's message again, as a new run.
///
/// Offered on the notice the failure left behind, so the operator retries the
/// thing that broke rather than retyping what they asked for.
#[tauri::command]
pub fn retry_turn(
    state: State<'_, AppState>,
    agent_id: AgentId,
    message_id: MessageId,
) -> Reply<RunId> {
    Ok(state.runtime.retry_turn(agent_id, message_id)?)
}

#[tauri::command]
pub fn clear_channel(state: State<'_, AppState>, channel_id: AgentId) -> Reply<usize> {
    Ok(state.runtime.store().delete_channel_messages(channel_id)?)
}

// ---- routines ------------------------------------------------------------

/// An agent's own schedule, as the operator sees it.
#[tauri::command]
pub fn agent_routines(state: State<'_, AppState>, id: AgentId) -> Reply<Vec<Routine>> {
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
}

impl RoutineDraft {
    /// Refuses a trigger this build does not understand rather than storing it
    /// and finding out at the next tick. A row nothing can parse is a schedule
    /// that silently never fires.
    fn checked(&self) -> Result<Trigger, CommandError> {
        let trigger = Trigger::parse(&self.trigger).ok_or_else(|| {
            CommandError::new("validation", format!("no trigger called {:?}", self.trigger))
        })?;
        routine::validate(&self.name, &self.what, trigger, self.in_secs)
            .map_err(|e| CommandError::new("validation", e.to_string()))?;
        Ok(trigger)
    }
}

#[tauri::command]
pub fn create_routine(
    state: State<'_, AppState>,
    agent_id: AgentId,
    draft: RoutineDraft,
) -> Reply<Routine> {
    let trigger = draft.checked()?;
    let first = trigger.first_run(now_ms(), draft.in_secs);
    let routine =
        state.runtime.store().create_routine(agent_id, &draft.name, &draft.what, trigger, first)?;
    state.runtime.emit(UiEvent::AgentsChanged);
    Ok(routine)
}

#[tauri::command]
pub fn update_routine(
    state: State<'_, AppState>,
    id: RoutineId,
    draft: RoutineDraft,
) -> Reply<Routine> {
    let trigger = draft.checked()?;

    let existing = state
        .runtime
        .store()
        .get_routine(id)?
        .ok_or_else(|| CommandError::new("notFound", format!("no routine with id {id}")))?;
    let next = match draft.in_secs {
        Some(_) => trigger.first_run(now_ms(), draft.in_secs),
        // The slot only stays put while the trigger does. "Every hour" turned
        // into "every weekday" keeps its hour but has to move off a Saturday,
        // or the label and the firing disagree from the moment it is saved.
        None if trigger.accepts(existing.next_run_at) => existing.next_run_at,
        None => trigger.next_after(existing.next_run_at, now_ms()).unwrap_or(existing.next_run_at),
    };

    let routine =
        state.runtime.store().update_routine(id, &draft.name, &draft.what, trigger, next)?;
    state.runtime.emit(UiEvent::AgentsChanged);
    Ok(routine)
}

/// Turns a routine off, or back on.
///
/// Not an edit to what it says, so it goes through its own command and leaves
/// the next firing where it was. A routine switched back on after its slot has
/// passed is overdue, and the scheduler fires an overdue slot once.
#[tauri::command]
pub fn set_routine_active(
    state: State<'_, AppState>,
    id: RoutineId,
    active: bool,
) -> Reply<Routine> {
    let routine = state.runtime.store().set_routine_active(id, active)?;
    state.runtime.emit(UiEvent::AgentsChanged);
    Ok(routine)
}

/// Fires a routine now, leaving its schedule alone.
///
/// The same delivery the scheduler makes, so what comes back from the button
/// is what will happen on Tuesday. Works on a routine that is switched off:
/// trying one out before turning it on is the point.
#[tauri::command]
pub fn test_routine(state: State<'_, AppState>, id: RoutineId) -> Reply<RunId> {
    let routine = state
        .runtime
        .store()
        .get_routine(id)?
        .ok_or_else(|| CommandError::new("notFound", format!("no routine with id {id}")))?;
    Ok(state.runtime.test_routine(&routine)?)
}

/// What a routine has done lately, newest first.
#[tauri::command]
pub fn routine_runs(state: State<'_, AppState>, id: RoutineId) -> Reply<Vec<RoutineRun>> {
    // Enough to answer "is this thing working" without turning the panel into
    // a log viewer. The transcript is where a firing is actually read.
    Ok(state.runtime.store().routine_runs(id, 20)?)
}

#[tauri::command]
pub fn delete_routine(state: State<'_, AppState>, id: RoutineId) -> Reply<()> {
    state.runtime.store().delete_routine(id)?;
    state.runtime.emit(UiEvent::AgentsChanged);
    Ok(())
}

/// What each of the given runs cost.
///
/// Asked for by the activity view, which knows which runs it is drawing. The
/// alternative, joining usage onto every message, would send the same totals
/// back once per message in the run.
#[tauri::command]
pub fn usage_for_runs(state: State<'_, AppState>, runs: Vec<RunId>) -> Reply<Vec<RunUsage>> {
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
#[tauri::command]
pub fn usage_summary(state: State<'_, AppState>) -> Reply<Vec<GroupUsage>> {
    Ok(state
        .runtime
        .store()
        .usage_by_group()?
        .into_iter()
        .map(|(group_id, tokens)| GroupUsage { group_id, tokens })
        .collect())
}

/// Empties every channel in a group. The crew stays; what it said does not.
#[tauri::command]
pub fn clear_group(state: State<'_, AppState>, group_id: GroupId) -> Reply<GroupReset> {
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
    pub notes: usize,
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
    pub base_url: Option<String>,
    pub api_key: Option<String>,
    pub default_model: Option<String>,
    pub request_timeout_secs: Option<u64>,
    pub limits: Option<GuardLimits>,
    pub e2b_api_key: Option<String>,
    pub computer_provider: Option<ProviderChoice>,
    pub computer_idle_minutes: Option<u32>,
}

#[tauri::command]
pub fn get_settings(state: State<'_, AppState>) -> Reply<RedactedConfig> {
    Ok(state.runtime.config().redacted())
}

/// Applies a patch to a config in memory. Shared by saving and testing, so a
/// tested configuration and a saved one can never diverge.
fn apply_patch(config: &mut AppConfig, patch: SettingsPatch) -> Result<(), CommandError> {
    if let Some(name) = patch.operator_name {
        config.operator_name = name.trim().to_string();
    }
    if let Some(base_url) = patch.base_url {
        config.inference.base_url = config::normalize_base_url(&base_url)?;
    }
    if let Some(key) = patch.e2b_api_key {
        config.e2b.api_key = key.trim().to_string();
    }
    if let Some(provider) = patch.computer_provider {
        // Only what a new computer is made with: the ones that exist keep
        // whoever made them until they are destroyed.
        config.computer.provider = provider;
    }
    if let Some(minutes) = patch.computer_idle_minutes {
        // A machine that sleeps after zero minutes can never be used, and one
        // that never sleeps is a bill nobody chose.
        config.computer.idle_minutes = minutes.clamp(1, 24 * 60);
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
    use crate::domain::computer::Provider;

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
    fn a_patch_writes_the_computer_settings_where_the_runtime_reads_them() {
        let mut config = AppConfig::default();

        apply_patch(
            &mut config,
            serde_json::from_str(
                r#"{"computerProvider":"appleContainer","computerIdleMinutes":0}"#,
            )
            .unwrap(),
        )
        .unwrap();

        assert_eq!(config.computer.provider, ProviderChoice::Provider(Provider::AppleContainer));
        assert_eq!(config.computer.idle_minutes, 1, "a machine that never wakes is unusable");
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
