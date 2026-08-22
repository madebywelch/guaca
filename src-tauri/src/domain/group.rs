//! Groups: the isolation boundary between agents, and the settings they run on.
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
//!
//! A group is also where a crew's settings live. App settings are defaults: how
//! a turn is paid for, which model answers it, and how far a conversation may
//! run are all decided per group, and only fall back to the app when the group
//! says nothing. `None` is that silence throughout, at every layer: in the
//! draft the operator sends, in the columns, and in the resolved overrides. An
//! empty string is a field an operator blanked, which means inherit too, and is
//! normalised to `None` on the way in so the two can never disagree.

use serde::{Deserialize, Serialize};

use super::ids::GroupId;
use crate::config::{InferenceConfig, Provider};
use crate::runtime::guard::GuardLimits;

/// What the UI sees. The API key is never on it: only whether one is set and a
/// hint, the same shape the app-wide settings use, so a group's key cannot be
/// read back out through the IPC boundary once written.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Group {
    pub id: GroupId,
    pub name: String,
    /// Everything about how this group's turns are paid for and answered.
    /// Every field `None` is a group that runs on the app's settings.
    pub inference: InferenceOverrides,
    pub api_key_set: bool,
    pub api_key_hint: String,
    /// How far a conversation started in this group may run.
    pub limits: GroupLimits,
    /// How many live agents are in it. Carried on the card because every screen
    /// that lists groups wants it, and counting per group in the UI would mean
    /// walking the whole roster once per group.
    pub agent_count: u32,
    pub created_at: i64,
}

/// The inference settings an operator can put on a group. `None` inherits.
///
/// The API key is deliberately not here. It is the one setting the UI cannot
/// read back, so it has its own rule on the way in (absent keeps the stored
/// one), and this struct is the shape where absent means inherit. Keeping the
/// two rules in two types is what stops a redacted key ever being written back
/// as a value.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct InferenceOverrides {
    /// Which of the two ways a turn is paid for. A group can pay with the
    /// operator's ChatGPT sign-in while another pays with a key, because the
    /// sign-in is one credential on this machine and choosing it is a setting
    /// rather than a second account.
    pub provider: Option<Provider>,
    pub base_url: Option<String>,
    /// The model used when a key is paying.
    pub default_model: Option<String>,
    /// The model used when the subscription is paying.
    ///
    /// Two fields for the same reason the app keeps two: the providers have
    /// disjoint model names, and a group that tries the subscription for an
    /// hour must find its endpoint model where it left it.
    pub subscription_model: Option<String>,
    pub request_timeout_secs: Option<u64>,
}

/// Per-field overrides of the app's loop guard. `None` inherits.
///
/// Group-scoped because a limit is a statement about one crew's work. A pair of
/// agents drafting a document need a handful of model calls; a crew working a
/// browser through a long form needs an order of magnitude more, and one number
/// for both means either the first is an open wallet or the second stops
/// halfway and reports nothing.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct GroupLimits {
    pub max_hops: Option<u16>,
    pub max_steps_per_run: Option<u32>,
    pub max_fanout_per_call: Option<usize>,
    pub max_sends_per_pair: Option<u32>,
    pub max_tool_rounds: Option<u16>,
}

impl GroupLimits {
    /// Layers this group's limits over the app's.
    pub fn apply(&self, base: GuardLimits) -> GuardLimits {
        GuardLimits {
            max_hops: self.max_hops.unwrap_or(base.max_hops),
            max_steps_per_run: self.max_steps_per_run.unwrap_or(base.max_steps_per_run),
            max_fanout_per_call: self.max_fanout_per_call.unwrap_or(base.max_fanout_per_call),
            max_sends_per_pair: self.max_sends_per_pair.unwrap_or(base.max_sends_per_pair),
            max_tool_rounds: self.max_tool_rounds.unwrap_or(base.max_tool_rounds),
        }
    }

    /// Clamps each override into the range the runtime survives.
    ///
    /// Done at the edit rather than only at the use, so the number the operator
    /// reads back after saving is the number their agents will run on. The
    /// ranges are `GuardLimits::sanitized`'s and are not restated here: a group
    /// limit that could be clamped differently from the app's would be a second
    /// definition of the same rule, drifting from the first.
    pub fn sanitized(self) -> Self {
        let full = self.apply(GuardLimits::default()).sanitized();
        Self {
            max_hops: self.max_hops.map(|_| full.max_hops),
            max_steps_per_run: self.max_steps_per_run.map(|_| full.max_steps_per_run),
            max_fanout_per_call: self.max_fanout_per_call.map(|_| full.max_fanout_per_call),
            max_sends_per_pair: self.max_sends_per_pair.map(|_| full.max_sends_per_pair),
            max_tool_rounds: self.max_tool_rounds.map(|_| full.max_tool_rounds),
        }
    }
}

