use crate::domain::effort::ReasoningEffort;
use serde::{Deserialize, Serialize};

use super::ids::GroupId;
use crate::config::{InferenceConfig, Provider};
use crate::runtime::guard::GuardLimits;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Group {
    pub id: GroupId,
    pub name: String,
    pub inference: InferenceOverrides,
    pub api_key_set: bool,
    pub api_key_hint: String,
    pub limits: GroupLimits,
    pub tidy: Tidy,
    pub agent_count: u32,
    pub created_at: i64,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct InferenceOverrides {
    pub provider: Option<Provider>,
    pub base_url: Option<String>,
    pub default_model: Option<String>,
    pub subscription_model: Option<String>,
    pub reasoning_effort: Option<ReasoningEffort>,
    pub request_timeout_secs: Option<u64>,
}

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
    pub fn apply(&self, base: GuardLimits) -> GuardLimits {
        GuardLimits {
            max_hops: self.max_hops.unwrap_or(base.max_hops),
            max_steps_per_run: self.max_steps_per_run.unwrap_or(base.max_steps_per_run),
            max_fanout_per_call: self.max_fanout_per_call.unwrap_or(base.max_fanout_per_call),
            max_sends_per_pair: self.max_sends_per_pair.unwrap_or(base.max_sends_per_pair),
            max_tool_rounds: self.max_tool_rounds.unwrap_or(base.max_tool_rounds),
        }
    }

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

pub const DEFAULT_TIDY_HOUR: u8 = 3;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Tidy {
    pub on: bool,
    pub hour: u8,
}

impl Default for Tidy {
    fn default() -> Self {
        Tidy { on: false, hour: DEFAULT_TIDY_HOUR }
    }
}

impl Tidy {
    pub fn sanitized(self) -> Self {
        Tidy { hour: self.hour.min(23), ..self }
    }

    pub fn last_struck(hour: u8, now: i64) -> Option<i64> {
        let today = super::local(now)?.date_naive();
        let at = |day: chrono::NaiveDate| {
            super::instant(day.and_hms_opt(u32::from(hour.min(23)), 0, 0)?)
        };
        match at(today)? {
            struck if struck <= now => Some(struck),
            _ => at(today.pred_opt()?),
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GroupDraft {
    pub name: String,
    #[serde(default)]
    pub inference: Option<InferenceOverrides>,
    #[serde(default)]
    pub api_key: Option<String>,
    #[serde(default)]
    pub limits: Option<GroupLimits>,
    #[serde(default)]
    pub tidy: Option<Tidy>,
}

#[derive(Debug, thiserror::Error, PartialEq)]
pub enum GroupError {
    #[error("group name must not be blank")]
    BlankName,
    #[error("group name must be {max} characters or fewer")]
    NameTooLong { max: usize },
    #[error("a group named {name:?} already exists")]
    DuplicateName { name: String },
    #[error("that inference endpoint is not usable: {0}")]
    BadEndpoint(String),
}

pub const MAX_GROUP_NAME_LEN: usize = 48;

#[derive(Debug, Clone, Default, PartialEq)]
pub struct CleanGroup {
    pub name: String,
    pub inference: Option<InferenceOverrides>,
    pub api_key: Option<Option<String>>,
    pub limits: Option<GroupLimits>,
    pub tidy: Option<Tidy>,
}

fn override_of(value: &Option<String>) -> Option<String> {
    value.as_ref().map(|raw| raw.trim().to_string()).filter(|trimmed| !trimmed.is_empty())
}

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
                base_url: match override_of(&raw.base_url) {
                    Some(url) => Some(
                        crate::config::normalize_base_url(&url)
                            .map_err(|e| GroupError::BadEndpoint(e.to_string()))?,
                    ),
                    None => None,
                },
                default_model: override_of(&raw.default_model),
                subscription_model: override_of(&raw.subscription_model),
                reasoning_effort: raw.reasoning_effort,
                request_timeout_secs: raw.request_timeout_secs.map(clamp_timeout),
            }),
        };

        Ok(CleanGroup {
            name: name.to_string(),
            inference,
            api_key: self.api_key.as_ref().map(|raw| {
                let trimmed = raw.trim();
                if trimmed.is_empty() {
                    None
                } else {
                    Some(trimmed.to_string())
                }
            }),
            limits: self.limits.map(GroupLimits::sanitized),
            tidy: self.tidy.map(Tidy::sanitized),
        })
    }
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct GroupInference {
    pub overrides: InferenceOverrides,
    pub api_key: Option<String>,
}

