//! Asking the operator before doing something they would want a say in.
//!
//! An agent's tools all act on its own machine, its own notes or its own peers.
//! Some things reach further than that, and those stop and ask. The mechanism is
//! general because the answer has to be recorded either way: which agent asked,
//! for what, and what the operator said. [`ProtectedAction`] has three variants
//! today, and adding another is a variant plus the call site that raises it.
//!
//! The wording an operator reads is composed here from what the runtime already
//! validated, never by the model. An agent that could write its own request
//! could describe creating an agent as tidying up. Where a request is
//! necessarily about something only the agent can describe, its words appear as
//! a quoted detail field, under a heading the runtime wrote.

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use super::decision::DecisionRequest;
use super::ids::{AgentId, ApprovalId, GroupId, RunId};

/// What an agent has stopped mid-turn to put to the operator.
///
/// Two kinds, and the line between them is what a yes does. A permission
/// authorizes: the agent could not do the thing, and the operator's answer is
/// what lets it. A question informs: the agent could act either way and does
/// not know which the operator wants, so the answer is a value it carries into
/// the rest of its turn, and whatever it then does with it passes through every
/// guard it already had.
///
/// That distinction is why the two are separate variants rather than one shape
/// with a flag, and it is what makes a question safe to draw as buttons with
/// the model's own words on them. Nothing a question can be answered with
/// grants anything, so no wording on it can talk the operator into granting
/// something. A permission is the opposite case, which is why every word on one
/// is Guaca's: see the note on this module.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase", rename_all_fields = "camelCase")]
pub enum Request {
    Permission {
        action: ProtectedAction,
    },
    Question {
        /// What the operator may pick, or empty for a written answer.
        ///
        /// The agent's own words, which is the one place in this module that is
        /// true. They are shown as the labels of buttons and they are capped
        /// and drawn as text, never as markup.
        options: Vec<String>,
    },
}

impl Request {
    /// The stored discriminator, which for a permission is the action itself.
    ///
    /// One column rather than two, because a second `kind` column would be a
    /// value that has to agree with this one and nothing would make it. The
    /// three tokens cannot collide: `QUESTION` is not a `ProtectedAction` and
    /// `ProtectedAction::parse` refuses it.
    pub fn as_str(&self) -> &'static str {
        match self {
            Request::Permission { action } => action.as_str(),
            Request::Question { .. } => QUESTION,
        }
    }

    /// The action this asks to be let off, if it asks to be let off anything.
    ///
    /// A question never grants, so it never has one, and every caller that
    /// reaches for a standing grant has to say so by unwrapping this.
    pub fn action(&self) -> Option<ProtectedAction> {
        match self {
            Request::Permission { action } => Some(*action),
            Request::Question { .. } => None,
        }
    }
}

/// The stored `action` of a question. Not a [`ProtectedAction`], on purpose.
pub const QUESTION: &str = "question";

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
    /// authorized is being told a claim, not given permission, and the only
    /// thing that settles it is the operator themselves. Before this existed
    /// the agent's only move was to refuse and ask them to repeat the
    /// instruction in another channel, which is the operator doing the
    /// routing by hand.
    ActOnBehalf,
    /// Changing the workspace's settings: a model, a timeout, a limit.
    ///
    /// Protected because the settings are what every agent runs inside, and
    /// the limits among them are the guard that ends a cascade. An agent that
    /// could raise its own relay depth has no limit. Each change is shown with
    /// its before and after, and none of them stands: see [`Self::stands`].
    ChangeSettings,
}

impl ProtectedAction {
    /// Whether a yes can be made standing, so the agent is not asked again.
    ///
    /// Only for creating an agent. "Always" is scoped to an agent and an
    /// action, and for the other two the action is too wide for one answer to
    /// cover. A settings change is "any setting, to any value", so a standing
    /// yes would cover a limit raised to its ceiling as readily as the timeout
    /// it was given for. Acting in the operator's name is every future send,
    /// submission and purchase, and it is also what a press on a signed-in
    /// site asks as, so a standing yes would switch the browser's consent gate
    /// off for good, on the strength of one answer given before any page that
    /// gate exists for had been read. Enforced where a verdict is recorded and
    /// where a grant is read, not only in the UI, because any client can send
    /// "always" and an older build recorded it.
    pub fn stands(self) -> bool {
        matches!(self, ProtectedAction::CreateAgent)
    }

