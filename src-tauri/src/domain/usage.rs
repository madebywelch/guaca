//! What model calls cost.
//!
//! Counts come from the provider and are never estimated here. A guessed token
//! count looks exactly like a real one on screen, and an operator watching a
//! crew work would have no way to tell which they were reading.

use serde::{Deserialize, Serialize};

use crate::domain::ids::{AgentId, GroupId, RunId};

/// One model call's cost, on its way to the store.
#[derive(Debug, Clone)]
pub struct UsageEntry {
    pub agent_id: AgentId,
    pub group_id: GroupId,
    pub run_id: RunId,
    pub model: String,
    pub prompt: u32,
    pub completion: u32,
    pub cost: Option<f64>,
}

/// A total, of whatever was summed.
#[derive(Debug, Clone, Copy, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Tokens {
    pub prompt: u64,
    pub completion: u64,
    /// Dollars, summed over the calls that were priced. None when none were.
    pub cost: Option<f64>,
    /// Model calls, not agent turns. One turn can make several, working through
    /// tool results, which is the whole reason the budget counts calls.
    pub calls: u64,
}

impl Tokens {
    pub fn total(&self) -> u64 {
        self.prompt + self.completion
    }
}

/// What a group has spent, addressed by group.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GroupUsage {
    pub group_id: GroupId,
    #[serde(flatten)]
    pub tokens: Tokens,
}

/// What a run cost, addressed by run.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RunUsage {
    pub run_id: RunId,
    #[serde(flatten)]
    pub tokens: Tokens,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_total_is_both_halves() {
        let tokens = Tokens { prompt: 1200, completion: 340, cost: None, calls: 3 };
        assert_eq!(tokens.total(), 1540);
    }

    #[test]
    fn group_usage_crosses_the_boundary_flat_and_in_camel_case() {
        // The UI reads these fields directly; a nested `tokens` object here and
        // a flat one in TypeScript is the kind of drift the contract test
        // cannot see.
        let json = serde_json::to_string(&GroupUsage {
            group_id: GroupId::new(),
            tokens: Tokens { prompt: 1, completion: 2, cost: Some(0.5), calls: 1 },
        })
        .unwrap();
        assert!(json.contains("\"groupId\""), "{json}");
        assert!(json.contains("\"prompt\":1"), "{json}");
        assert!(!json.contains("\"tokens\""), "flattened, not nested: {json}");
    }
}