/// Fields an operator can set. Separate from `Group` so `id` and timestamps
/// cannot be forged across the IPC boundary.
///
/// Each block is all-or-nothing: absent leaves every override in it exactly as
/// it was, and present replaces the lot, with a null field meaning inherit.
/// One rule per block rather than per field, so a caller cannot half-write a
/// group's settings and a UI that renders what it read back cannot clear a
/// field by forgetting to mention it.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GroupDraft {
    pub name: String,
    #[serde(default)]
    pub inference: Option<InferenceOverrides>,
    /// Absent leaves the stored key alone; `Some("")` clears it. Without that
    /// distinction the UI could not render a redacted key without erasing it on
    /// the next save.
    #[serde(default)]
    pub api_key: Option<String>,
    #[serde(default)]
    pub limits: Option<GroupLimits>,
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
#[derive(Debug, Clone, Default, PartialEq)]
pub struct CleanGroup {
    pub name: String,
    /// `None` leaves the stored overrides alone.
    pub inference: Option<InferenceOverrides>,
    /// `Some(None)` clears the key, `None` leaves it as it was.
    pub api_key: Option<Option<String>>,
    pub limits: Option<GroupLimits>,
}

/// Blank input means "inherit", so it is stored as NULL rather than "".
fn override_of(value: &Option<String>) -> Option<String> {
    value.as_ref().map(|raw| raw.trim().to_string()).filter(|trimmed| !trimmed.is_empty())
}

/// The same range the app's own timeout is clamped to, and for the same reason:
/// five seconds cannot complete a call, and a quarter of an hour is already
/// longer than any model this app talks to takes to refuse.
fn clamp_timeout(secs: u64) -> u64 {
    secs.clamp(5, 900)
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

        let inference = match &self.inference {
            None => None,
            Some(raw) => Some(InferenceOverrides {
                provider: raw.provider,
                // A base URL that cannot be parsed would fail on every turn of
                // every agent in the group, so it is rejected at the edit
                // instead.
                base_url: match override_of(&raw.base_url) {
                    Some(url) => Some(
                        crate::config::normalize_base_url(&url)
                            .map_err(|e| GroupError::BadEndpoint(e.to_string()))?,
                    ),
                    None => None,
                },
                default_model: override_of(&raw.default_model),
                subscription_model: override_of(&raw.subscription_model),
                request_timeout_secs: raw.request_timeout_secs.map(clamp_timeout),
            }),
        };

        Ok(CleanGroup {
            name: name.to_string(),
            inference,
            // The one field where absent and blank differ, so it cannot go
            // through `override_of`: absent keeps the stored key.
            api_key: self.api_key.as_ref().map(|raw| {
                let trimmed = raw.trim();
                if trimmed.is_empty() {
                    None
                } else {
                    Some(trimmed.to_string())
                }
            }),
            limits: self.limits.map(GroupLimits::sanitized),
        })
    }
}

/// One group's inference overrides, resolved against the app defaults.
///
/// Never crosses IPC: it carries the key in plaintext because the runtime needs
/// it to make a request.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct GroupInference {
    pub overrides: InferenceOverrides,
    pub api_key: Option<String>,
}

