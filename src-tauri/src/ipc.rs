//! The command surface, written once and served over two transports.
//!
//! Every command in [`crate::commands`] takes `&AppState` and returns a
//! [`Reply`]. This module is the list of them, and the three things that list
//! has to produce:
//!
//! - the `#[tauri::command]` wrappers the desktop app registers,
//! - [`dispatch`], which is the same call arriving as JSON over HTTP,
//! - [`NAMES`], which is what `ipc.contract.test.ts` compares against
//!   `src/lib/ipc.ts`.
//!
//! ## Why a macro rather than three lists
//!
//! Because three lists drift, and the drift is invisible until somebody clicks
//! something. A command reachable from the desktop and not from a server is a
//! panel that works at your desk and fails on your box, with nothing on screen
//! saying which half is missing. Written once, that failure cannot be
//! expressed: adding a line here adds it everywhere or compiles nowhere.
//!
//! The contract test is still the gate, because it is the only thing that can
//! see the fourth list, which is in TypeScript.
//!
//! ## Two shapes, not two protocols
//!
//! Tauri already models a command as "a name, some named arguments, and a
//! result or a structured error". That is also what an HTTP POST is, so
//! [`dispatch`] is a translation rather than a second API: same names, same
//! camelCase argument keys, same [`CommandError`] on the way back. Nothing
//! above this line learns which one it arrived on.

use serde::Serialize;
use serde_json::Value;

use crate::commands::{
    self, AppState, ArtifactAddress, CommandError, FileRef, GroupReset, HarnessOnMachine,
    RoutineDraft, SettingsPatch, Staged,
};
use crate::config::RedactedConfig;
use crate::domain::agent::{AgentCard, AgentDraft};
use crate::domain::approval::{Approval, ApprovalState, Decision, ProtectedAction};
use crate::domain::connector::{Connector, ConnectorDraft};
use crate::domain::deployment::Capabilities;
use crate::domain::envelope::Envelope;
use crate::domain::escalation::Escalation;
use crate::domain::group::{Group, GroupDraft};
use crate::domain::ids::{
    AgentId, ApprovalId, ConnectorId, EscalationId, GroupId, MessageId, PluginId, RepositoryId,
    RoutineId, RunId,
};
use crate::domain::plugin::{
    HeaderPair, Plugin, PluginAccess, PluginKind, PluginOffer, ServerReport,
};
use crate::domain::repository::{Bench, Gate, Harness, Repository, RepositoryDraft};
use crate::domain::routine::{Routine, RoutineRun};
use crate::domain::search::SearchHits;
use crate::domain::signin::Signin;
use crate::domain::usage::{GroupUsage, RunUsage};
use crate::domain::worknote::WorkingNote;
use crate::e2b::Computer;
use crate::kernel::Browser;
use crate::llm::catalog::RankedModel;
use crate::menubar::Presence;
use crate::runtime::events::Activity;
use crate::subscription::{DeviceCode, Status};

use std::collections::HashMap;

/// Why a call did not produce an answer.
///
/// Four cases rather than one string, because they are four different people's
/// problems. An unknown command or malformed arguments is a client that is out
/// of step with this build, which is the failure the contract test exists to
/// prevent and which an operator can do nothing about. A [`CommandError`] is
/// the command itself refusing, and it is the one the UI already knows how to
/// draw. A result that will not serialize is this build's own bug.
#[derive(Debug)]
pub enum Refused {
    Unknown(String),
    Arguments { command: String, why: String },
    Command(CommandError),
    Answer(String),
}

impl Refused {
    /// The status a transport should answer with.
    ///
    /// A refusal by the command is a 200 carrying a structured error, not an
    /// HTTP failure: the webview has always received those as a rejected
    /// promise with a `kind` on it, and turning half of them into 4xx would
    /// give the client two ways to learn the same thing.
    pub const fn status(&self) -> u16 {
        match self {
            Refused::Unknown(_) => 404,
            Refused::Arguments { .. } => 400,
            Refused::Command(_) => 200,
            Refused::Answer(_) => 500,
        }
    }

