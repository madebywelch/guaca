//! The backend command surface. Browsers and native clients both use its HTTP
//! dispatcher. Native-only host management lives in app.rs and holds no runtime.
//! The macro keeps command names, argument shapes and responses in one place;
//! ipc.contract.test.ts checks the frontend against this list.

use crate::domain::decision::WorkDecision;
use crate::domain::ids::DecisionId;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::commands::{
    self, AppState, ArtifactAddress, CommandError, FileRef, GroupReset, HarnessOnMachine,
    RoutineDraft, SettingsPatch, Staged, TerminalView,
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
    AgentId, ApprovalId, ConnectorId, EscalationId, GroupId, MessageId, PluginId, RoutineId, RunId,
};
use crate::domain::plugin::{
    HeaderPair, Plugin, PluginAccess, PluginKind, PluginOffer, ServerReport,
};
use crate::domain::routine::{Routine, RoutineRun};
use crate::domain::search::SearchHits;
use crate::domain::signin::Signin;
use crate::domain::terminal::{Gate, Harness, Payer, Tuning};
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
        self.body_for(None)
    }

    /// The same, naming which side to update when the client said what it is.
    pub fn body_for(&self, client: Option<&Client>) -> CommandError {
        match self {
            Refused::Unknown(name) => CommandError::new(
                "unknownCommand",
                format!("this build has no command called `{name}`. {}", skew(client)),
            ),
            Refused::Arguments { command, why } => CommandError::new(
                "badArguments",
                format!(
                    "`{command}` was called with arguments this build does not recognize ({why}). \
                     {}",
                    skew(client)
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

/// What a client says it is, on every call.
///
/// In the body rather than a header: a header an older host's CORS does not
/// admit fails the preflight of every call, which turns a version skew into a
/// workspace that cannot be reached at all.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Client {
    pub version: String,
    /// The commit it was built from. A source build carries the version of
    /// the last release, so two builds that agree on a version can disagree
    /// about which commands exist, and only this tells them apart.
    #[serde(default)]
    pub build: String,
    #[serde(default)]
    pub desktop: bool,
    /// One page load. What a sign-in's browser tab is addressed to, so the
    /// window that asked opens it and the others do not.
    #[serde(default)]
    pub id: Option<String>,
}

impl Client {
    /// The page id, if it is one: a short token, never text to repeat.
    pub fn page(&self) -> Option<String> {
        self.id.clone().filter(|id| {
            !id.is_empty()
                && id.len() <= 64
                && id.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-')
        })
    }

    /// The build, if it is one: a commit, possibly `-dirty`, never text to
    /// repeat.
    pub fn build(&self) -> Option<&str> {
        is_build(&self.build).then_some(self.build.as_str())
    }
}

fn commit(s: &str) -> bool {
    (7..=40).contains(&s.len())
        && s.bytes().all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}

fn is_build(s: &str) -> bool {
    commit(s.strip_suffix("-dirty").unwrap_or(s))
}

/// Two builds known to be different code: both commits, and not one commit
/// spelled at two lengths. A `-dirty` build matches only itself. The page's
/// copy of this rule is `otherBuild` in `src/lib/releases.ts`.
fn different_builds(a: &str, b: &str) -> bool {
    let same = a == b || (commit(a) && commit(b) && (a.starts_with(b) || b.starts_with(a)));
    is_build(a) && is_build(b) && !same
}

/// A commit as a person reads it: seven characters, and whether it was dirty.
fn short(build: &str) -> String {
    let (hash, dirty) = match build.strip_suffix("-dirty") {
        Some(hash) => (hash, "-dirty"),
        None => (build, ""),
    };
    format!("{}{dirty}", hash.get(..7).unwrap_or(hash))
}

/// The sentence that says which side of a skew to update.
///
/// Only a comparable release is ordered. Equal versions with different
/// commands are two builds under one number, at least one of them from
/// source, and a guess about which is older is worse than naming both.
pub fn skew(client: Option<&Client>) -> String {
    skew_from(client, env!("CARGO_PKG_VERSION"), crate::updates::BUILD)
}

fn skew_from(client: Option<&Client>, host: &str, build: &str) -> String {
    use std::cmp::Ordering;
    let order = client.and_then(|c| {
        let theirs = semver::Version::parse(&c.version).ok()?;
        Some(theirs.cmp(&semver::Version::parse(host).ok()?))
    });
    match (client, order) {
        (Some(c), Some(Ordering::Less)) => format!(
            "This {} is Guaca {} and the host is {host}; {}",
            if c.desktop { "app" } else { "page" },
            c.version,
            if c.desktop {
                "download the latest Guaca for this computer"
            } else {
                "reload the page to load the host's own version"
            }
        ),
        (Some(c), Some(Ordering::Greater)) => format!(
            "This {} is Guaca {} and the host is {host}; update the host",
            if c.desktop { "app" } else { "page" },
            c.version
        ),
        (Some(c), Some(Ordering::Equal)) if different_builds(&c.build, build) => {
            let side = if c.desktop { "app" } else { "page" };
            format!(
                "This {side} and the host are both Guaca {host} but different builds ({side} {}, \
                 host {}); {}",
                short(&c.build),
                short(build),
                if c.desktop {
                    "run an app and a host built from the same commit"
                } else {
                    "reload the page to load the host's own version"
                }
            )
        }
        _ => "The app and the workspace it is connected to are different builds; update \
              whichever is older"
            .into(),
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
    export_group(id: GroupId) -> crate::transfer::Archive,
    import_group(archive: crate::transfer::Archive, name: String) -> Group,
    group_reconnect(id: GroupId) -> Vec<crate::transfer::Reconnect>,

    set_agent_browser_consent(id: AgentId, consent: crate::domain::agent::Consent) -> (),
    set_agent_errands(id: AgentId, given: bool) -> (),
    calendar(from: i64, until: i64, group_id: Option<GroupId>) -> Vec<crate::domain::occasion::Occasion>,
    create_occasion(draft: crate::commands::OccasionDraft) -> crate::domain::occasion::Occasion,
    update_occasion(id: crate::domain::ids::OccasionId, draft: crate::commands::OccasionDraft) -> crate::domain::occasion::Occasion,
    delete_occasion(id: crate::domain::ids::OccasionId) -> (),
    artifacts(group_id: Option<GroupId>) -> Vec<crate::domain::artifact::Artifact>,
    artifact_detail(id: crate::domain::ids::ArtifactId) -> crate::commands::ArtifactDetail,
    artifact_page(id: crate::domain::ids::ArtifactId, version: Option<u32>) -> String,
    artifact_data(id: crate::domain::ids::ArtifactId, version: Option<u32>) -> Vec<crate::domain::artifact::Read>,
    allow_artifact_sources(id: crate::domain::ids::ArtifactId) -> crate::domain::artifact::Artifact,
    restore_artifact(id: crate::domain::ids::ArtifactId, version: u32) -> crate::domain::artifact::Artifact,
    hand_artifact(id: crate::domain::ids::ArtifactId, agent_id: AgentId) -> crate::domain::artifact::Artifact,
    delete_artifact(id: crate::domain::ids::ArtifactId) -> (),
    artifact_condensed(id: crate::domain::ids::ArtifactId) -> crate::commands::CondensedView,
    pin_artifact(id: crate::domain::ids::ArtifactId, width: crate::domain::widget::Width) -> RedactedConfig,
    unpin_artifact(id: crate::domain::ids::ArtifactId) -> RedactedConfig,
    webhook_address() -> crate::commands::WebhookAddress,

    report_view(view: crate::domain::view::OperatorView) -> (),
    builtin_tools() -> Vec<crate::llm::tools::ToolSummary>,
    add_quick_action(label: String, does: crate::domain::quick::Does) -> RedactedConfig,
    remove_quick_action(id: String) -> RedactedConfig,
    list_skills(scope: crate::domain::skill::Scope) -> Vec<crate::domain::skill::Skill>,
    read_skill(scope: crate::domain::skill::Scope, name: String) -> crate::domain::skill::Skill,
    save_skill(scope: crate::domain::skill::Scope, draft: crate::commands::SkillDraft) -> crate::domain::skill::Skill,
    delete_skill(scope: crate::domain::skill::Scope, name: String) -> bool,
    skill_directory(board: crate::skills_sh::Board, page: u32) -> crate::skills_sh::Page,
    search_skill_directory(query: String) -> Vec<crate::skills_sh::Listing>,
    preview_directory_skill(id: String) -> crate::skills_sh::Preview,
    add_directory_skill(scope: crate::domain::skill::Scope, id: String, hash: String) -> crate::domain::skill::Skill,

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
    update_connector(id: ConnectorId, agents: Vec<AgentId>, secret: Option<String>) -> (),
    delete_connector(id: ConnectorId) -> (),
    agent_terminal(id: AgentId) -> TerminalView,
    give_agent_terminal(id: AgentId) -> (),
    take_agent_terminal(id: AgentId) -> (),
    set_agent_coding(id: AgentId, harness: Harness, gate: Gate) -> (),
    set_coding_tuning(id: AgentId, harness: Harness, tuning: Tuning) -> (),
    coding_harnesses() -> Vec<HarnessOnMachine>,
    coding_models(id: AgentId, harness: Harness, pays: Payer) -> Vec<crate::coding::ModelOffer>,
    message_coding_job(agent_id: AgentId, message: String) -> crate::runtime::Continued,
    stop_coding_job(agent_id: AgentId) -> (),
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
    list_decisions() -> Vec<WorkDecision>,
    answer_decision(id: DecisionId, answer: String, updated_at: i64) -> WorkDecision,
    resume_decision(id: DecisionId) -> WorkDecision,
    snooze_decision(id: DecisionId, until: i64) -> (),
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
    move_group(id: GroupId, before: Option<GroupId>) -> Vec<Group>,
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
    agent_notebook(id: AgentId) -> Vec<crate::notebook::Entry>,
    read_notebook(id: AgentId, path: String) -> String,
    delete_notebook_file(id: AgentId, path: String) -> bool,
    agent_working_notes(id: AgentId) -> Vec<WorkingNote>,
    clear_agent_working_notes(id: AgentId) -> (),
    agent_last_active() -> HashMap<AgentId, i64>,
    channel_messages(channel_id: AgentId, limit: Option<u32>, through: Option<MessageId>) -> Vec<Envelope>,
    pair_messages(a: AgentId, b: AgentId, limit: Option<u32>) -> Vec<Envelope>,
    conversation_flow(group: GroupId, limit: Option<u32>) -> Vec<Envelope>,
    search(query: String, limit: Option<u32>) -> SearchHits,
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
    subscription_models() -> Vec<crate::llm::codex::SubscriptionModel>,
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

    fn client(version: &str, build: &str, desktop: bool) -> Client {
        Client { version: version.into(), build: build.into(), desktop, id: None }
    }

    #[test]
    fn a_skewed_client_is_told_which_side_to_update() {
        let host = semver::Version::parse(env!("CARGO_PKG_VERSION")).unwrap();
        let older = client("0.0.1", "", true);
        let newer = client(&format!("{}.0.0", host.major + 1), "", true);
        let message = Refused::Unknown("x".into()).body_for(Some(&older)).message;
        assert!(message.contains("download the latest Guaca"), "{message}");
        let message = Refused::Unknown("x".into()).body_for(Some(&newer)).message;
        assert!(message.contains("update the host"), "{message}");
        let page = client("0.0.1", "", false);
        let message = Refused::Unknown("x".into()).body_for(Some(&page)).message;
        assert!(message.contains("reload the page"), "{message}");
        // A client at the host's own version that names no build, or no claim
        // at all, cannot be told apart, and the message does not pretend otherwise.
        let same = client(env!("CARGO_PKG_VERSION"), "", true);
        for client in [Some(&same), None] {
            let message = Refused::Unknown("x".into()).body_for(client).message;
            assert!(message.contains("whichever is older"), "{message}");
        }
    }

    #[test]
    fn one_version_on_two_builds_names_both_builds() {
        // The failure this is for: a source-built app carries the version of
        // the release before it, so against that release's host the versions
        // agree and the commands do not. "Different versions" was false and
        // the Host pane agreed with neither side.
        let release = "c15bd9a58d5e4406c92f4ad65ca263a35d68a492";
        let app = client("0.2.0", "79961bb-dirty", true);
        let message = skew_from(Some(&app), "0.2.0", release);
        assert_eq!(
            message,
            "This app and the host are both Guaca 0.2.0 but different builds (app 79961bb-dirty, \
             host c15bd9a); run an app and a host built from the same commit"
        );
        let page = client("0.2.0", "79961bb", false);
        assert!(skew_from(Some(&page), "0.2.0", release).contains("reload the page"));

        // One commit at two lengths, and one dirty tree on both sides, are one build.
        for build in ["c15bd9a", release] {
            let same = client("0.2.0", build, true);
            assert!(skew_from(Some(&same), "0.2.0", release).contains("whichever is older"));
        }
        let dirty = client("0.2.0", "d02c114-dirty", true);
        assert!(skew_from(Some(&dirty), "0.2.0", "d02c114-dirty").contains("whichever is older"));
        assert!(skew_from(Some(&dirty), "0.2.0", "d02c114").contains("different builds"));

        // A client older than the field still parses, and names no build.
        let older: Client =
            serde_json::from_value(serde_json::json!({"version": "0.2.0"})).unwrap();
        assert_eq!(older.build(), None);

        // A host built without a commit knows nothing to compare, and a
        // build that is not a commit is not repeated back.
        assert!(skew_from(Some(&app), "0.2.0", "").contains("whichever is older"));
        let odd = client("0.2.0", "<script>", true);
        assert!(skew_from(Some(&odd), "0.2.0", release).contains("whichever is older"));
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
