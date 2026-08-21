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

/// What the UI sees. The API key is never on it: only whether one is set and a
/// hint, the same shape the app-wide settings use, so a group's key cannot be
/// read back out through the IPC boundary once written.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Group {
    pub id: GroupId,
    pub name: String,
    /// `None` means inherit the app default. An empty string is a value an
    /// operator deliberately blanked, so the two are kept distinct.
    pub base_url: Option<String>,
    pub default_model: Option<String>,
    pub api_key_set: bool,
    pub api_key_hint: String,
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
    #[serde(default)]
    pub base_url: Option<String>,
    #[serde(default)]
    pub default_model: Option<String>,
    /// Absent leaves the stored key alone; `Some("")` clears it. Without that
    /// distinction the UI could not render a redacted key without erasing it on
    /// the next save.
    #[serde(default)]
    pub api_key: Option<String>,
}

#[derive(Debug, thiserror::Error, PartialEq)]
pub enum GroupError {
    #[error("group name must not be blank")]
    BlankName,
    #[error("group name must be {max} characters or fewer")]
    NameTooLong { max: usize },
    #[error("a group named {name:?} already exists")]
    DuplicateName { name: String },
    // Carries the message rather than the error: ConfigError wraps io::Error,
    // which is not comparable, and this enum is worth being able to assert on.
    #[error("that inference endpoint is not usable: {0}")]
    BadEndpoint(String),
}

pub const MAX_GROUP_NAME_LEN: usize = 48;

/// The validated form of a draft, ready to store.
#[derive(Debug, Clone, PartialEq)]
pub struct CleanGroup {
    pub name: String,
    /// `Some(None)` clears the override, `None` leaves it as it was.
    pub base_url: Option<Option<String>>,
    pub default_model: Option<Option<String>>,
    pub api_key: Option<Option<String>>,
}

/// Blank input means "inherit", so it is stored as NULL rather than "".
fn override_of(value: &Option<String>) -> Option<Option<String>> {
    value.as_ref().map(|raw| {
        let trimmed = raw.trim();
        if trimmed.is_empty() {
            None
        } else {
            Some(trimmed.to_string())
        }
    })
}

impl GroupDraft {
    pub fn validate(&self) -> Result<CleanGroup, GroupError> {
        let name = self.name.trim();
        if name.is_empty() {
            return Err(GroupError::BlankName);
        }
        if name.chars().count() > MAX_GROUP_NAME_LEN {
            return Err(GroupError::NameTooLong { max: MAX_GROUP_NAME_LEN });
        }

        // A base URL that cannot be parsed would fail on every turn of every
        // agent in the group, so it is rejected at the edit instead.
        let base_url = match override_of(&self.base_url) {
            Some(Some(raw)) => Some(Some(
                crate::config::normalize_base_url(&raw)
                    .map_err(|e| GroupError::BadEndpoint(e.to_string()))?,
            )),
            other => other,
        };

        Ok(CleanGroup {
            name: name.to_string(),
            base_url,
            default_model: override_of(&self.default_model),
            api_key: override_of(&self.api_key),
        })
    }
}

/// One group's inference overrides, resolved against the app defaults.
///
/// Never crosses IPC: it carries the key in plaintext because the runtime needs
/// it to make a request.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct GroupInference {
    pub base_url: Option<String>,
    pub api_key: Option<String>,
    pub default_model: Option<String>,
}

