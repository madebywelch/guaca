use serde::{Deserialize, Serialize};

use super::cut_to;
use super::ids::{AgentId, EscalationId, GroupId, RunId};

pub const MAX_SUMMARY: usize = 400;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Escalation {
    pub id: EscalationId,
    pub agent_id: AgentId,
    pub group_id: GroupId,
    pub run_id: RunId,
    pub summary: String,
    pub raised_at: i64,
    pub said_at: i64,
    pub times: u32,
    pub cleared_at: Option<i64>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Raised {
    First(Escalation),
    Again(Escalation),
}

impl Raised {
    pub fn escalation(&self) -> &Escalation {
        match self {
            Raised::First(one) | Raised::Again(one) => one,
        }
    }
}

pub fn store_as(summary: &str) -> (String, bool) {
    cut_to(summary, MAX_SUMMARY)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_summary_within_the_cap_is_stored_whole() {
        let (body, cut) = store_as("  the deploy needs a key only you have  ");
        assert_eq!(body, "the deploy needs a key only you have");
        assert!(!cut);
    }

    #[test]
    fn an_oversized_summary_is_cut_and_says_so() {
        let (body, cut) = store_as(&"word ".repeat(200));
        assert!(cut);
        assert!(body.chars().count() <= MAX_SUMMARY);
    }

    #[test]
    fn both_outcomes_carry_the_row() {
        let one = Escalation {
            id: EscalationId::new(),
            agent_id: AgentId::new(),
            group_id: GroupId::new(),
            run_id: RunId::new(),
            summary: "stuck".into(),
            raised_at: 1,
            said_at: 1,
            times: 1,
            cleared_at: None,
        };
        assert_eq!(Raised::First(one.clone()).escalation(), &one);
        assert_eq!(Raised::Again(one.clone()).escalation(), &one);
    }
}
