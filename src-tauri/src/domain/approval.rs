use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use super::decision::DecisionRequest;
use super::ids::{AgentId, ApprovalId, GroupId, RunId};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase", rename_all_fields = "camelCase")]
pub enum Request {
    Permission { action: ProtectedAction },
    Question { options: Vec<String> },
}

impl Request {
    pub fn as_str(&self) -> &'static str {
        match self {
            Request::Permission { action } => action.as_str(),
            Request::Question { .. } => QUESTION,
        }
    }

    pub fn action(&self) -> Option<ProtectedAction> {
        match self {
            Request::Permission { action } => Some(*action),
            Request::Question { .. } => None,
        }
    }
}

pub const QUESTION: &str = "question";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ProtectedAction {
    CreateAgent,
    ActOnBehalf,
    ChangeSettings,
}

impl ProtectedAction {
    pub fn stands(self) -> bool {
        matches!(self, ProtectedAction::CreateAgent)
    }

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

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Decision {
    Allow,
    AlwaysAllow,
    Deny,
}

impl Decision {
    pub fn grants(self) -> bool {
        matches!(self, Decision::Allow | Decision::AlwaysAllow)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ApprovalState {
    Pending,
    Allow,
    AlwaysAllow,
    Deny,
    Answered,
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

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Approval {
    pub id: ApprovalId,
    pub agent_id: AgentId,
    pub group_id: GroupId,
    pub run_id: RunId,
    pub request: Request,
    pub summary: String,
    pub detail: Vec<DetailField>,
    pub state: ApprovalState,
    pub answer: Option<String>,
    pub created_at: i64,
    pub decided_at: Option<i64>,
    pub lapses_at: Option<i64>,
}

pub const ALLOW: &str = "Allow";
pub const DENY: &str = "Deny";

pub const GRANT_LIFETIME_MS: i64 = 24 * 60 * 60 * 1000;

pub fn fingerprint(action: ProtectedAction, what: &serde_json::Value) -> String {
    format!("{:x}", Sha256::digest(format!("{}\n{what}", action.as_str()).as_bytes()))
}

impl Approval {
    pub fn lapsing_after(self, window: std::time::Duration) -> Self {
        let lapses_at = (self.state == ApprovalState::Pending)
            .then(|| self.created_at + window.as_millis() as i64);
        Self { lapses_at, ..self }
    }

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
            lapses_at: None,
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
        assert!(serde_json::from_str::<Decision>("\"pending\"").is_err());
        assert!(serde_json::from_str::<Decision>("\"expired\"").is_err());
        assert_eq!(
            serde_json::from_str::<Decision>("\"alwaysAllow\"").unwrap(),
            Decision::AlwaysAllow
        );
    }
}