impl GroupInference {
    /// Layers this group over the app-wide settings. Anything the group does
    /// not set is inherited, so a group with no overrides behaves exactly as
    /// before groups had settings of their own.
    pub fn apply(&self, base: &InferenceConfig) -> InferenceConfig {
        let mut out = base.clone();
        let over = &self.overrides;

        // First, because the model that gets collapsed below depends on it.
        //
        // Nothing is inferred here. A group that names an endpoint used to be
        // taken to mean it wanted one, because there was no way to say so; the
        // migration that added this column wrote that meaning down for every
        // group it was true of, so the guess has no work left to do. What it
        // cost was the sentence an operator could not act on: with a guess in
        // this path, "follow the app settings" and an endpoint in the box
        // disagree, and the endpoint silently wins.
        if let Some(provider) = over.provider {
            out.provider = provider;
        }

        if let Some(url) = &over.base_url {
            out.base_url = url.clone();
        }
        if let Some(key) = &self.api_key {
            out.api_key = key.clone();
        }
        if let Some(model) = &over.default_model {
            out.default_model = model.clone();
        }
        if let Some(model) = &over.subscription_model {
            out.subscription_model = model.clone();
        }
        if let Some(secs) = over.request_timeout_secs {
            out.request_timeout_secs = secs;
        }

        // Everything downstream of here reads one model field. Collapsing to
        // the resolved provider's model, after both overrides have landed,
        // means a group gets the model that the provider paying for its turns
        // can actually run.
        out.default_model = out.active_model().to_string();

        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn draft(name: &str) -> GroupDraft {
        GroupDraft { name: name.into(), inference: None, api_key: None, limits: None }
    }

    /// A draft that sets inference overrides, starting from all-inherit.
    fn with_inference(name: &str, over: InferenceOverrides) -> GroupDraft {
        GroupDraft { inference: Some(over), ..draft(name) }
    }

    #[test]
    fn a_draft_is_read_from_the_shape_the_webview_sends() {
        // Everything crossing IPC is camelCase, and `rename_all` on the outer
        // struct does not reach a nested one: each block carries its own. A
        // rename that only lands on one side is a field silently read as
        // "inherit", which looks exactly like an operator clearing it.
        let draft: GroupDraft = serde_json::from_str(
            r#"{
                 "name": "Research",
                 "inference": {
                   "provider": "chatgpt",
                   "baseUrl": "http://localhost:1234/v1",
                   "defaultModel": "local/qwen",
                   "subscriptionModel": "gpt-5.4",
                   "requestTimeoutSecs": 600
                 },
                 "apiKey": "sk-x",
                 "limits": { "maxStepsPerRun": 9, "maxToolRounds": 40 }
               }"#,
        )
        .unwrap();

        let clean = draft.validate().unwrap();
        let over = clean.inference.unwrap();
        assert_eq!(over.provider, Some(Provider::Chatgpt));
        assert_eq!(over.base_url.as_deref(), Some("http://localhost:1234/v1"));
        assert_eq!(over.subscription_model.as_deref(), Some("gpt-5.4"));
        assert_eq!(over.request_timeout_secs, Some(600));
        assert_eq!(clean.api_key, Some(Some("sk-x".into())));
        assert_eq!(clean.limits.unwrap().max_steps_per_run, Some(9));
    }

    #[test]
    fn a_group_is_written_in_the_shape_the_webview_reads() {
        let group = Group {
            id: GroupId::new(),
            name: "Research".into(),
            inference: InferenceOverrides {
                provider: Some(Provider::Compatible),
                subscription_model: Some("gpt-5.4".into()),
                request_timeout_secs: Some(600),
                ..Default::default()
            },
            api_key_set: false,
            api_key_hint: String::new(),
            limits: GroupLimits { max_steps_per_run: Some(9), ..Default::default() },
            agent_count: 0,
            created_at: 0,
        };

        let json = serde_json::to_value(&group).unwrap();
        assert_eq!(json["inference"]["provider"], "compatible");
        assert_eq!(json["inference"]["subscriptionModel"], "gpt-5.4");
        assert_eq!(json["inference"]["requestTimeoutSecs"], 600);
        assert_eq!(json["limits"]["maxStepsPerRun"], 9);
        assert!(json["limits"]["maxHops"].is_null(), "inherit crosses as null, not as a number");
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
        let d = with_inference(
            "Research",
            InferenceOverrides { default_model: Some("   ".into()), ..Default::default() },
        );
        assert_eq!(d.validate().unwrap().inference.unwrap().default_model, None);
    }

    #[test]
    fn an_absent_block_leaves_the_stored_overrides_alone() {
        let clean = draft("Research").validate().unwrap();
        assert_eq!(clean.inference, None);
        assert_eq!(clean.limits, None);
        assert_eq!(clean.api_key, None);
    }

    #[test]
    fn a_present_block_of_nulls_clears_every_override_in_it() {
        // The whole point of the block being all-or-nothing: a group put back
        // on the app settings is one save, not five.
        let clean = with_inference("Research", InferenceOverrides::default()).validate().unwrap();
        assert_eq!(clean.inference, Some(InferenceOverrides::default()));
    }

    #[test]
    fn a_blank_key_clears_it_and_an_absent_one_keeps_it() {
        assert_eq!(draft("Research").validate().unwrap().api_key, None);
        assert_eq!(
            GroupDraft { api_key: Some("  ".into()), ..draft("Research") }
                .validate()
                .unwrap()
                .api_key,
            Some(None)
        );
        assert_eq!(
            GroupDraft { api_key: Some(" sk-x ".into()), ..draft("Research") }
                .validate()
                .unwrap()
                .api_key,
            Some(Some("sk-x".into()))
        );
    }

