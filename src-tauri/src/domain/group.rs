//! Groups: the isolation boundary between agents.
//!
//! Every agent belongs to exactly one group, and agents in different groups
//! cannot reach each other. That is not a UI filter. `directory` only lists
//! peers from the caller's own group, and a send addressed to a name outside it
//! is refused as an unknown recipient, so from inside a group the rest of the
//! roster does not exist.
//!
//! Agents are never told what a group is. There is no group tool, nothing in
//! the system prompt, and no way to enumerate or address one. An agent that
//! cannot observe the boundary cannot be talked across it, which is a stronger
//! guarantee than a rule in a prompt and needs no cooperation from the model.
//!
//! The operator is not in a group. A human can open any agent and talk to it;
//! the wall is between agents.
//!
//! There is always at least one group, created by the migration that introduced
//! them, so "no group" is not a state the rest of the app has to handle.

use serde::{Deserialize, Serialize};

use super::ids::GroupId;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Group {
    pub id: GroupId,
    pub name: String,
    /// How many live agents are in it. Carried on the card because every screen
    /// that lists groups wants it, and counting per group in the UI would mean
    /// walking the whole roster once per group.
    pub agent_count: u32,
    pub created_at: i64,
}

/// Fields an operator can set. Separate from `Group` so `id` and timestamps
/// cannot be forged across the IPC boundary.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GroupDraft {
    pub name: String,
}

#[derive(Debug, thiserror::Error, PartialEq)]
pub enum GroupError {
    #[error("group name must not be blank")]
    BlankName,
    #[error("group name must be {max} characters or fewer")]
    NameTooLong { max: usize },
    #[error("a group named {name:?} already exists")]
    DuplicateName { name: String },
}

pub const MAX_GROUP_NAME_LEN: usize = 48;

impl GroupDraft {
    pub fn validate(&self) -> Result<String, GroupError> {
        let name = self.name.trim();
        if name.is_empty() {
            return Err(GroupError::BlankName);
        }
        if name.chars().count() > MAX_GROUP_NAME_LEN {
            return Err(GroupError::NameTooLong { max: MAX_GROUP_NAME_LEN });
        }
        Ok(name.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn validate_trims() {
        let draft = GroupDraft { name: "  Research  ".into() };
        assert_eq!(draft.validate().unwrap(), "Research");
    }

    #[test]
    fn blank_name_is_rejected() {
        assert_eq!(GroupDraft { name: "   ".into() }.validate(), Err(GroupError::BlankName));
    }

    #[test]
    fn overlong_name_is_rejected_by_character_count_not_bytes() {
        // Same reasoning as agent names: 40 emoji is 40 characters but well
        // over 48 bytes, and counting bytes would reject a legal name.
        let draft = GroupDraft { name: "\u{1f951}".repeat(40) };
        assert!(draft.validate().is_ok());
        let draft = GroupDraft { name: "\u{1f951}".repeat(49) };
        assert_eq!(draft.validate(), Err(GroupError::NameTooLong { max: MAX_GROUP_NAME_LEN }));
    }
}