impl GroupInference {
    pub fn endpoint(&self, base: &InferenceConfig) -> InferenceConfig {
        let mut out = base.clone();
        out.provider = Provider::Compatible;
        if let Some(url) = &self.overrides.base_url {
            out.base_url = url.clone();
        }
        if let Some(key) = &self.api_key {
            out.api_key = key.clone();
        }
        if let Some(model) = &self.overrides.default_model {
            out.default_model = model.clone();
        }
        out
    }

    pub fn apply(&self, base: &InferenceConfig) -> InferenceConfig {
        let mut out = self.endpoint(base);
        out.provider = base.provider;
        let over = &self.overrides;

        if let Some(provider) = over.provider {
            out.provider = provider;
        }

        if let Some(model) = &over.subscription_model {
            out.subscription_model = model.clone();
        }
        if let Some(effort) = over.reasoning_effort {
            out.reasoning_effort = effort;
        }
        if let Some(secs) = over.request_timeout_secs {
            out.request_timeout_secs = secs;
        }

        out.default_model = out.active_model().to_string();

        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn draft(name: &str) -> GroupDraft {
        GroupDraft { name: name.into(), inference: None, api_key: None, limits: None, tidy: None }
    }

    fn with_inference(name: &str, over: InferenceOverrides) -> GroupDraft {
        GroupDraft { inference: Some(over), ..draft(name) }
    }

    #[test]
    fn a_draft_is_read_from_the_shape_the_webview_sends() {
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
            tidy: Tidy::default(),
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
        assert!(draft(&"\u{1f951}".repeat(40)).validate().is_ok());
        assert_eq!(
            draft(&"\u{1f951}".repeat(49)).validate(),
            Err(GroupError::NameTooLong { max: MAX_GROUP_NAME_LEN })
        );
    }

    #[test]
    fn a_blanked_override_is_stored_as_inherit_not_as_empty() {
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

    fn on_subscription() -> InferenceConfig {
        InferenceConfig {
            provider: Provider::Chatgpt,
            default_model: "anthropic/claude-sonnet-4.5".into(),
            subscription_model: "gpt-5.6-luna".into(),
            api_key: "app-key".into(),
            ..Default::default()
        }
    }

    fn only(over: InferenceOverrides) -> GroupInference {
        GroupInference { overrides: over, api_key: None }
    }

    #[test]
    fn a_group_with_no_overrides_uses_the_subscription_and_its_own_model() {
        let resolved = GroupInference::default().apply(&on_subscription());
        assert_eq!(resolved.provider, Provider::Chatgpt);
        assert_eq!(resolved.default_model, "gpt-5.6-luna");
    }

    #[test]
    fn a_group_that_only_overrides_the_subscription_model_stays_on_the_subscription() {
        let resolved = only(InferenceOverrides {
            subscription_model: Some("gpt-5.4-mini".into()),
            ..Default::default()
        })
        .apply(&on_subscription());
        assert_eq!(resolved.provider, Provider::Chatgpt);
        assert_eq!(resolved.default_model, "gpt-5.4-mini");
    }

    #[test]
    fn a_group_can_run_on_claude_while_the_app_is_on_a_key() {
        let resolved =
            only(InferenceOverrides { provider: Some(Provider::Claude), ..Default::default() })
                .apply(&InferenceConfig::default());

        assert_eq!(resolved.provider, Provider::Claude);
        assert_eq!(resolved.default_model, crate::llm::claude::MODEL_LABEL);
    }

    #[test]
    fn a_model_named_on_a_claude_group_is_kept_and_never_used() {
        let resolved = only(InferenceOverrides {
            provider: Some(Provider::Claude),
            default_model: Some("local/qwen".into()),
            subscription_model: Some("gpt-5.4-mini".into()),
            ..Default::default()
        })
        .apply(&InferenceConfig::default());

        assert_eq!(resolved.default_model, crate::llm::claude::MODEL_LABEL);
        assert_eq!(resolved.subscription_model, "gpt-5.4-mini");
    }

    #[test]
    fn a_model_set_for_the_other_provider_is_kept_and_not_used() {
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
        assert_eq!(resolved.default_model, "anthropic/claude-sonnet-4.5");
    }

    #[test]
    fn an_endpoint_alone_does_not_decide_who_pays() {
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
        let base = InferenceConfig::default();
        assert_eq!(GroupInference::default().apply(&base), base);
    }

    #[test]
    fn a_group_on_the_subscription_still_lends_the_key_it_holds() {
        let crew = GroupInference {
            overrides: InferenceOverrides {
                provider: Some(Provider::Chatgpt),
                base_url: Some("https://openrouter.ai/api/v1".into()),
                default_model: Some("deepseek/deepseek-v4.1-flash".into()),
                subscription_model: Some("gpt-6-astra".into()),
                ..Default::default()
            },
            api_key: Some("crew-key".into()),
        };
        let app = InferenceConfig { api_key: String::new(), ..on_subscription() };

        let lent = crew.endpoint(&app);
        assert_eq!(lent.base_url, "https://openrouter.ai/api/v1");
        assert_eq!(lent.api_key, "crew-key");
        assert_eq!(lent.default_model, "deepseek/deepseek-v4.1-flash");
        assert_eq!(lent.active_model(), "deepseek/deepseek-v4.1-flash");
        assert_eq!(crew.apply(&app).provider, Provider::Chatgpt);
        assert_eq!(crew.apply(&app).default_model, "gpt-6-astra");
    }

    #[test]
    fn a_group_with_no_key_of_its_own_lends_the_apps() {
        let lent = GroupInference::default().endpoint(&on_subscription());
        assert_eq!(lent.api_key, "app-key");
        assert_eq!(lent.base_url, on_subscription().base_url);
        assert_eq!(lent.default_model, "anthropic/claude-sonnet-4.5");
    }

    #[test]
    fn the_tidy_is_owed_for_the_last_time_its_hour_struck() {
        use crate::domain::{instant, local};
        let hour = |day: chrono::NaiveDate, h: u32, m: u32| {
            instant(day.and_hms_opt(h, m, 0).unwrap()).unwrap()
        };
        let today = local(crate::domain::now_ms()).unwrap().date_naive();
        let yesterday = today.pred_opt().unwrap();
        assert_eq!(Tidy::last_struck(3, hour(today, 2, 59)), Some(hour(yesterday, 3, 0)));
        assert_eq!(Tidy::last_struck(3, hour(today, 3, 0)), Some(hour(today, 3, 0)));
        assert_eq!(Tidy::last_struck(3, hour(today, 23, 0)), Some(hour(today, 3, 0)));
        let later = today.checked_add_days(chrono::Days::new(3)).unwrap();
        assert_eq!(Tidy::last_struck(3, hour(later, 9, 0)), Some(hour(later, 3, 0)));
        assert_eq!(Tidy { on: true, hour: 40 }.sanitized(), Tidy { on: true, hour: 23 });
        assert_eq!(Tidy::default(), Tidy { on: false, hour: 3 });
    }

    #[test]
    fn a_draft_with_no_tidy_leaves_the_switch_alone() {
        let read: GroupDraft =
            serde_json::from_value(serde_json::json!({ "name": "Ops" })).unwrap();
        assert_eq!(read.tidy, None);
        let read: GroupDraft = serde_json::from_value(
            serde_json::json!({ "name": "Ops", "tidy": { "on": true, "hour": 4 } }),
        )
        .unwrap();
        assert_eq!(read.tidy, Some(Tidy { on: true, hour: 4 }));
    }
}