    /// The stored form. Identical to the serialized form on purpose: two
    /// spellings of one token is a mapping table waiting to go wrong.
    pub fn as_str(self) -> &'static str {
        match self {
            ProtectedAction::CreateAgent => "createAgent",
            ProtectedAction::ActOnBehalf => "actOnBehalf",
            ProtectedAction::ChangeSettings => "changeSettings",
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "createAgent" => Some(ProtectedAction::CreateAgent),
            "actOnBehalf" => Some(ProtectedAction::ActOnBehalf),
            "changeSettings" => Some(ProtectedAction::ChangeSettings),
            _ => None,
        }
    }
}

/// What the operator can answer a *permission* with. Separate from
/// [`ApprovalState`] so that the two states nobody can choose, pending and
/// expired, cannot arrive over IPC.
///
/// A question is answered through its own command with its own text, rather
/// than by a fourth variant here. Two reasons, and the second is the one that
/// matters. A verdict is three tokens on a wire and an answer is arbitrary text
/// from a person, so folding them together turns a plain string into a tagged
/// object everywhere this is already spoken, including the menu bar, which can
/// take a verdict and can never take an answer. And the split is the same one
/// [`Request`] draws: these three authorize, and an answer does not.
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
    /// A question, answered. What was said is on the row rather than in here,
    /// because it is a value and this is a state.
    Answered,
    /// Nobody answered in time, or the app restarted while it was waiting. A
    /// request that outlives the turn it belongs to can never be granted: the
    /// agent it would have unblocked is gone. One that ran out of time is
    /// filed as a decision in the same commit, which is where a late answer
    /// goes instead: see [`Approval::as_decision`].
    Expired,
}

