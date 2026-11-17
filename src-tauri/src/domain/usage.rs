use serde::{Deserialize, Serialize};

use crate::domain::ids::{AgentId, GroupId, RunId};

#[derive(Debug, Clone)]
pub struct UsageEntry {
    pub agent_id: AgentId,
    pub group_id: GroupId,
    pub run_id: RunId,
    pub model: String,
    pub prompt: u32,
    pub completion: u32,
    pub cached: u32,
    pub cost: Option<f64>,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Tokens {
    pub prompt: u64,
    pub completion: u64,
    pub cost: Option<f64>,
    pub calls: u64,
}

impl Tokens {
    pub fn total(&self) -> u64 {
        self.prompt + self.completion
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GroupUsage {
    pub group_id: GroupId,
    #[serde(flatten)]
    pub tokens: Tokens,
}

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