    /// What the client is told, in the shape it already parses.
    pub fn body(&self) -> CommandError {
        match self {
            Refused::Unknown(name) => CommandError::new(
                "unknownCommand",
                format!(
                    "this build has no command called `{name}`. The app and the workspace it is \
                     connected to are different versions; update whichever is older"
                ),
            ),
            Refused::Arguments { command, why } => CommandError::new(
                "badArguments",
                format!(
                    "`{command}` was called with arguments this build does not recognize ({why}). \
                     The app and the workspace it is connected to are different versions; update \
                     whichever is older"
                ),
            ),
            Refused::Command(err) => CommandError::new(err.kind, err.message.clone()),
            Refused::Answer(why) => CommandError::new(
                "storage",
                format!("the answer could not be encoded to send back ({why})"),
            ),
        }
    }
}

impl std::fmt::Display for Refused {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.body().message)
    }
}

/// One call, answered.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Answered {
    pub status: u16,
    pub value: Value,
}

macro_rules! surface {
    ($( $name:ident ( $( $arg:ident : $ty:ty ),* $(,)? ) -> $ret:ty ),* $(,)?) => {
        /// The desktop half: the same commands, wearing Tauri's attribute.
        ///
        /// Absent from a daemon build, which is why the `surface!` list rather
        /// than this module is what the contract test reads: the list is the
        /// same in both builds and this module is not.
        ///
        /// Thin by construction. A wrapper that grew a line of its own would be
        /// behavior the server transport does not have, which is the one thing
        /// this module exists to make impossible.
        #[cfg(feature = "desktop")]
        pub mod desktop {
            use tauri::State;

            use super::*;

            $(
                #[tauri::command]
                pub async fn $name(
                    state: State<'_, AppState>,
                    $( $arg: $ty, )*
                ) -> crate::commands::Reply<$ret> {
                    commands::$name(&state, $( $arg, )*).await
                }
            )*
        }

        /// The server half: a name, some JSON arguments, and an answer.
        pub async fn dispatch(state: &AppState, name: &str, args: Value) -> Result<Value, Refused> {
            // A call with no arguments arrives as `null` from some clients and
            // `{}` from others, and both mean the same thing. Normalizing here
            // rather than in each arm keeps the nineteen zero-argument commands
            // from each needing a case.
            let args = if args.is_null() { Value::Object(Default::default()) } else { args };
            match name {
                $(
                    stringify!($name) => {
                        #[derive(serde::Deserialize)]
                        #[serde(rename_all = "camelCase")]
                        struct Args { $( $arg: $ty, )* }

                        let Args { $( $arg, )* } = serde_json::from_value(args).map_err(|err| {
                            Refused::Arguments {
                                command: name.to_string(),
                                why: err.to_string(),
                            }
                        })?;
                        // Annotated rather than inferred, so the return type
                        // written in the list has to be the one the command
                        // actually has. Without it the list could drift from
                        // the implementation and only the desktop wrappers
                        // would notice.
                        let value: $ret = commands::$name(state, $( $arg, )*)
                            .await
                            .map_err(Refused::Command)?;
                        serde_json::to_value(value).map_err(|err| Refused::Answer(err.to_string()))
                    }
                )*
                _ => Err(Refused::Unknown(name.to_string())),
            }
        }

        /// Every command this build answers to.
        ///
        /// Read by `ipc.contract.test.ts`, which compares it against the calls
        /// `src/lib/ipc.ts` can make. A name on one side and not the other is a
        /// build failure rather than a click that does nothing.
        pub const NAMES: &[&str] = &[ $( stringify!($name), )* ];
    };
}