impl ApprovalState {
    pub fn as_str(self) -> &'static str {
        match self {
            ApprovalState::Pending => "pending",
            ApprovalState::Allow => "allow",
            ApprovalState::AlwaysAllow => "alwaysAllow",
            ApprovalState::Deny => "deny",
            ApprovalState::Answered => "answered",
            ApprovalState::Expired => "expired",
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "pending" => Some(ApprovalState::Pending),
            "allow" => Some(ApprovalState::Allow),
            "alwaysAllow" => Some(ApprovalState::AlwaysAllow),
            "deny" => Some(ApprovalState::Deny),
            "answered" => Some(ApprovalState::Answered),
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
/// The values are what the model asked for, so they are shown as labeled data
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
    pub request: Request,
    /// Guaca's own one-line description of what was asked for.
    pub summary: String,
    pub detail: Vec<DetailField>,
    pub state: ApprovalState,
    /// What the operator picked or wrote, once they have. Only ever set on a
    /// question: a verdict is a state, not a value.
    pub answer: Option<String>,
    pub created_at: i64,
    pub decided_at: Option<i64>,
}

/// What the operator picks to let a lapsed permission through, and to refuse
/// it. Exactly these, because the answer is compared with them: a decision's
/// answer is text, and only this one grants.
pub const ALLOW: &str = "Allow";
pub const DENY: &str = "Deny";

/// How long an Allow given on a lapsed permission waits to be used.
///
/// A day, because the request lapsed while nobody was there, and the operator
/// who answers it tomorrow morning is answering for work the agent picks up
/// again whenever it next runs. Longer and the yes outlives what it was about.
pub const GRANT_LIFETIME_MS: i64 = 24 * 60 * 60 * 1000;

/// What a one-time grant is bound to: the action, and what the agent said it
/// would do, as Guaca recorded it before asking.
///
/// Hashed rather than stored, because nothing reads it back except to compare
/// it with the next request, and the words it is made of are already on the
/// approvals row. `what` is built by the caller from the declared facts, so
/// what counts as "the same action" is decided once per action, where its
/// facts are known: the recipients and amount of a send, the details of a new
/// agent, each setting with its before and after. A settings grant therefore
/// stops matching the moment any setting in it changes, because its "before"
/// is part of what was allowed.
pub fn fingerprint(action: ProtectedAction, what: &serde_json::Value) -> String {
    format!("{:x}", Sha256::digest(format!("{}\n{what}", action.as_str()).as_bytes()))
}

impl Approval {
    /// The same request, as a decision the operator can answer later.
    ///
    /// What a request becomes when nobody answers it in time. The question is
    /// the one on the card: Guaca's own sentence for a permission, the agent's
    /// for a question. A permission's details become the context, one per
    /// line, as the card drew them. The choices are Allow and Deny, or the
    /// question's own, and a question that took a written answer still does.
    /// Cut to what a decision holds rather than refused: the request was valid
    /// when it was asked, and a long one losing its tail is better than one
    /// that never reaches the desk.
    pub fn as_decision(&self) -> DecisionRequest {
        let (question, cut) = super::cut_to(&self.summary, 300);
        let context = if self.detail.is_empty() && cut {
            super::cut_to(&self.summary, 1200).0
        } else {
            let lines: Vec<String> = self
                .detail
                .iter()
                .map(|field| format!("{}: {}", field.label.trim(), field.value.trim()))
                .collect();
            super::cut_to(&lines.join("\n"), 1200).0
        };
        let options = match &self.request {
            Request::Permission { .. } => vec![ALLOW.to_string(), DENY.to_string()],
            Request::Question { options } => {
                let mut kept: Vec<String> = Vec::new();
                for option in options {
                    let (option, _) = super::cut_to(option, 120);
                    if !option.is_empty() && !kept.contains(&option) && kept.len() < 6 {
                        kept.push(option);
                    }
                }
                if kept.len() < 2 {
                    kept.clear();
                }
                kept
            }
        };
        DecisionRequest {
            question,
            context,
            recommendation: String::new(),
            options,
            source: String::new(),
        }
    }
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
    fn a_fingerprint_is_the_action_and_exactly_what_was_asked() {
        let what = serde_json::json!({"action": "Send the weekly report", "recipients": ["a@b.c"]});
        let same = serde_json::json!({"action": "Send the weekly report", "recipients": ["a@b.c"]});
        let other =
            serde_json::json!({"action": "Send the weekly report", "recipients": ["x@b.c"]});
        let act = ProtectedAction::ActOnBehalf;
        assert_eq!(fingerprint(act, &what), fingerprint(act, &same));
        assert_ne!(fingerprint(act, &what), fingerprint(act, &other));
        assert_ne!(fingerprint(act, &what), fingerprint(ProtectedAction::CreateAgent, &what));
    }

    #[test]
    fn a_request_becomes_a_decision_the_desk_can_hold() {
        let request = |request: Request, summary: String, detail: Vec<DetailField>| Approval {
            id: ApprovalId::new(),
            agent_id: AgentId::new(),
            group_id: GroupId::new(),
            run_id: RunId::new(),
            request,
            summary,
            detail,
            state: ApprovalState::Pending,
            answer: None,
            created_at: 0,
            decided_at: None,
        };
        let permission = request(
            Request::Permission { action: ProtectedAction::ChangeSettings },
            "Helper wants to change the workspace's settings".into(),
            vec![DetailField::new("Relay depth", "8 → 12"), DetailField::new("Timeout", "60 → 90")],
        )
        .as_decision();
        assert_eq!(permission.question, "Helper wants to change the workspace's settings");
        assert_eq!(permission.context, "Relay depth: 8 → 12\nTimeout: 60 → 90");
        assert_eq!(permission.options, [ALLOW, DENY]);
        assert!(permission.clone().validate().is_ok());

        // A long question keeps its whole text behind the cut one, and a
        // question that took a written answer still takes one.
        let long = "Which of these? ".repeat(40);
        let question =
            request(Request::Question { options: vec![] }, long.clone(), vec![]).as_decision();
        assert!(question.question.chars().count() <= 300);
        assert!(question.context.starts_with(long.trim().get(..400).unwrap()));
        assert!(question.options.is_empty());
        assert!(question.validate().is_ok());

        let repeated = request(
            Request::Question { options: vec!["A".into(), "A".into()] },
            "Which?".into(),
            vec![],
        )
        .as_decision();
        assert!(repeated.options.is_empty(), "one distinct choice is not a choice");
        assert!(repeated.validate().is_ok());
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