    #[test]
    fn a_bad_endpoint_is_rejected_at_the_edit_not_on_every_turn() {
        let d = with_inference(
            "Research",
            InferenceOverrides { base_url: Some("not-a-url".into()), ..Default::default() },
        );
        assert!(matches!(d.validate(), Err(GroupError::BadEndpoint(_))));
    }

    #[test]
    fn a_timeout_is_clamped_at_the_edit_so_it_reads_back_as_what_will_be_used() {
        let d = with_inference(
            "Research",
            InferenceOverrides { request_timeout_secs: Some(99_999), ..Default::default() },
        );
        assert_eq!(d.validate().unwrap().inference.unwrap().request_timeout_secs, Some(900));
    }

    #[test]
    fn a_limit_is_clamped_at_the_edit_and_an_unset_one_stays_unset() {
        let d = GroupDraft {
            limits: Some(GroupLimits {
                max_steps_per_run: Some(9_000),
                max_hops: Some(0),
                ..Default::default()
            }),
            ..draft("Research")
        };
        let stored = d.validate().unwrap().limits.unwrap();
        assert_eq!(stored.max_steps_per_run, Some(500));
        assert_eq!(stored.max_hops, Some(1));
        // Clamping must not turn "inherit" into a number: a group that says
        // nothing about fan-out has to keep saying nothing after a save.
        assert_eq!(stored.max_fanout_per_call, None);
    }

    #[test]
    fn limits_layer_over_the_app_and_an_unset_one_inherits() {
        let app = GuardLimits { max_steps_per_run: 60, max_hops: 8, ..GuardLimits::default() };
        let tighter = GroupLimits { max_steps_per_run: Some(10), ..Default::default() };
        let resolved = tighter.apply(app);
        assert_eq!(resolved.max_steps_per_run, 10);
        assert_eq!(resolved.max_hops, 8);
        assert_eq!(GroupLimits::default().apply(app), app, "a silent group changes nothing");
    }

    #[test]
    fn overrides_layer_over_the_app_defaults() {
        let base = InferenceConfig::default();
        let empty = GroupInference::default();
        assert_eq!(empty.apply(&base), base, "a group with no overrides changes nothing");

        let pinned = GroupInference {
            overrides: InferenceOverrides {
                default_model: Some("local/qwen".into()),
                ..Default::default()
            },
            ..Default::default()
        };
        let resolved = pinned.apply(&base);
        assert_eq!(resolved.default_model, "local/qwen");
        assert_eq!(resolved.base_url, base.base_url, "an unset field still inherits");
    }

    #[test]
    fn a_group_can_run_on_its_own_timeout() {
        let base = InferenceConfig::default();
        let slow = GroupInference {
            overrides: InferenceOverrides { request_timeout_secs: Some(600), ..Default::default() },
            ..Default::default()
        };
        assert_eq!(slow.apply(&base).request_timeout_secs, 600);
    }

    /// App-wide settings with a subscription chosen, and a model set for each
    /// provider so it is visible which one a resolution picked.
    fn on_subscription() -> InferenceConfig {
        InferenceConfig {
            provider: Provider::Chatgpt,
            default_model: "anthropic/claude-sonnet-4.5".into(),
            subscription_model: "gpt-5.6-luna".into(),
            api_key: "app-key".into(),
            ..Default::default()
        }
    }

    /// A group that overrides one inference field and nothing else.
    fn only(over: InferenceOverrides) -> GroupInference {
        GroupInference { overrides: over, api_key: None }
    }

    #[test]
    fn a_group_with_no_overrides_uses_the_subscription_and_its_own_model() {
        let resolved = GroupInference::default().apply(&on_subscription());
        assert_eq!(resolved.provider, Provider::Chatgpt);
        // The one field everything downstream reads has to be the subscription's
        // model, not the endpoint's, or every agent fails on a model the backend
        // has never heard of.
        assert_eq!(resolved.default_model, "gpt-5.6-luna");
    }

    #[test]
    fn a_group_that_only_overrides_the_subscription_model_stays_on_the_subscription() {
        // The common case: a crew that wants a cheaper model on whatever the app
        // is already paying with.
        let resolved = only(InferenceOverrides {
            subscription_model: Some("gpt-5.4-mini".into()),
            ..Default::default()
        })
        .apply(&on_subscription());
        assert_eq!(resolved.provider, Provider::Chatgpt);
        assert_eq!(resolved.default_model, "gpt-5.4-mini");
    }

