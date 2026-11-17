use crate::domain::effort::ReasoningEffort;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::runtime::guard::GuardLimits;

pub const DEFAULT_BASE_URL: &str = "https://openrouter.ai/api/v1";
pub const DEFAULT_MODEL: &str = "anthropic/claude-sonnet-4.5";

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Provider {
    #[default]
    Compatible,
    Chatgpt,
    Claude,
}

impl Provider {
    pub const ALL: [Provider; 3] = [Provider::Compatible, Provider::Chatgpt, Provider::Claude];

    pub fn as_str(&self) -> &'static str {
        match self {
            Provider::Compatible => "compatible",
            Provider::Chatgpt => "chatgpt",
            Provider::Claude => "claude",
        }
    }

    pub fn parse(raw: &str) -> Option<Self> {
        match raw.trim() {
            "compatible" => Some(Provider::Compatible),
            "chatgpt" => Some(Provider::Chatgpt),
            "claude" => Some(Provider::Claude),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct InferenceConfig {
    #[serde(default)]
    pub provider: Provider,
    pub base_url: String,
    #[serde(default)]
    pub api_key: String,
    pub default_model: String,
    #[serde(default = "default_subscription_model")]
    pub subscription_model: String,
    #[serde(default)]
    pub reasoning_effort: ReasoningEffort,
    #[serde(default = "default_referer")]
    pub referer: String,
    #[serde(default = "default_title")]
    pub title: String,
    #[serde(default = "default_timeout")]
    pub request_timeout_secs: u64,
}

fn default_referer() -> String {
    "https://github.com/madebywelch/guac".to_string()
}

fn default_title() -> String {
    "Guac".to_string()
}

fn default_timeout() -> u64 {
    120
}

fn default_subscription_model() -> String {
    crate::llm::codex::DEFAULT_MODEL.to_string()
}

impl Default for InferenceConfig {
    fn default() -> Self {
        Self {
            provider: Provider::default(),
            base_url: DEFAULT_BASE_URL.to_string(),
            api_key: String::new(),
            default_model: DEFAULT_MODEL.to_string(),
            subscription_model: default_subscription_model(),
            reasoning_effort: ReasoningEffort::default(),
            referer: default_referer(),
            title: default_title(),
            request_timeout_secs: default_timeout(),
        }
    }
}

impl InferenceConfig {
    pub fn chat_completions_url(&self) -> String {
        format!("{}/chat/completions", self.base_url.trim_end_matches('/'))
    }

    pub fn is_ready(&self) -> bool {
        match self.provider {
            Provider::Chatgpt | Provider::Claude => true,
            Provider::Compatible => {
                !self.api_key.trim().is_empty() && !self.base_url.trim().is_empty()
            }
        }
    }

    pub fn endpoint(&self) -> &str {
        match self.provider {
            Provider::Chatgpt => crate::llm::codex::DEFAULT_BASE_URL,
            Provider::Claude => crate::llm::claude::PROGRAM,
            Provider::Compatible => &self.base_url,
        }
    }

    pub fn active_model(&self) -> &str {
        match self.provider {
            Provider::Chatgpt => &self.subscription_model,
            Provider::Claude => crate::llm::claude::MODEL_LABEL,
            Provider::Compatible => &self.default_model,
        }
    }
}

pub const CURRENT_VERSION: u32 = 1;

const V0_LIMITS: GuardLimits = GuardLimits {
    max_hops: 4,
    max_steps_per_run: 40,
    max_fanout_per_call: 8,
    max_sends_per_pair: 3,
    max_tool_rounds: 4,
};

#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct AppConfig {
    pub version: u32,
    #[serde(default)]
    pub operator_name: String,
    pub inference: InferenceConfig,
    pub limits: GuardLimits,
    #[serde(default)]
    pub e2b: E2bConfig,
    #[serde(default)]
    pub kernel: KernelConfig,
    #[serde(default)]
    pub webhook: WebhookConfig,
    #[serde(default)]
    pub quick_actions: Vec<crate::domain::quick::QuickAction>,
    #[serde(default)]
    pub widgets: Vec<crate::domain::widget::Widget>,
    #[serde(default)]
    pub attention: crate::domain::outbox::Attention,
    #[serde(default)]
    pub delivery: DeliveryConfig,
}

#[derive(Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct DeliveryConfig {
    pub url: String,
    pub token: String,
    pub format: crate::push::Format,
    pub content: crate::push::Content,
    pub link_base: String,
}

impl DeliveryConfig {
    pub fn is_on(&self) -> bool {
        !self.url.trim().is_empty()
    }
}

impl std::fmt::Debug for DeliveryConfig {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("DeliveryConfig")
            .field("url", &url_hint(&self.url))
            .field("token", &hint_for(&self.token))
            .field("format", &self.format)
            .field("content", &self.content)
            .field("link_base", &self.link_base)
            .finish()
    }
}

