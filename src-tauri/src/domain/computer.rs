//! What the app knows about an agent's computer, independent of who runs it.
//!
//! A computer is a Guaca id, a provider, and whatever that provider needs to
//! find the machine again. The provider's own identifier and tokens are here
//! as data; driving the machine is `crate::computer`.

use serde::{Deserialize, Deserializer, Serialize, Serializer};

use super::ids::{AgentId, ComputerId};

/// Who runs the machine. Stored as text, so it fails closed on a value this
/// build does not know rather than defaulting to one it does.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Provider {
    E2b,
    AppleContainer,
}

impl Provider {
    /// The stored form. Identical to the serialized form on purpose: two
    /// spellings of one token is a mapping table waiting to go wrong.
    pub fn as_str(self) -> &'static str {
        match self {
            Provider::E2b => "e2b",
            Provider::AppleContainer => "appleContainer",
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "e2b" => Some(Provider::E2b),
            "appleContainer" => Some(Provider::AppleContainer),
            _ => None,
        }
    }
}

/// What the operator picked in settings, which is a choice rather than an
/// answer: `Automatic` is resolved to a `Provider` once, when a computer is
/// made, and it is the computer's row that records who actually runs it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ProviderChoice {
    #[default]
    Automatic,
    Provider(Provider),
}

impl ProviderChoice {
    pub fn as_str(self) -> &'static str {
        match self {
            ProviderChoice::Automatic => "automatic",
            ProviderChoice::Provider(provider) => provider.as_str(),
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "automatic" => Some(ProviderChoice::Automatic),
            other => Provider::parse(other).map(ProviderChoice::Provider),
        }
    }
}

/// Whether an agent can be given a machine, and when it cannot, what it should
/// say about that.
///
/// Deliberately not a bool. The prompt built from it is read by a model that
/// has to tell an operator something they can act on, and "there is no
/// computer" has several different causes with several different next steps: a
/// Mac that cannot run the local runtime is not one that has not installed it,
/// and a provider the operator named and left unconfigured is neither. One
/// sentence covering all of them is wrong in most of them.
///
/// The two clauses are written for a model rather than for Settings: no CLI
/// commands, no paths, no version numbers. What the operator has to type is on
/// the screen that can also show it to them.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ComputerAccess {
    Available,
    Unavailable {
        /// Why there is none, as a clause that follows a colon: "no computer
        /// provider is set up on this Mac".
        because: String,
        /// What would give it one, as a clause that follows "tell the operator
        /// that": "adding an E2B key in Settings would give you one".
        remedy: String,
    },
}

impl ComputerAccess {
    pub fn unavailable(because: impl Into<String>, remedy: impl Into<String>) -> Self {
        Self::Unavailable { because: because.into(), remedy: remedy.into() }
    }

    /// What the tool list is built from: the four that need a machine are
    /// offered on exactly this answer.
    pub fn is_available(&self) -> bool {
        matches!(self, ComputerAccess::Available)
    }
}

// Written by hand rather than derived, because the derived form of a newtype
// variant is an object and this has to be the one flat token `as_str` returns:
// the same string is in the config file, in the UI, and in a `computers` row.
impl Serialize for ProviderChoice {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(self.as_str())
    }
}

impl<'de> Deserialize<'de> for ProviderChoice {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let raw = String::deserialize(deserializer)?;
        // Refused rather than read as `Automatic`. A settings file written by a
        // newer build names a provider on purpose, and quietly choosing another
        // one would run an agent's machine somewhere nobody asked for.
        Self::parse(&raw).ok_or_else(|| {
            serde::de::Error::custom(format!(
                "unknown computer provider {raw:?}; expected automatic, appleContainer or e2b"
            ))
        })
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
    pub fn as_str(self) -> &'static str {
        match self {
            RecordState::Provisioning => "provisioning",
            RecordState::Ready => "ready",
            RecordState::DeletePending => "deletePending",
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        match value {
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
        assert_eq!(Provider::parse("appleContainer"), Some(Provider::AppleContainer));
        assert_eq!(Provider::parse("apple-container"), None);
        assert_eq!(Provider::parse("docker"), None, "not in this PR");
        assert_eq!(Provider::E2b.as_str(), "e2b");
        assert_eq!(Provider::AppleContainer.as_str(), "appleContainer");

        // One spelling, so a row written by the runtime and a provider read by
        // the UI can never mean different things.
        for provider in [Provider::E2b, Provider::AppleContainer] {
            assert_eq!(serde_json::to_value(provider).unwrap().as_str(), Some(provider.as_str()));
        }
    }

    #[test]
    fn a_provider_choice_round_trips_and_an_unknown_one_is_refused() {
        let choices = [
            ProviderChoice::Automatic,
            ProviderChoice::Provider(Provider::AppleContainer),
            ProviderChoice::Provider(Provider::E2b),
        ];
        for choice in choices {
            assert_eq!(ProviderChoice::parse(choice.as_str()), Some(choice));
            // The same pinning as `Provider`: a choice stored in the config
            // file and one read by the UI are the same token or neither works.
            assert_eq!(serde_json::to_value(choice).unwrap().as_str(), Some(choice.as_str()));
            assert_eq!(
                serde_json::from_value::<ProviderChoice>(serde_json::json!(choice.as_str()))
                    .unwrap(),
                choice
            );
        }
        assert_eq!(ProviderChoice::Automatic.as_str(), "automatic");
        assert_eq!(ProviderChoice::parse("docker"), None, "PR C, not this one");
        assert_eq!(ProviderChoice::parse("Automatic"), None);

        // Refused rather than quietly read as automatic: a config naming a
        // provider this build cannot drive is worth saying out loud.
        let err = serde_json::from_str::<ProviderChoice>(r#""docker""#).unwrap_err().to_string();
        assert!(err.contains("docker"), "the message must name the value it refused: {err}");
    }

    #[test]
    fn a_record_state_round_trips_and_an_unknown_one_is_refused() {
        for state in [RecordState::Provisioning, RecordState::Ready, RecordState::DeletePending] {
            assert_eq!(RecordState::parse(state.as_str()), Some(state));
        }
        assert_eq!(RecordState::parse("ready "), None);

        // Pinned, because renaming both halves together round-trips green while
        // every row already in the database stops parsing.
        assert_eq!(RecordState::Provisioning.as_str(), "provisioning");
        assert_eq!(RecordState::Ready.as_str(), "ready");
        assert_eq!(RecordState::DeletePending.as_str(), "deletePending");
    }

    #[test]
    fn a_secret_does_not_print_itself() {
        let secret = Secret::new("tok-123");
        assert_eq!(format!("{secret:?}"), "[redacted]");
        assert_eq!(secret.expose(), "tok-123");
        assert!(Secret::new("").is_empty());
    }
}
