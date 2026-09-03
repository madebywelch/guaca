//! Newtype identifiers.
//!
//! These are separate types on purpose. An `AgentId` and a `RunId` are both
//! UUIDs underneath, and mixing them up is the kind of bug that only shows up
//! once messages are flying between agents. The type system is cheaper than
//! the debugging session.

use std::fmt;
use std::str::FromStr;

use serde::{Deserialize, Serialize};
use uuid::Uuid;

macro_rules! declare_id {
    ($name:ident, $prefix:literal) => {
        #[derive(
            Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize,
        )]
        #[serde(transparent)]
        pub struct $name(Uuid);

        impl $name {
            pub fn new() -> Self {
                Self(Uuid::new_v4())
            }

            pub const fn from_uuid(uuid: Uuid) -> Self {
                Self(uuid)
            }

            pub const fn as_uuid(&self) -> &Uuid {
                &self.0
            }

            /// Short, human-scannable form for log lines and UI affordances.
            pub fn short(&self) -> String {
                self.0.simple().to_string()[..8].to_string()
            }

            pub const PREFIX: &'static str = $prefix;
        }

        impl Default for $name {
            fn default() -> Self {
                Self::new()
            }
        }

        impl fmt::Display for $name {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                write!(f, "{}", self.0)
            }
        }

        impl FromStr for $name {
            type Err = uuid::Error;

            fn from_str(s: &str) -> Result<Self, Self::Err> {
                Ok(Self(Uuid::parse_str(s)?))
            }
        }
    };
}

declare_id!(AgentId, "agent");
declare_id!(ApprovalId, "approval");
declare_id!(ConnectorId, "connector");
declare_id!(EscalationId, "escalation");
declare_id!(GroupId, "group");
declare_id!(MessageId, "msg");
declare_id!(OccasionId, "occasion");
declare_id!(PluginId, "plugin");
declare_id!(RepositoryId, "repo");
declare_id!(RoutineId, "routine");
declare_id!(RunId, "run");

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ids_round_trip_through_strings() {
        let id = AgentId::new();
        assert_eq!(id, AgentId::from_str(&id.to_string()).unwrap());
    }

    #[test]
    fn ids_round_trip_through_json_as_bare_strings() {
        let id = MessageId::new();
        let json = serde_json::to_string(&id).unwrap();
        // `transparent` keeps the wire format a plain string, so the TypeScript
        // side sees `string` and not `{ "0": "..." }`.
        assert!(json.starts_with('"'), "expected a bare JSON string, got {json}");
        assert_eq!(id, serde_json::from_str::<MessageId>(&json).unwrap());
    }

    #[test]
    fn short_form_is_eight_hex_chars() {
        let id = RunId::new();
        let short = id.short();
        assert_eq!(short.len(), 8);
        assert!(short.chars().all(|c| c.is_ascii_hexdigit()));
    }

    #[test]
    fn distinct_ids_do_not_collide() {
        let a = AgentId::new();
        let b = AgentId::new();
        assert_ne!(a, b);
    }
}