impl GroupInference {
    /// Layers this group over the app-wide settings. Anything the group does
    /// not set is inherited, so a group with no overrides behaves exactly as
    /// before groups existed.
    pub fn apply(&self, base: &crate::config::InferenceConfig) -> crate::config::InferenceConfig {
        let mut out = base.clone();

        // A group that names its own endpoint or its own key has chosen an
        // endpoint, and an endpoint is not where a subscription is spent. Left
        // alone, such a group would inherit the app's subscription and have both
        // of its overrides quietly ignored: the operator would be looking at a
        // key and a URL that nothing used, with no error to explain it.
        //
        // Only these two flip it. Overriding the model alone is a group asking
        // for a different model on whatever the app is already paying with,
        // which is the common case and the one that has to keep working.
        //
        // First, because the model that gets collapsed below depends on it.
        if self.base_url.is_some() || self.api_key.is_some() {
            out.provider = crate::config::Provider::Compatible;
        }

        // The app keeps a model per provider, and everything downstream of here
        // reads one field. Collapsing to the resolved provider's model, after
        // the flip above and before the group's own override, means each of the
        // three cases lands on the model that provider can actually run.
        out.default_model = out.active_model().to_string();

        if let Some(url) = &self.base_url {
            out.base_url = url.clone();
        }
        if let Some(key) = &self.api_key {
            out.api_key = key.clone();
        }
        if let Some(model) = &self.default_model {
            out.default_model = model.clone();
        }

        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn draft(name: &str) -> GroupDraft {
        GroupDraft { name: name.into(), base_url: None, default_model: None, api_key: None }
    }

    #[test]
    fn validate_trims() {
        assert_eq!(draft("  Research  ").validate().unwrap().name, "Research");
    }

    #[test]
    fn blank_name_is_rejected() {
        assert_eq!(draft("   ").validate(), Err(GroupError::BlankName));
    }

    #[test]
    fn overlong_name_is_rejected_by_character_count_not_bytes() {
        // Same reasoning as agent names: 40 emoji is 40 characters but well
        // over 48 bytes, and counting bytes would reject a legal name.
        assert!(draft(&"\u{1f951}".repeat(40)).validate().is_ok());
        assert_eq!(
            draft(&"\u{1f951}".repeat(49)).validate(),
            Err(GroupError::NameTooLong { max: MAX_GROUP_NAME_LEN })
        );
    }

    #[test]
    fn a_blanked_override_is_stored_as_inherit_not_as_empty() {
        // Otherwise clearing the field in the UI would pin every agent in the
        // group to an empty model rather than falling back to the app default.
        let mut d = draft("Research");
        d.default_model = Some("   ".into());
        assert_eq!(d.validate().unwrap().default_model, Some(None));
    }

    #[test]
    fn an_absent_override_leaves_the_stored_value_alone() {
        assert_eq!(draft("Research").validate().unwrap().default_model, None);
    }

    #[test]
    fn a_bad_endpoint_is_rejected_at_the_edit_not_on_every_turn() {
        let mut d = draft("Research");
        d.base_url = Some("not-a-url".into());
        assert!(matches!(d.validate(), Err(GroupError::BadEndpoint(_))));
    }

    #[test]
    fn overrides_layer_over_the_app_defaults() {
        let base = crate::config::InferenceConfig::default();
        let empty = GroupInference::default();
        assert_eq!(empty.apply(&base), base, "a group with no overrides changes nothing");

        let pinned =
            GroupInference { default_model: Some("local/qwen".into()), ..Default::default() };
        let resolved = pinned.apply(&base);
        assert_eq!(resolved.default_model, "local/qwen");
        assert_eq!(resolved.base_url, base.base_url, "an unset field still inherits");
    }

    /// App-wide settings with a subscription chosen, and a model set for each
    /// provider so it is visible which one a resolution picked.
    fn on_subscription() -> crate::config::InferenceConfig {
        crate::config::InferenceConfig {
            provider: crate::config::Provider::Chatgpt,
            default_model: "anthropic/claude-sonnet-4.5".into(),
            subscription_model: "gpt-5.6-luna".into(),
            api_key: "app-key".into(),
            ..Default::default()
        }
    }

    #[test]
    fn a_group_with_no_overrides_uses_the_subscription_and_its_own_model() {
        let resolved = GroupInference::default().apply(&on_subscription());
        assert_eq!(resolved.provider, crate::config::Provider::Chatgpt);
        // The one field everything downstream reads has to be the subscription's
        // model, not the endpoint's, or every agent fails on a model the backend
        // has never heard of.
        assert_eq!(resolved.default_model, "gpt-5.6-luna");
    }

    #[test]
    fn a_group_that_only_overrides_the_model_stays_on_the_subscription() {
        // The common case: a crew that wants a cheaper model on whatever the app
        // is already paying with.
        let resolved =
            GroupInference { default_model: Some("gpt-5.4-mini".into()), ..Default::default() }
                .apply(&on_subscription());
        assert_eq!(resolved.provider, crate::config::Provider::Chatgpt);
        assert_eq!(resolved.default_model, "gpt-5.4-mini");
    }

    #[test]
    fn a_group_that_names_its_own_endpoint_leaves_the_subscription() {
        let resolved = GroupInference {
            base_url: Some("http://localhost:1234/v1".into()),
            ..Default::default()
        }
        .apply(&on_subscription());

        // Otherwise the group's endpoint is set, visible in the UI, and used by
        // nothing, with no error anywhere to explain it.
        assert_eq!(resolved.provider, crate::config::Provider::Compatible);
        assert_eq!(resolved.base_url, "http://localhost:1234/v1");
        // And it gets the endpoint's model, not the subscription's, because the
        // provider flip happens before the model is collapsed.
        assert_eq!(resolved.default_model, "anthropic/claude-sonnet-4.5");
    }

    #[test]
    fn a_group_that_names_its_own_key_also_leaves_the_subscription() {
        let resolved = GroupInference { api_key: Some("group-key".into()), ..Default::default() }
            .apply(&on_subscription());
        assert_eq!(resolved.provider, crate::config::Provider::Compatible);
        assert_eq!(resolved.api_key, "group-key");
        assert_eq!(resolved.default_model, "anthropic/claude-sonnet-4.5");
    }

    #[test]
    fn a_group_on_its_own_endpoint_can_still_name_its_own_model() {
        let resolved = GroupInference {
            base_url: Some("http://localhost:1234/v1".into()),
            default_model: Some("local/qwen".into()),
            ..Default::default()
        }
        .apply(&on_subscription());
        assert_eq!(resolved.provider, crate::config::Provider::Compatible);
        assert_eq!(resolved.default_model, "local/qwen");
    }

    #[test]
    fn a_group_changes_nothing_when_the_app_is_on_an_endpoint() {
        // The behaviour that existed before subscriptions, unchanged.
        let base = crate::config::InferenceConfig::default();
        assert_eq!(GroupInference::default().apply(&base), base);
    }
}
