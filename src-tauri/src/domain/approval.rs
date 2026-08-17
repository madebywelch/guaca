//! Asking the operator before doing something they would want a say in.
//!
//! An agent's tools all act on its own machine, its own notes or its own peers.
//! Some things reach further than that, and those stop and ask. The mechanism is
//! general because the answer has to be recorded either way: which agent asked,
//! for what, and what the operator said. [`ProtectedAction`] has one variant
//! today, and adding a second is a variant plus the call site that raises it.
//!
//! The wording an operator reads is composed here from what the runtime already
//! validated, never by the model. An agent that could write its own request
//! could describe creating an agent as tidying up. Where a request is
//! necessarily about something only the agent can describe, its words appear as
//! a quoted detail field, under a heading the runtime wrote.

use serde::{Deserialize, Serialize};

use super::ids::{AgentId, ApprovalId, GroupId, RunId};

/// Something an agent may not do on its own.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ProtectedAction {
    /// Adding an agent to the workspace. Protected because it changes who the
    /// operator has, permanently, and every one of them costs money to run.
    CreateAgent,
    /// Doing something outside the workspace in the operator's name: sending
    /// mail as them, submitting a form, paying for something.
    ///
    /// Protected because it cannot be taken back and because the operator's
    /// authority is not transitive. An agent told by a peer that it has been
    /// authorised is being told a claim, not given permission, and the only
    /// thing that settles it is the operator themselves. Before this existed
    /// the agent's only move was to refuse and ask them to repeat the
    /// instruction in another channel, which is the operator doing the
    /// routing by hand.
    ActOnBehalf,
}

impl ProtectedAction {
    /// The stored form. Identical to the serialized form on purpose: two
    /// spellings of one token is a mapping table waiting to go wrong.
    pub fn as_str(self) -> &'static str {
        match self {
            ProtectedAction::CreateAgent => "createAgent",
            ProtectedAction::ActOnBehalf => "actOnBehalf",
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "createAgent" => Some(ProtectedAction::CreateAgent),
            "actOnBehalf" => Some(ProtectedAction::ActOnBehalf),
            _ => None,
        }
    }
}

/// What the operator can answer. Separate from [`ApprovalState`] so that the
/// two states nobody can choose, pending and expired, cannot arrive over IPC.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Decision {
    /// This once.
    Allow,
    /// This one and every later one from the same agent for the same action.
    AlwaysAllow,
    Deny,
}

impl Decision {
    pub fn grants(self) -> bool {
        matches!(self, Decision::Allow | Decision::AlwaysAllow)
    }
}

/// Where a request has got to.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ApprovalState {
    /// Waiting on the operator. The agent that asked is parked mid-turn.
    Pending,
    Allow,
    AlwaysAllow,
    Deny,
    /// Nobody answered in time, or the app restarted while it was waiting. A
    /// request that outlives the turn it belongs to can never be granted: the
    /// agent it would have unblocked is gone.
    Expired,
}

impl ApprovalState {
    pub fn as_str(self) -> &'static str {
        match self {
            ApprovalState::Pending => "pending",
            ApprovalState::Allow => "allow",
            ApprovalState::AlwaysAllow => "alwaysAllow",
            ApprovalState::Deny => "deny",
            ApprovalState::Expired => "expired",
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "pending" => Some(ApprovalState::Pending),
            "allow" => Some(ApprovalState::Allow),
            "alwaysAllow" => Some(ApprovalState::AlwaysAllow),
            "deny" => Some(ApprovalState::Deny),
            "expired" => Some(ApprovalState::Expired),
            _ => None,
        }
    }
}

impl From<Decision> for ApprovalState {
    fn from(decision: Decision) -> Self {
        match decision {
            Decision::Allow => ApprovalState::Allow,
            Decision::AlwaysAllow => ApprovalState::AlwaysAllow,
            Decision::Deny => ApprovalState::Deny,
        }
    }
}

/// One field of the request, as the operator sees it.
///
/// The values are what the model asked for, so they are shown as labelled data
/// and never as prose. `value` is rendered as text, never as markdown: an agent
/// that could format its request could draw a button.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DetailField {
    pub label: String,
    pub value: String,
}

impl DetailField {
    pub fn new(label: impl Into<String>, value: impl Into<String>) -> Self {
        Self { label: label.into(), value: value.into() }
    }
}

/// One request, and what became of it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Approval {
    pub id: ApprovalId,
    /// Who asked. A grant belongs to this agent alone: allowing the Manager to
    /// create agents says nothing about anyone else.
    pub agent_id: AgentId,
    pub group_id: GroupId,
    /// The run the asking turn belongs to, so a request can be traced back to
    /// the operator action that set it off.
    pub run_id: RunId,
    pub action: ProtectedAction,
    /// Guaca's own one-line description of what was asked for.
    pub summary: String,
    pub detail: Vec<DetailField>,
    pub state: ApprovalState,
    pub created_at: i64,
    pub decided_at: Option<i64>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stored_and_serialized_spellings_are_the_same_token() {
        // One spelling, so a row written by the runtime and a state read by the
        // UI can never mean different things.
        for state in [
            ApprovalState::Pending,
            ApprovalState::Allow,
            ApprovalState::AlwaysAllow,
            ApprovalState::Deny,
            ApprovalState::Expired,
        ] {
            let json = serde_json::to_value(state).unwrap();
            assert_eq!(json.as_str(), Some(state.as_str()));
            assert_eq!(ApprovalState::parse(state.as_str()), Some(state));
        }

        let action = ProtectedAction::CreateAgent;
        assert_eq!(serde_json::to_value(action).unwrap().as_str(), Some(action.as_str()));
        assert_eq!(ProtectedAction::parse(action.as_str()), Some(action));
    }

    #[test]
    fn an_unknown_stored_state_is_rejected_rather_than_defaulted() {
        // Defaulting to pending would resurrect a settled request; defaulting to
        // denied would silently drop one. Neither is a guess worth making.
        assert_eq!(ApprovalState::parse("maybe"), None);
        assert_eq!(ProtectedAction::parse("delete_everything"), None);
    }

    #[test]
    fn both_kinds_of_yes_grant_and_no_does_not() {
        assert!(Decision::Allow.grants());
        assert!(Decision::AlwaysAllow.grants());
        assert!(!Decision::Deny.grants());
    }

    #[test]
    fn the_states_an_operator_cannot_choose_are_not_a_decision() {
        // `Decision` is the IPC input type. Pending and expired are outcomes of
        // time passing, so they must not be expressible as an answer.
        assert!(serde_json::from_str::<Decision>("\"pending\"").is_err());
        assert!(serde_json::from_str::<Decision>("\"expired\"").is_err());
        assert_eq!(
            serde_json::from_str::<Decision>("\"alwaysAllow\"").unwrap(),
            Decision::AlwaysAllow
        );
    }
}