#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct WebhookConfig {
    pub secret: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct E2bConfig {
    pub api_key: String,
    #[serde(default = "default_idle_minutes")]
    pub idle_minutes: u32,
}

pub fn default_idle_minutes() -> u32 {
    15
}

impl Default for E2bConfig {
    fn default() -> Self {
        Self { api_key: String::new(), idle_minutes: default_idle_minutes() }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct KernelConfig {
    pub api_key: String,
    #[serde(default = "default_browser_idle_minutes")]
    pub idle_minutes: u32,
    #[serde(default)]
    pub stealth: bool,
}

pub fn default_browser_idle_minutes() -> u32 {
    60
}

impl Default for KernelConfig {
    fn default() -> Self {
        Self {
            api_key: String::new(),
            idle_minutes: default_browser_idle_minutes(),
            stealth: false,
        }
    }
}

pub fn migrate(config: &mut AppConfig) -> bool {
    if config.version >= CURRENT_VERSION {
        return false;
    }

    let current = GuardLimits::default();

    if config.limits.max_hops == V0_LIMITS.max_hops {
        config.limits.max_hops = current.max_hops;
    }
    if config.limits.max_steps_per_run == V0_LIMITS.max_steps_per_run {
        config.limits.max_steps_per_run = current.max_steps_per_run;
    }
    if config.limits.max_sends_per_pair == V0_LIMITS.max_sends_per_pair {
        config.limits.max_sends_per_pair = current.max_sends_per_pair;
    }
    if config.limits.max_tool_rounds == V0_LIMITS.max_tool_rounds {
        config.limits.max_tool_rounds = current.max_tool_rounds;
    }

    config.version = CURRENT_VERSION;
    true
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RedactedConfig {
    pub operator_name: String,
    pub e2b_key_set: bool,
    pub e2b_key_hint: String,
    pub computer_idle_minutes: u32,
    pub kernel_key_set: bool,
    pub kernel_key_hint: String,
    pub browser_idle_minutes: u32,
    pub browser_stealth: bool,
    pub provider: Provider,
    pub base_url: String,
    pub default_model: String,
    pub subscription_model: String,
    #[serde(default)]
    pub reasoning_effort: ReasoningEffort,
    pub api_key_set: bool,
    pub api_key_hint: String,
    pub request_timeout_secs: u64,
    pub limits: GuardLimits,
    pub subscription_models: Vec<String>,
    #[serde(default)]
    pub quick_actions: Vec<crate::domain::quick::QuickAction>,
    #[serde(default)]
    pub widgets: Vec<crate::domain::widget::Widget>,
    #[serde(default)]
    pub attention: crate::domain::outbox::Attention,
    #[serde(default)]
    pub delivery: RedactedDelivery,
}

#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct RedactedDelivery {
    pub url_set: bool,
    pub url_hint: String,
    pub token_set: bool,
    pub token_hint: String,
    pub format: crate::push::Format,
    pub content: crate::push::Content,
    pub link_base: String,
}

impl AppConfig {
    pub fn redacted(&self) -> RedactedConfig {
        RedactedConfig {
            operator_name: self.operator_name.clone(),
            e2b_key_set: !self.e2b.api_key.trim().is_empty(),
            e2b_key_hint: hint_for(&self.e2b.api_key),
            computer_idle_minutes: self.e2b.idle_minutes,
            kernel_key_set: !self.kernel.api_key.trim().is_empty(),
            kernel_key_hint: hint_for(&self.kernel.api_key),
            browser_idle_minutes: self.kernel.idle_minutes,
            browser_stealth: self.kernel.stealth,
            provider: self.inference.provider,
            base_url: self.inference.base_url.clone(),
            default_model: self.inference.default_model.clone(),
            subscription_model: self.inference.subscription_model.clone(),
            reasoning_effort: self.inference.reasoning_effort,
            api_key_set: !self.inference.api_key.trim().is_empty(),
            api_key_hint: hint_for(&self.inference.api_key),
            request_timeout_secs: self.inference.request_timeout_secs,
            limits: self.limits,
            subscription_models: vec![crate::llm::codex::DEFAULT_MODEL.to_string()],
            quick_actions: self.quick_actions.clone(),
            widgets: self.widgets.clone(),
            attention: self.attention,
            delivery: RedactedDelivery {
                url_set: self.delivery.is_on(),
                url_hint: url_hint(&self.delivery.url),
                token_set: !self.delivery.token.trim().is_empty(),
                token_hint: hint_for(&self.delivery.token),
                format: self.delivery.format,
                content: self.delivery.content,
                link_base: self.delivery.link_base.clone(),
            },
        }
    }
}

pub fn hint_for(key: &str) -> String {
    let key = key.trim();
    if key.is_empty() {
        return String::new();
    }
    let tail: String = key.chars().rev().take(4).collect::<Vec<_>>().into_iter().rev().collect();
    format!("...{tail}")
}

pub fn url_hint(url: &str) -> String {
    let url = url.trim();
    if url.is_empty() {
        return String::new();
    }
    let Ok(parsed) = reqwest::Url::parse(url) else {
        return hint_for(url);
    };
    let host = parsed.host_str().unwrap_or_default();
    let port = parsed.port().map(|port| format!(":{port}")).unwrap_or_default();
    let rest =
        format!("{}{}", parsed.path(), parsed.query().map(|q| format!("?{q}")).unwrap_or_default());
    let rest = rest.trim_matches('/');
    let shown = if rest.is_empty() { String::new() } else { hint_for(rest) };
    format!("{}://{host}{port}/{shown}", parsed.scheme())
}

fn web_address(input: &str, field: &'static str) -> Result<String, ConfigError> {
    let trimmed = input.trim();
    let parsed = reqwest::Url::parse(trimmed).map_err(|_| ConfigError::BadAddress { field })?;
    if !matches!(parsed.scheme(), "http" | "https") || parsed.host_str().is_none() {
        return Err(ConfigError::BadAddress { field });
    }
    Ok(trimmed.to_string())
}

#[derive(Debug, thiserror::Error)]
pub enum ConfigError {
    #[error("config i/o failed at {path}: {source}")]
    Io {
        path: PathBuf,
        #[source]
        source: io::Error,
    },
    #[error("config at {path} is not valid JSON: {source}")]
    Parse {
        path: PathBuf,
        #[source]
        source: serde_json::Error,
    },
    #[error("base URL must start with http:// or https://, got {got:?}")]
    BadBaseUrl { got: String },
    #[error("{field} must not be blank")]
    Blank { field: &'static str },
    #[error(transparent)]
    Quick(#[from] crate::domain::quick::QuickError),
    #[error(transparent)]
    Widget(#[from] crate::domain::widget::WidgetError),
    #[error("{field} must be an http:// or https:// address with a host")]
    BadAddress { field: &'static str },
}

pub fn normalize_base_url(input: &str) -> Result<String, ConfigError> {
    let trimmed = input.trim().trim_end_matches('/');
    if !(trimmed.starts_with("http://") || trimmed.starts_with("https://")) {
        return Err(ConfigError::BadBaseUrl { got: input.to_string() });
    }
    let cleaned = trimmed.trim_end_matches("/chat/completions").trim_end_matches('/');
    if cleaned.is_empty() {
        return Err(ConfigError::BadBaseUrl { got: input.to_string() });
    }
    Ok(cleaned.to_string())
}

#[derive(Debug, Default, Clone, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct LimitsPatch {
    pub max_hops: Option<u16>,
    pub max_steps_per_run: Option<u32>,
    pub max_fanout_per_call: Option<usize>,
    pub max_sends_per_pair: Option<u32>,
    pub max_tool_rounds: Option<u16>,
}

#[derive(Debug, Default, Clone, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct SettingsPatch {
    pub operator_name: Option<String>,
    pub provider: Option<Provider>,
    pub base_url: Option<String>,
    pub api_key: Option<String>,
    pub default_model: Option<String>,
    pub subscription_model: Option<String>,
    pub reasoning_effort: Option<crate::domain::effort::ReasoningEffort>,
    pub request_timeout_secs: Option<u64>,
    pub limits: Option<LimitsPatch>,
    pub e2b_api_key: Option<String>,
    pub computer_idle_minutes: Option<u32>,
    pub kernel_api_key: Option<String>,
    pub browser_idle_minutes: Option<u32>,
    pub browser_stealth: Option<bool>,
    pub attention: Option<AttentionPatch>,
    pub delivery: Option<DeliveryPatch>,
}

#[derive(Debug, Default, Clone, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct DeliveryPatch {
    pub url: Option<String>,
    pub token: Option<String>,
    pub format: Option<crate::push::Format>,
    pub content: Option<crate::push::Content>,
    pub link_base: Option<String>,
}

#[derive(Debug, Default, Clone, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct AttentionPatch {
    pub needs_you: Option<bool>,
    pub failures: Option<bool>,
    pub finished: Option<bool>,
}

impl AppConfig {
    pub fn apply(&mut self, patch: SettingsPatch) -> Result<(), ConfigError> {
        if let Some(name) = patch.operator_name {
            self.operator_name = name.trim().to_string();
        }
        if let Some(provider) = patch.provider {
            self.inference.provider = provider;
        }
        if let Some(base_url) = patch.base_url {
            self.inference.base_url = normalize_base_url(&base_url)?;
        }
        if let Some(key) = patch.e2b_api_key {
            self.e2b.api_key = key.trim().to_string();
        }
        if let Some(minutes) = patch.computer_idle_minutes {
            self.e2b.idle_minutes = minutes.clamp(1, 24 * 60);
        }
        if let Some(key) = patch.kernel_api_key {
            self.kernel.api_key = key.trim().to_string();
        }
        if let Some(minutes) = patch.browser_idle_minutes {
            self.kernel.idle_minutes = minutes.clamp(1, 72 * 60);
        }
        if let Some(stealth) = patch.browser_stealth {
            self.kernel.stealth = stealth;
        }
        if let Some(attention) = patch.attention {
            let now = &mut self.attention;
            now.needs_you = attention.needs_you.unwrap_or(now.needs_you);
            now.failures = attention.failures.unwrap_or(now.failures);
            now.finished = attention.finished.unwrap_or(now.finished);
        }
        if let Some(delivery) = patch.delivery {
            let now = &mut self.delivery;
            if let Some(url) = delivery.url {
                now.url = match url.trim() {
                    "" => String::new(),
                    url => web_address(url, "the address to send to")?,
                };
            }
            if let Some(token) = delivery.token {
                now.token = token.trim().to_string();
            }
            now.format = delivery.format.unwrap_or(now.format);
            now.content = delivery.content.unwrap_or(now.content);
            if let Some(link) = delivery.link_base {
                now.link_base = match link.trim() {
                    "" => String::new(),
                    link => web_address(link, "the address a push opens")?,
                };
            }
        }
        if let Some(api_key) = patch.api_key {
            self.inference.api_key = api_key.trim().to_string();
        }
        if let Some(model) = patch.default_model {
            let trimmed = model.trim();
            if trimmed.is_empty() {
                return Err(ConfigError::Blank { field: "default model" });
            }
            self.inference.default_model = trimmed.to_string();
        }
        if let Some(effort) = patch.reasoning_effort {
            self.inference.reasoning_effort = effort;
        }
        if let Some(model) = patch.subscription_model {
            let trimmed = model.trim();
            if trimmed.is_empty() {
                return Err(ConfigError::Blank { field: "subscription model" });
            }
            self.inference.subscription_model = trimmed.to_string();
        }
        if let Some(timeout) = patch.request_timeout_secs {
            self.inference.request_timeout_secs = timeout.clamp(5, 900);
        }
        if let Some(limits) = patch.limits {
            self.limits = GuardLimits {
                max_hops: limits.max_hops.unwrap_or(self.limits.max_hops),
                max_steps_per_run: limits
                    .max_steps_per_run
                    .unwrap_or(self.limits.max_steps_per_run),
                max_fanout_per_call: limits
                    .max_fanout_per_call
                    .unwrap_or(self.limits.max_fanout_per_call),
                max_sends_per_pair: limits
                    .max_sends_per_pair
                    .unwrap_or(self.limits.max_sends_per_pair),
                max_tool_rounds: limits.max_tool_rounds.unwrap_or(self.limits.max_tool_rounds),
            }
            .sanitized();
        }
        Ok(())
    }
}

pub const AGENT_WRITABLE: &[&str] = &[
    "operatorName",
    "defaultModel",
    "subscriptionModel",
    "reasoningEffort",
    "requestTimeoutSecs",
    "limits",
    "computerIdleMinutes",
    "browserIdleMinutes",
    "browserStealth",
];

pub const OPERATOR_ONLY: &[&str] =
    &["provider", "baseUrl", "apiKey", "e2bApiKey", "kernelApiKey", "permissionRules"];

pub const LIMIT_LABELS: &[(&str, &str)] = &[
    ("maxStepsPerRun", "Model calls per conversation"),
    ("maxToolRounds", "Tool calls per turn"),
    ("maxHops", "Relay depth"),
    ("maxSendsPerPair", "Messages between any two agents"),
    ("maxFanoutPerCall", "Recipients per send"),
];

#[allow(dead_code)]
#[derive(Default, Deserialize)]
#[serde(rename_all = "camelCase", default)]
struct AgentPatchValidation {
    operator_name: Option<String>,
    default_model: Option<String>,
    subscription_model: Option<String>,
    reasoning_effort: Option<ReasoningEffort>,
    request_timeout_secs: Option<u64>,
    limits: Option<LimitsValidation>,
    computer_idle_minutes: Option<u32>,
    browser_idle_minutes: Option<u32>,
    browser_stealth: Option<bool>,
}

#[allow(dead_code)]
#[derive(Default, Deserialize)]
#[serde(rename_all = "camelCase", default)]
struct LimitsValidation {
    max_hops: u16,
    max_steps_per_run: u32,
    max_fanout_per_call: usize,
    max_sends_per_pair: u32,
    max_tool_rounds: u16,
}

pub fn agent_patch(changes: &serde_json::Value) -> Result<SettingsPatch, String> {
    let Some(asked) = changes.as_object().filter(|asked| !asked.is_empty()) else {
        return Err("`changes` must name the settings to change, for example \
                    {\"limits\": {\"maxHops\": 12}}. `read` shows their names."
            .into());
    };
    let mut value = serde_json::Map::new();
    for (key, wanted) in asked {
        if OPERATOR_ONLY.contains(&key.as_str()) {
            return Err(format!(
                "`{key}` is the operator's to change in Settings, and no agent can change it, \
                 even with approval. Tell them what you would change and why."
            ));
        }
        if !AGENT_WRITABLE.contains(&key.as_str()) {
            return Err(format!(
                "there is no setting called `{key}` that an agent can change. The ones you can: {}.",
                AGENT_WRITABLE.join(", ")
            ));
        }
        if key == "limits" {
            let Some(limits) = wanted.as_object() else {
                return Err("`limits` must be an object, for example {\"maxHops\": 12}.".into());
            };
            if let Some(limit) =
                limits.keys().find(|limit| !LIMIT_LABELS.iter().any(|(name, _)| name == limit))
            {
                return Err(format!(
                    "there is no limit called `{limit}`. The five are: {}.",
                    LIMIT_LABELS.iter().map(|(name, _)| *name).collect::<Vec<_>>().join(", ")
                ));
            }
        }
        value.insert(key.clone(), wanted.clone());
    }
    let value = serde_json::Value::Object(value);
    serde_json::from_value::<AgentPatchValidation>(value.clone()).map_err(|err| {
        format!("those values could not be read ({err}). `read` shows each setting's type.")
    })?;
    serde_json::from_value(value).map_err(|err| {
        format!("those values could not be read ({err}). `read` shows each setting's type.")
    })
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Change {
    pub label: String,
    pub from: String,
    pub to: String,
}

pub fn changes(before: &AppConfig, after: &AppConfig) -> Vec<Change> {
    let mut out = Vec::new();
    let mut compare = |label: &str, from: String, to: String| {
        if from != to {
            out.push(Change { label: label.to_string(), from, to });
        }
    };
    compare("Your name", before.operator_name.clone(), after.operator_name.clone());
    compare(
        "Default model",
        before.inference.default_model.clone(),
        after.inference.default_model.clone(),
    );
    compare(
        "ChatGPT model",
        before.inference.subscription_model.clone(),
        after.inference.subscription_model.clone(),
    );
    compare(
        "Reasoning effort",
        before.inference.reasoning_effort.as_str().to_string(),
        after.inference.reasoning_effort.as_str().to_string(),
    );
    compare(
        "Give up on a model call after",
        format!("{}s", before.inference.request_timeout_secs),
        format!("{}s", after.inference.request_timeout_secs),
    );
    let (was, now) = (
        serde_json::to_value(before.limits).unwrap_or_default(),
        serde_json::to_value(after.limits).unwrap_or_default(),
    );
    for (name, label) in LIMIT_LABELS {
        compare(label, was[name].to_string(), now[name].to_string());
    }
    compare(
        "Sleep computers after",
        format!("{} min", before.e2b.idle_minutes),
        format!("{} min", after.e2b.idle_minutes),
    );
    compare(
        "Close browsers after",
        format!("{} min", before.kernel.idle_minutes),
        format!("{} min", after.kernel.idle_minutes),
    );
    compare(
        "Hide that browsers are automated",
        if before.kernel.stealth { "on" } else { "off" }.to_string(),
        if after.kernel.stealth { "on" } else { "off" }.to_string(),
    );
    out
}

pub fn without_userinfo(url: &str) -> String {
    match url.split_once("://") {
        Some((scheme, rest)) => {
            let (authority, path) = rest.split_once('/').map_or((rest, ""), |(a, p)| (a, p));
            let host = authority.rsplit_once('@').map_or(authority, |(_, host)| host);
            if path.is_empty() && !rest.contains('/') {
                format!("{scheme}://{host}")
            } else {
                format!("{scheme}://{host}/{path}")
            }
        }
        None => url.to_string(),
    }
}

pub fn load(path: &Path) -> Result<AppConfig, ConfigError> {
    let mut config = match fs::read_to_string(path) {
        Ok(raw) => serde_json::from_str::<AppConfig>(&raw)
            .map_err(|source| ConfigError::Parse { path: path.to_path_buf(), source })?,
        Err(e) if e.kind() == io::ErrorKind::NotFound => {
            return Ok(AppConfig { version: CURRENT_VERSION, ..Default::default() });
        }
        Err(source) => return Err(ConfigError::Io { path: path.to_path_buf(), source }),
    };

    if migrate(&mut config) {
        if let Err(err) = save(path, &config) {
            tracing::warn!(%err, "could not persist migrated settings");
        }
    }
    Ok(config)
}

pub fn save(path: &Path, config: &AppConfig) -> Result<(), ConfigError> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)
            .map_err(|source| ConfigError::Io { path: parent.to_path_buf(), source })?;
    }