surface! {
    set_agent_browser_consent(id: AgentId, consent: crate::domain::agent::Consent) -> AgentCard,
    calendar(from: i64, until: i64, group_id: Option<GroupId>) -> Vec<crate::domain::occasion::Occasion>,
    create_occasion(draft: crate::commands::OccasionDraft) -> crate::domain::occasion::Occasion,
    update_occasion(id: crate::domain::ids::OccasionId, draft: crate::commands::OccasionDraft) -> crate::domain::occasion::Occasion,
    delete_occasion(id: crate::domain::ids::OccasionId) -> (),
    webhook_address() -> crate::commands::WebhookAddress,

    agent_computer(id: AgentId) -> Option<Computer>,
    give_agent_computer(id: AgentId) -> (),
    take_agent_computer(id: AgentId) -> (),
    start_agent_computer(id: AgentId) -> Computer,
    stop_agent_computer(id: AgentId) -> Option<Computer>,
    delete_agent_computer(id: AgentId) -> (),
    agent_browser(id: AgentId) -> Option<Browser>,
    give_agent_browser(id: AgentId) -> (),
    take_agent_browser(id: AgentId) -> (),
    start_agent_browser(id: AgentId) -> Browser,
    stop_agent_browser(id: AgentId) -> (),
    group_connectors(group_id: GroupId) -> Vec<Connector>,
    create_connector(draft: ConnectorDraft) -> Connector,
    delete_connector(id: ConnectorId) -> (),
    group_repositories(group_id: GroupId) -> Vec<Repository>,
    repository_statuses() -> std::collections::HashMap<RepositoryId, crate::repo::RepoStatus>,
    list_repositories() -> Vec<Repository>,
    create_repository(draft: RepositoryDraft) -> Repository,
    update_repository(id: RepositoryId, name: String, note: String, harness: Harness, gate: Gate, bench: Bench) -> Repository,
    coding_harnesses() -> Vec<HarnessOnMachine>,
    set_repository_author(id: RepositoryId, author: crate::domain::repository::GitIdentity) -> crate::repo::auth::Connection,
    repository_connection(id: RepositoryId) -> crate::repo::auth::Connection,
    create_github_repository(draft: RepositoryDraft) -> Repository,
    set_repository_github(id: RepositoryId) -> crate::repo::auth::Connection,
    set_repository_credential(id: RepositoryId, username: String, token: String) -> crate::repo::auth::Connection,
    clear_repository_credential(id: RepositoryId) -> crate::repo::auth::Connection,
    check_repository_connection(id: RepositoryId) -> String,
    message_coding_job(agent_id: AgentId, message: String) -> (),
    stop_coding_job(agent_id: AgentId) -> (),
    delete_repository(id: RepositoryId) -> (),
    set_agent_repository(id: AgentId, repository_id: Option<RepositoryId>) -> AgentCard,
    plugin_catalog() -> Vec<PluginOffer>,
    group_plugins(group_id: GroupId) -> Vec<Plugin>,
    connect_plugin(group_id: GroupId, kind: PluginKind, connection: Option<String>) -> Plugin,
    add_plugin(group_id: GroupId, name: String, url: String, key: Option<String>, headers: Option<Vec<HeaderPair>>) -> Plugin,
    set_plugin_connection(group_id: GroupId, kind: PluginKind, connection: String) -> Plugin,
    readdress_plugin(group_id: GroupId, id: PluginId, url: String, key: Option<String>, headers: Option<Vec<HeaderPair>>) -> Plugin,
    probe_server(url: String, key: Option<String>, headers: Option<Vec<HeaderPair>>) -> ServerReport,
    check_plugin(id: PluginId) -> ServerReport,
    set_plugin_access(id: PluginId, access: PluginAccess) -> Plugin,
    set_plugin_tool(id: PluginId, tool: String, access: PluginAccess) -> Plugin,
    disconnect_plugin(id: PluginId) -> (),
    scan_agent_signins(id: AgentId) -> Vec<Signin>,
    agent_signins(id: AgentId) -> Vec<Signin>,
    approval_states() -> HashMap<ApprovalId, ApprovalState>,
    pending_approvals() -> Vec<Approval>,
    agent_grants(id: AgentId) -> Vec<ProtectedAction>,
    revoke_grant(id: AgentId, action: ProtectedAction) -> Vec<ProtectedAction>,
    decide_approval(id: ApprovalId, decision: Decision) -> Approval,
    answer_question(id: ApprovalId, answer: String) -> Approval,
    open_escalations() -> Vec<Escalation>,
    clear_escalation(id: EscalationId) -> (),
    list_groups() -> Vec<Group>,
    create_group(draft: GroupDraft) -> Group,
    update_group(id: GroupId, draft: GroupDraft) -> Group,
    test_group_connection(id: Option<GroupId>, draft: GroupDraft) -> String,
    delete_group(id: GroupId) -> (),
    disband_group(id: GroupId) -> (),
    list_agents() -> Vec<AgentCard>,
    create_agent(draft: AgentDraft) -> AgentCard,
    update_agent(id: AgentId, draft: AgentDraft) -> AgentCard,
    delete_agent(id: AgentId) -> (),
    restore_agent(id: AgentId) -> AgentCard,
    purge_agent(id: AgentId) -> (),
    set_agent_paused(id: AgentId, paused: bool) -> AgentCard,
    set_agent_pinned(id: AgentId, pinned: bool) -> AgentCard,
    move_agent(id: AgentId, group_id: GroupId, before: Option<AgentId>) -> AgentCard,
    duplicate_agent(id: AgentId) -> AgentCard,
    hire_agents(group_id: GroupId, drafts: Vec<AgentDraft>) -> Vec<AgentCard>,
    agent_activity() -> HashMap<AgentId, Activity>,
    agent_memory(id: AgentId) -> String,
    set_agent_memory(id: AgentId, content: String) -> String,
    agent_working_notes(id: AgentId) -> Vec<WorkingNote>,
    clear_agent_working_notes(id: AgentId) -> (),
    agent_last_active() -> HashMap<AgentId, i64>,
    channel_messages(channel_id: AgentId, limit: Option<u32>, through: Option<MessageId>) -> Vec<Envelope>,
    pair_messages(a: AgentId, b: AgentId, limit: Option<u32>) -> Vec<Envelope>,
    conversation_flow(group: GroupId, limit: Option<u32>) -> Vec<Envelope>,
    search(query: String, limit: Option<u32>) -> SearchHits,
    stage_files(paths: Vec<String>) -> Staged,
    send_message(agent_id: AgentId, text: String, files: Option<Vec<FileRef>>) -> RunId,
    save_file(digest: String, name: String) -> String,
    frame_artifact(html: String) -> ArtifactAddress,
    retry_turn(agent_id: AgentId, message_id: MessageId) -> RunId,
    stop_run(run_id: RunId) -> bool,
    clear_channel(channel_id: AgentId) -> usize,
    agent_routines(id: AgentId) -> Vec<Routine>,
    create_routine(agent_id: AgentId, draft: RoutineDraft) -> Routine,
    update_routine(id: RoutineId, draft: RoutineDraft) -> Routine,
    set_routine_active(id: RoutineId, active: bool) -> Routine,
    test_routine(id: RoutineId) -> RunId,
    routine_runs(id: RoutineId) -> Vec<RoutineRun>,
    delete_routine(id: RoutineId) -> (),
    usage_for_runs(runs: Vec<RunId>) -> Vec<RunUsage>,
    usage_summary() -> Vec<GroupUsage>,
    clear_group(group_id: GroupId) -> GroupReset,
    capabilities() -> Capabilities,
    forward_files(origin: String, token: String, paths: Vec<String>) -> Staged,
    report_presence(presence: Option<Presence>) -> (),
    stop_everything() -> usize,
    get_settings() -> RedactedConfig,
    account_status() -> crate::account::Status,
    sign_in_account() -> crate::account::Status,
    account_connectors() -> crate::account::Connectors,
    sign_out_account() -> crate::account::Status,
    subscription_status() -> Status,
    begin_subscription_signin() -> DeviceCode,
    complete_subscription_signin(code: DeviceCode) -> Status,
    sign_out_subscription() -> RedactedConfig,
    update_settings(patch: SettingsPatch) -> RedactedConfig,
    test_connection(patch: Option<SettingsPatch>) -> String,
    ranked_models(category: String) -> Vec<RankedModel>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_command_is_listed_exactly_once() {
        // A duplicate compiles: the match arm is unreachable and the Tauri
        // module fails, but only if the arities differ. This is the cheap check
        // that the generated list is a set.
        let mut seen = NAMES.to_vec();
        seen.sort_unstable();
        let count = seen.len();
        seen.dedup();
        assert_eq!(count, seen.len(), "a command is listed twice");
    }

    #[test]
    fn the_surface_is_not_accidentally_empty() {
        // The macro takes a trailing comma and an empty list, so a botched
        // generation is a build that compiles and answers nothing.
        assert!(NAMES.len() > 90, "only {} commands reached the surface", NAMES.len());
    }

    #[tokio::test]
    async fn an_unknown_command_names_itself_and_says_what_to_do() {
        // The failure this is written for is a client and a workspace on
        // different builds, which on a server is routine rather than
        // impossible. "unknown command" alone sends an operator to look at
        // their crew.
        let refused = Refused::Unknown("summon_kraken".into());
        assert_eq!(refused.status(), 404);
        let body = refused.body();
        assert_eq!(body.kind, "unknownCommand");
        assert!(body.message.contains("summon_kraken"), "{}", body.message);
        assert!(body.message.contains("update"), "{}", body.message);
    }

    #[test]
    fn a_command_refusing_is_not_an_http_failure() {
        // The webview has always received a refusal as a rejected promise
        // carrying a `kind`. Making half of them a 4xx would give the client
        // two ways to learn one thing, and the two would disagree.
        let refused = Refused::Command(CommandError::new("duplicateName", "there are two Pips"));
        assert_eq!(refused.status(), 200);
        assert_eq!(refused.body().kind, "duplicateName");
    }
}
