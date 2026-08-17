//! What the app knows about an agent's computer, independent of who runs it.
//!
//! A computer is a Guaca id, a provider, and whatever that provider needs to
//! find the machine again. The provider's own identifier and tokens are here
//! as data; driving the machine is `crate::computer`.

use serde::{Deserialize, Serialize};

use super::ids::{AgentId, ComputerId};

/// Who runs the machine. Stored as text, so it fails closed on a value this
/// build does not know rather than defaulting to one it does.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Provider {
    E2b,
}

impl Provider {
    pub fn as_str(&self) -> &'static str {
        match self {
            Provider::E2b => "e2b",
        }
    }

    pub fn parse(raw: &str) -> Option<Self> {
        match raw {
            "e2b" => Some(Provider::E2b),
            _ => None,
        }
    }
}

/// Where a row is in its life. `Provisioning` exists so a crash between making
/// the resource and writing it down leaves a trace to reconcile at startup;
/// `DeletePending` exists so a failed teardown is retried rather than leaked.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RecordState {
    Provisioning,
    Ready,
    DeletePending,
}

impl RecordState {
    pub fn as_str(&self) -> &'static str {
        match self {
            RecordState::Provisioning => "provisioning",
            RecordState::Ready => "ready",
            RecordState::DeletePending => "deletePending",
        }
    }

    pub fn parse(raw: &str) -> Option<Self> {
        match raw {
            "provisioning" => Some(RecordState::Provisioning),
            "ready" => Some(RecordState::Ready),
            "deletePending" => Some(RecordState::DeletePending),
            _ => None,
        }
    }
}

/// A token that reaches a machine. Deliberately without `Serialize` and with a
/// `Debug` that says nothing: the two ways a secret has left a process before
/// are a derived debug print in a log line and a struct that happened to be
/// serialisable reaching IPC.
#[derive(Clone, PartialEq, Eq, Default)]
pub struct Secret(String);

impl Secret {
    pub fn new(raw: impl Into<String>) -> Self {
        Self(raw.into())
    }

    pub fn expose(&self) -> &str {
        &self.0
    }

    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }
}

impl std::fmt::Debug for Secret {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("[redacted]")
    }
}

/// One row of `computers`. Cannot derive `Serialize` because `Secret` does
/// not, which is the point: the redacted `Computer` in `crate::computer` is
/// what crosses IPC.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ComputerRecord {
    pub id: ComputerId,
    pub agent_id: AgentId,
    pub provider: Provider,
    /// The provider's own identifier. Absent while provisioning.
    pub provider_id: Option<String>,
    /// What commands are sent with. E2B's envd token; empty for local providers.
    pub control_secret: Secret,
    /// What the viewer is sent with. E2B's traffic token; empty for local providers.
    pub viewer_secret: Secret,
    /// The pinned image digest, for providers that have one. Empty for E2B.
    pub image_ref: String,
    pub state: RecordState,
    pub last_used_at: i64,
    pub created_at: i64,
    pub updated_at: i64,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_provider_name_that_is_not_known_fails_closed() {
        // A row written by a newer build names a provider this one cannot
        // drive. Guessing would let it operate on somebody else's resource.
        assert_eq!(Provider::parse("e2b"), Some(Provider::E2b));
        assert_eq!(Provider::parse("E2B"), None);
        assert_eq!(Provider::parse("appleContainer"), None, "not in this PR");
        assert_eq!(Provider::E2b.as_str(), "e2b");
    }

    #[test]
    fn a_record_state_round_trips_and_an_unknown_one_is_refused() {
        for state in [RecordState::Provisioning, RecordState::Ready, RecordState::DeletePending] {
            assert_eq!(RecordState::parse(state.as_str()), Some(state));
        }
        assert_eq!(RecordState::parse("ready "), None);
    }

    #[test]
    fn a_secret_does_not_print_itself() {
        let secret = Secret::new("tok-123");
        assert_eq!(format!("{secret:?}"), "[redacted]");
        assert_eq!(secret.expose(), "tok-123");
        assert!(Secret::new("").is_empty());
    }
}