    #[test]
    fn a_model_set_for_the_other_provider_is_kept_and_not_used() {
        // Each model belongs to one provider. A group holding an endpoint model
        // while the subscription is paying is a group that has been switched
        // over, not a group whose model was ignored by mistake, and switching
        // back has to find it intact.
        let resolved = only(InferenceOverrides {
            default_model: Some("local/qwen".into()),
            ..Default::default()
        })
        .apply(&on_subscription());
        assert_eq!(resolved.default_model, "gpt-5.6-luna", "the subscription is paying");

        let back = only(InferenceOverrides {
            provider: Some(Provider::Compatible),
            default_model: Some("local/qwen".into()),
            ..Default::default()
        })
        .apply(&on_subscription());
        assert_eq!(back.default_model, "local/qwen");
    }

    #[test]
    fn a_group_on_its_own_endpoint_leaves_the_subscription() {
        let resolved = GroupInference {
            overrides: InferenceOverrides {
                provider: Some(Provider::Compatible),
                base_url: Some("http://localhost:1234/v1".into()),
                ..Default::default()
            },
            api_key: Some("group-key".into()),
        }
        .apply(&on_subscription());

        assert_eq!(resolved.provider, Provider::Compatible);
        assert_eq!(resolved.base_url, "http://localhost:1234/v1");
        assert_eq!(resolved.api_key, "group-key");
        // And it gets the endpoint's model, not the subscription's, because the
        // provider is resolved before the model is collapsed.
        assert_eq!(resolved.default_model, "anthropic/claude-sonnet-4.5");
    }

    #[test]
    fn an_endpoint_alone_does_not_decide_who_pays() {
        // The guess this replaced could not be argued with: an endpoint left in
        // the box outvoted the operator choosing to follow the app settings, and
        // there was nothing on screen to say why. The groups that meant it were
        // written down by the migration instead.
        let resolved = only(InferenceOverrides {
            base_url: Some("http://localhost:1234/v1".into()),
            ..Default::default()
        })
        .apply(&on_subscription());

        assert_eq!(resolved.provider, Provider::Chatgpt);
        assert_eq!(resolved.default_model, "gpt-5.6-luna");
    }

    #[test]
    fn a_group_on_the_subscription_keeps_the_endpoint_it_came_from() {
        // Kept rather than blanked, exactly as the app keeps its own, so a group
        // that tries the subscription for an hour finds its endpoint where it
        // left it.
        let resolved = GroupInference {
            overrides: InferenceOverrides {
                provider: Some(Provider::Chatgpt),
                base_url: Some("http://localhost:1234/v1".into()),
                subscription_model: Some("gpt-5.4".into()),
                ..Default::default()
            },
            api_key: Some("group-key".into()),
        }
        .apply(&InferenceConfig::default());

        assert_eq!(resolved.provider, Provider::Chatgpt);
        assert_eq!(resolved.default_model, "gpt-5.4");
        assert_eq!(resolved.base_url, "http://localhost:1234/v1", "kept for the way back");
    }

    #[test]
    fn a_group_can_pay_with_the_subscription_while_the_app_pays_with_a_key() {
        // One sign-in on the machine, and choosing it is a setting rather than
        // a second account, so a group can be moved onto it on its own.
        let resolved =
            only(InferenceOverrides { provider: Some(Provider::Chatgpt), ..Default::default() })
                .apply(&InferenceConfig::default());
        assert_eq!(resolved.provider, Provider::Chatgpt);
        assert_eq!(resolved.default_model, crate::llm::codex::DEFAULT_MODEL);
    }

    #[test]
    fn a_group_on_its_own_endpoint_can_still_name_its_own_model() {
        let resolved = only(InferenceOverrides {
            provider: Some(Provider::Compatible),
            base_url: Some("http://localhost:1234/v1".into()),
            default_model: Some("local/qwen".into()),
            ..Default::default()
        })
        .apply(&on_subscription());
        assert_eq!(resolved.provider, Provider::Compatible);
        assert_eq!(resolved.default_model, "local/qwen");
    }

    #[test]
    fn a_group_changes_nothing_when_the_app_is_on_an_endpoint() {
        // The behaviour that existed before groups had settings, unchanged.
        let base = InferenceConfig::default();
        assert_eq!(GroupInference::default().apply(&base), base);
    }
}
