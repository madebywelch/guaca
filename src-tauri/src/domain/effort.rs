//! A saved effort is independent of a model override. Auto asks the model to
//! choose; an absent override inherits from the enclosing group or app.
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ReasoningEffort {
    #[default]
    Auto,
    None,
    Minimal,
    Low,
    Medium,
    High,
    Xhigh,
    Max,
    Ultra,
}

impl ReasoningEffort {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Auto => "auto",
            Self::None => "none",
            Self::Minimal => "minimal",
            Self::Low => "low",
            Self::Medium => "medium",
            Self::High => "high",
            Self::Xhigh => "xhigh",
            Self::Max => "max",
            Self::Ultra => "ultra",
        }
    }

    pub fn wire(self) -> Option<Self> {
        (self != Self::Auto).then_some(self)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn invalid_efforts_are_rejected_and_auto_is_distinct_from_no_reasoning() {
        assert!(serde_json::from_str::<ReasoningEffort>(r#""hgh""#).is_err());
        assert_eq!(ReasoningEffort::Auto.wire(), None);
        assert_eq!(ReasoningEffort::None.wire(), Some(ReasoningEffort::None));
    }
}