    let json = serde_json::to_string_pretty(config)
        .map_err(|source| ConfigError::Parse { path: path.to_path_buf(), source })?;

    let tmp = path.with_extension("json.tmp");
    fs::write(&tmp, json).map_err(|source| ConfigError::Io { path: tmp.clone(), source })?;
    restrict_permissions(&tmp)?;
    fs::rename(&tmp, path)
        .map_err(|source| ConfigError::Io { path: path.to_path_buf(), source })?;
    Ok(())
}

#[cfg(unix)]
fn restrict_permissions(path: &Path) -> Result<(), ConfigError> {
    use std::os::unix::fs::PermissionsExt;
    fs::set_permissions(path, fs::Permissions::from_mode(0o600))
        .map_err(|source| ConfigError::Io { path: path.to_path_buf(), source })
}

#[cfg(not(unix))]
fn restrict_permissions(_path: &Path) -> Result<(), ConfigError> {
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_agent_may_ask_for_a_limit_and_never_for_the_endpoint_or_a_key() {
        let current = AppConfig::default();
        for key in OPERATOR_ONLY {
            let asked = serde_json::json!({ *key: "https://attacker.example" });
            let refused = agent_patch(&asked).unwrap_err();
            assert!(refused.contains("the operator's"), "{key}: {refused}");
        }
        let refused = agent_patch(&serde_json::json!({ "webhook": {} })).unwrap_err();
        assert!(refused.contains("operatorName"), "lists what it may change: {refused}");
        assert!(agent_patch(&serde_json::json!({})).is_err());

        let patch = agent_patch(&serde_json::json!({ "limits": { "maxHops": 12 } })).unwrap();
        let limits = patch.clone().limits.unwrap();
        assert_eq!(limits.max_hops, Some(12));
        assert_eq!(limits.max_steps_per_run, None);
        assert_eq!(limits.max_fanout_per_call, None);
        assert_eq!(limits.max_sends_per_pair, None);
        assert_eq!(limits.max_tool_rounds, None);
        let mut applied = current.clone();
        applied.apply(patch).unwrap();
        assert_eq!(applied.limits, GuardLimits { max_hops: 12, ..current.limits });
        let refused = agent_patch(&serde_json::json!({ "limits": { "maxEverything": 1 } }));
        assert!(refused.unwrap_err().contains("maxHops"));
    }

    #[test]
    fn what_interrupts_the_operator_is_theirs_alone_to_set() {
        for asked in [
            serde_json::json!({ "attention": { "needsYou": false } }),
            serde_json::json!({ "attention": { "failures": false, "finished": false } }),
            serde_json::json!({ "limits": { "maxHops": 12 }, "attention": { "needsYou": false } }),
        ] {
            let refused = agent_patch(&asked).unwrap_err();
            assert!(refused.contains("attention"), "{refused}");
        }
        assert!(!AGENT_WRITABLE.contains(&"attention"));

        let mut config = AppConfig::default();
        let patch: SettingsPatch =
            serde_json::from_value(serde_json::json!({ "attention": { "finished": false } }))
                .unwrap();
        config.apply(patch).unwrap();
        assert!(!config.attention.finished);
        assert!(config.attention.needs_you && config.attention.failures);
        assert_eq!(config.redacted().attention, config.attention, "every window reads it back");
    }

    #[test]
    fn a_change_reads_as_the_pane_labels_it_before_and_after() {
        let before = AppConfig::default();
        let mut after = before.clone();
        after
            .apply(agent_patch(&serde_json::json!({ "limits": { "maxHops": 12 } })).unwrap())
            .unwrap();
        let listed = changes(&before, &after);
        assert_eq!(
            listed,
            vec![Change {
                label: "Relay depth".into(),
                from: before.limits.max_hops.to_string(),
                to: "12".into(),
            }]
        );
        assert!(changes(&before, &before).is_empty());
    }

    #[test]
    fn an_address_shown_to_a_model_carries_no_credentials() {
        assert_eq!(without_userinfo("https://u:p@host.example/v1"), "https://host.example/v1");
        assert_eq!(without_userinfo("https://host.example/v1"), "https://host.example/v1");
        assert_eq!(without_userinfo("http://u@localhost:1234"), "http://localhost:1234");
    }

    #[test]
    fn a_provider_is_spelled_the_same_way_in_sqlite_and_on_the_wire() {
        for provider in Provider::ALL {
            let wire = serde_json::to_value(provider).unwrap();
            assert_eq!(wire, serde_json::Value::String(provider.as_str().to_string()));
            assert_eq!(Provider::parse(provider.as_str()), Some(provider));
        }
        assert_eq!(Provider::parse("anthropic-native"), None, "a future value is inherit");
    }

    #[test]
    fn chat_url_is_built_without_a_double_slash() {
        let mut cfg = InferenceConfig::default();
        assert_eq!(cfg.chat_completions_url(), "https://openrouter.ai/api/v1/chat/completions");
        cfg.base_url = "https://openrouter.ai/api/v1/".into();
        assert_eq!(cfg.chat_completions_url(), "https://openrouter.ai/api/v1/chat/completions");
    }

    #[test]
    fn base_url_requires_a_scheme() {
        assert!(matches!(
            normalize_base_url("openrouter.ai/api/v1"),
            Err(ConfigError::BadBaseUrl { .. })
        ));
        assert!(normalize_base_url("http://localhost:1234/v1").is_ok());
    }

    #[test]
    fn base_url_tolerates_a_pasted_endpoint() {
        assert_eq!(
            normalize_base_url("https://openrouter.ai/api/v1/chat/completions").unwrap(),
            "https://openrouter.ai/api/v1"
        );
    }

    #[test]
    fn base_url_strips_trailing_slashes() {
        assert_eq!(
            normalize_base_url("  https://example.com/v1///  ").unwrap(),
            "https://example.com/v1"
        );
    }

    #[test]
    fn redaction_never_exposes_the_key() {
        let cfg = AppConfig {
            inference: InferenceConfig {
                api_key: "sk-or-v1-supersecretvalue9999".into(),
                ..Default::default()
            },
            ..Default::default()
        };
        let json = serde_json::to_string(&cfg.redacted()).unwrap();
        assert!(!json.contains("supersecret"), "redacted config leaked the key: {json}");
        assert!(json.contains("...9999"), "expected a last-four hint");
        assert!(cfg.redacted().api_key_set);
    }

    #[test]
    fn redaction_of_an_empty_key_reports_not_set() {
        let cfg = AppConfig::default();
        let r = cfg.redacted();
        assert!(!r.api_key_set);
        assert_eq!(r.api_key_hint, "");
    }

    #[test]
    fn the_push_address_and_token_reach_the_webview_only_as_hints() {
        let mut config = AppConfig::default();
        let patch: SettingsPatch = serde_json::from_value(serde_json::json!({
            "delivery": {
                "url": "https://ntfy.sh/guaca-secret-topic-7f3a",
                "token": "tk_supersecrettoken1234",
                "format": "json",
                "content": "generic",
                "linkBase": "https://box.example",
            }
        }))
        .unwrap();
        config.apply(patch).unwrap();
        assert_eq!(config.delivery.url, "https://ntfy.sh/guaca-secret-topic-7f3a");
        assert_eq!(config.delivery.format, crate::push::Format::Json);

        let redacted = config.redacted();
        let json = serde_json::to_string(&redacted).unwrap();
        assert!(!json.contains("secret-topic"), "{json}");
        assert!(!json.contains("supersecret"), "{json}");
        assert_eq!(redacted.delivery.url_hint, "https://ntfy.sh/...7f3a");
        assert_eq!(redacted.delivery.token_hint, "...1234");
        assert!(redacted.delivery.url_set && redacted.delivery.token_set);
        assert_eq!(redacted.delivery.link_base, "https://box.example");

        let printed = format!("{config:?}");
        assert!(!printed.contains("secret-topic") && !printed.contains("supersecret"), "{printed}");
    }

    #[test]
    fn a_push_address_is_kept_until_it_is_replaced_or_blanked() {
        let mut config = AppConfig::default();
        let patch = |value: serde_json::Value| -> SettingsPatch {
            serde_json::from_value(serde_json::json!({ "delivery": value })).unwrap()
        };
        config
            .apply(patch(serde_json::json!({ "url": " https://ntfy.example/t ", "token": "tk_a" })))
            .unwrap();
        config.apply(patch(serde_json::json!({ "content": "generic" }))).unwrap();
        assert_eq!(
            config.delivery.url, "https://ntfy.example/t",
            "a patch that names it not keeps it"
        );
        assert_eq!(config.delivery.token, "tk_a");

        let refused = config.apply(patch(serde_json::json!({ "url": "ntfy.sh/my-private-topic" })));
        let said = refused.unwrap_err().to_string();
        assert!(!said.contains("my-private-topic"), "a refusal does not repeat the topic: {said}");
        assert!(said.contains("http"), "{said}");
        assert!(config.apply(patch(serde_json::json!({ "linkBase": "ftp://box" }))).is_err());
        assert_eq!(
            config.delivery.url, "https://ntfy.example/t",
            "a refused patch changes nothing"
        );

        config.apply(patch(serde_json::json!({ "url": "", "token": "" }))).unwrap();
        assert!(!config.delivery.is_on());
        assert!(config.delivery.token.is_empty());
        assert_eq!(config.redacted().delivery.url_hint, "");
    }

    #[test]
    fn no_agent_can_ask_for_the_push_address() {
        let asked = serde_json::json!({ "delivery": { "url": "https://attacker.example/topic" } });
        let refused = agent_patch(&asked).unwrap_err();
        assert!(refused.contains("delivery"), "{refused}");
        assert!(!AGENT_WRITABLE.contains(&"delivery"));
    }

    #[test]
    fn an_address_hint_names_the_server_and_never_the_path() {
        assert_eq!(url_hint(""), "");
        assert_eq!(url_hint("https://ntfy.sh/abcdefgh"), "https://ntfy.sh/...efgh");
        assert_eq!(
            url_hint("http://user:pw@10.0.0.2:8080/hook?k=v"),
            "http://10.0.0.2:8080/...?k=v"
        );
        assert_eq!(url_hint("https://ntfy.example/"), "https://ntfy.example/");
        assert_eq!(url_hint("not-a-url"), "...-url");
    }

    #[test]
    fn hint_handles_short_and_multibyte_keys_without_panicking() {
        assert_eq!(hint_for("ab"), "...ab");
        assert_eq!(
            hint_for("\u{1f951}\u{1f951}\u{1f951}\u{1f951}\u{1f951}"),
            "...\u{1f951}\u{1f951}\u{1f951}\u{1f951}"
        );
    }

    #[test]
    fn missing_config_file_is_first_run_not_failure() {
        let dir = tempfile::tempdir().unwrap();
        let cfg = load(&dir.path().join("absent.json")).unwrap();
        assert_eq!(cfg.limits, GuardLimits::default());
        assert_eq!(cfg.version, CURRENT_VERSION, "a fresh config is already current");
    }

    #[test]
    fn a_superseded_default_is_replaced_on_load() {
        let mut config = AppConfig { version: 0, limits: V0_LIMITS, ..Default::default() };
        assert!(migrate(&mut config));

        let current = GuardLimits::default();
        assert_eq!(config.limits.max_steps_per_run, current.max_steps_per_run);
        assert_eq!(config.limits.max_sends_per_pair, current.max_sends_per_pair);
        assert_eq!(config.limits.max_hops, current.max_hops);
        assert_eq!(config.version, CURRENT_VERSION);
    }

    #[test]
    fn a_deliberately_chosen_limit_is_left_alone() {
        let mut config = AppConfig {
            version: 0,
            limits: GuardLimits { max_hops: 16, ..V0_LIMITS },
            ..Default::default()
        };
        migrate(&mut config);

        assert_eq!(config.limits.max_hops, 16, "a chosen value must survive");
        assert_eq!(config.limits.max_steps_per_run, GuardLimits::default().max_steps_per_run);
    }

    #[test]
    fn migration_runs_once() {
        let mut config = AppConfig { version: 0, limits: V0_LIMITS, ..Default::default() };
        assert!(migrate(&mut config));
        assert!(!migrate(&mut config), "an up-to-date config is left alone");
    }

    #[test]
    fn loading_an_old_config_rewrites_it_in_place() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.json");
        let old = AppConfig { version: 0, limits: V0_LIMITS, ..Default::default() };
        save(&path, &old).unwrap();

        let loaded = load(&path).unwrap();
        assert_eq!(loaded.limits, GuardLimits::default());

        let reread: AppConfig = serde_json::from_str(&fs::read_to_string(&path).unwrap()).unwrap();
        assert_eq!(reread.version, CURRENT_VERSION);
    }

    #[test]
    fn config_round_trips_through_disk() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("nested").join("config.json");
        let mut cfg = AppConfig { version: CURRENT_VERSION, ..Default::default() };
        cfg.inference.api_key = "sk-test".into();
        cfg.limits.max_hops = 7;

        save(&path, &cfg).unwrap();
        assert_eq!(load(&path).unwrap(), cfg, "a current config round trips untouched");
    }

    #[test]
    fn a_webhook_port_from_an_older_build_loads_and_is_dropped_on_save() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.json");
        let mut cfg = AppConfig { version: CURRENT_VERSION, ..Default::default() };
        cfg.operator_name = "Ada".into();
        cfg.inference.api_key = "sk-kept".into();
        cfg.limits.max_hops = 7;
        cfg.webhook.secret = "kept-secret".into();
        let mut older = serde_json::to_value(&cfg).unwrap();
        older["webhook"]["port"] = serde_json::json!(4711);
        fs::write(&path, older.to_string()).unwrap();

        let loaded = load(&path).unwrap();
        assert_eq!(loaded, cfg, "everything but the port is read as it was written");

        save(&path, &loaded).unwrap();
        let written: serde_json::Value =
            serde_json::from_str(&fs::read_to_string(&path).unwrap()).unwrap();
        assert_eq!(written["webhook"], serde_json::json!({ "secret": "kept-secret" }));
    }

    #[cfg(unix)]
    #[test]
    fn saved_config_is_not_world_readable() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.json");
        save(&path, &AppConfig::default()).unwrap();
        let mode = fs::metadata(&path).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode, 0o600, "config holds an API key and must stay owner-only");
    }

    #[test]
    fn save_leaves_no_temp_file_behind() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.json");
        save(&path, &AppConfig::default()).unwrap();
        assert!(!path.with_extension("json.tmp").exists());
    }

    #[test]
    fn corrupt_config_reports_the_path_rather_than_silently_resetting() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.json");
        fs::write(&path, "{ not json").unwrap();
        assert!(matches!(load(&path), Err(ConfigError::Parse { .. })));
    }

    #[test]
    fn unknown_fields_and_missing_fields_both_survive_a_version_skew() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.json");
        fs::write(
            &path,
            r#"{"inference":{"baseUrl":"https://x/v1","defaultModel":"m","futureField":1}}"#,
        )
        .unwrap();
        let cfg = load(&path).unwrap();
        assert_eq!(cfg.inference.base_url, "https://x/v1");
        assert_eq!(cfg.inference.request_timeout_secs, default_timeout());
        assert_eq!(cfg.limits, GuardLimits::default());
    }
}
