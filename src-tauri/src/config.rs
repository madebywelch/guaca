//! Operator configuration.
//!
//! Stored as JSON next to the database in the platform config directory. The
//! API key lives here in plaintext, which is worth being blunt about: Guac is
//! a local, no-auth app, and a key encrypted with a key stored beside it is
//! theater. The file is written 0600 and the key is never sent to the webview.
//! If you want real secret storage, the honest answer is the OS keychain, and
//! that is a deliberate follow-up rather than something faked here.

use crate::domain::effort::ReasoningEffort;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::runtime::guard::GuardLimits;

pub const DEFAULT_BASE_URL: &str = "https://openrouter.ai/api/v1";
pub const DEFAULT_MODEL: &str = "anthropic/claude-sonnet-4.5";

/// How a turn is paid for.
///
/// Three answers, not one with a flag: a pasted key, a signed-in subscription
/// and a program on this machine differ in the endpoint, the wire protocol, the
/// auth header, the models on offer and whether a call has a price. Modeling
/// any of them as "a base URL with a different key" would put that whole
/// disagreement behind a string an operator can type, and the first symptom
/// would be an agent failing on a parameter nobody set.
///
/// The third is not an endpoint at all. `Claude` runs the `claude` program once
/// per model call and reads its stdout, which is the only way an Anthropic
/// subscription can pay for a turn: consumer OAuth tokens are restricted to the
/// program they were issued to, so the way to spend one is to be that program.
/// `docs/PROTOCOL.md` has the dates and the sources.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Provider {
    /// Any OpenAI-compatible endpoint, with a key the operator pasted. What
    /// Guaca has always done, and still the default.
    #[default]
    Compatible,
    /// A ChatGPT subscription, signed in to on this machine. Billed to the plan
    /// rather than per token.
    Chatgpt,
    /// The `claude` program on this machine, run once per model call. Billed to
    /// whatever that program is signed in to, which is the operator's business
    /// and not this app's.
    Claude,
}

impl Provider {
    /// Every provider there is.
    ///
    /// A fixed-length array rather than a loose list, so a variant added
    /// without being added here is a compile error rather than a case the
    /// round-trip suite below silently stops covering.
    pub const ALL: [Provider; 3] = [Provider::Compatible, Provider::Chatgpt, Provider::Claude];

    /// How a provider is spelled in SQLite, which is the same as on the wire.
    ///
    /// Not `Display`: the point is that the stored spelling and the IPC one
    /// cannot drift, and a formatting impl is where a friendlier name would
    /// eventually be written.
    pub fn as_str(&self) -> &'static str {
        match self {
            Provider::Compatible => "compatible",
            Provider::Chatgpt => "chatgpt",
            Provider::Claude => "claude",
        }
    }

    /// Reads one back. Anything unrecognized is `None`, which every caller
    /// already has a meaning for: inherit. A column written by a newer build
    /// must leave a crew running on the app settings rather than refusing to
    /// load the group.
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
    /// Which of the two places a call goes to. Absent in anything written before
    /// subscriptions existed, and absent means the endpoint below.
    #[serde(default)]
    pub provider: Provider,
    /// Any OpenAI-compatible base. Swappable so a local llama.cpp or LM Studio
    /// endpoint works without a code change.
    ///
    /// Ignored when the provider is a subscription, and kept rather than blanked:
    /// an operator who tries a subscription and goes back should find their
    /// endpoint and key where they left them.
    pub base_url: String,
    #[serde(default)]
    pub api_key: String,
    /// The model used when a pasted key is paying.
    pub default_model: String,
    /// The model used when a subscription is paying.
    ///
    /// A second field rather than one shared with the endpoint's, because the
    /// two providers have disjoint model names and neither will accept the
    /// other's. Sharing one field meant every switch broke the model, and
    /// switching back did not put it right: an operator who ran out of
    /// subscription quota, moved to a key for an hour and moved back would find
    /// their model replaced both times.
    #[serde(default = "default_subscription_model")]
    pub subscription_model: String,
    #[serde(default)]
    pub reasoning_effort: ReasoningEffort,
    /// OpenRouter attributes requests by these headers. Harmless elsewhere.
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
    /// Full URL for the chat completions endpoint.
    pub fn chat_completions_url(&self) -> String {
        format!("{}/chat/completions", self.base_url.trim_end_matches('/'))
    }

    /// Whether a call has any chance of working, as far as settings can tell.
    ///
    /// Neither a subscription nor the `claude` program is answered here. Whether
    /// one is signed in is held by the credential store or by the program's own
    /// config, not by settings, and a config that guessed would report a sign-in
    /// it cannot see. Each transport asks the thing that actually knows and
    /// produces a refusal that names the actual problem.
    pub fn is_ready(&self) -> bool {
        match self.provider {
            Provider::Chatgpt | Provider::Claude => true,
            Provider::Compatible => {
                !self.api_key.trim().is_empty() && !self.base_url.trim().is_empty()
            }
        }
    }

    /// Where a call actually goes, for the one line that reports it.
    pub fn endpoint(&self) -> &str {
        match self.provider {
            Provider::Chatgpt => crate::llm::codex::DEFAULT_BASE_URL,
            Provider::Claude => crate::llm::claude::PROGRAM,
            Provider::Compatible => &self.base_url,
        }
    }

    /// The model the active provider is set to.
    ///
    /// Every reader that is about to make a call wants this rather than either
    /// field: an agent or a group can still override it afterward, but the
    /// value being overridden has to be the one that belongs to the provider
    /// doing the work.
    ///
    /// `Claude` has no field to read and does not gain one. The model belongs to
    /// the program, which is signed in, configured and updated somewhere this
    /// app does not reach. What comes back is a label rather than a model id, and the model
    /// the program actually ran is reported in its own stream.
    pub fn active_model(&self) -> &str {
        match self.provider {
            Provider::Chatgpt => &self.subscription_model,
            Provider::Claude => crate::llm::claude::MODEL_LABEL,
            Provider::Compatible => &self.default_model,
        }
    }
}

/// Bumped whenever stored settings need adjusting on load.
pub const CURRENT_VERSION: u32 = 1;

/// The limits Guac shipped with before they were retuned against real use.
///
/// Kept so a stored value can be told apart from a deliberate one. A number
/// that exactly matches an old default was almost certainly never chosen by
/// anyone; a different number was.
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
    /// 0 means "written before settings were versioned".
    pub version: u32,
    /// What agents call the person using this app.
    ///
    /// Ambient rather than something each agent discovers and writes into its
    /// own memory: an operator had to tell one agent their name, that agent
    /// stored it privately, and every other agent still had no idea who it was
    /// working for. Empty falls back to "the operator", which is what agents
    /// said before this existed.
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
    /// The status bar's buttons, in the order they are drawn. A setting like
    /// the rest, so every window draws the same bar and a change reaches them
    /// through the same event.
    #[serde(default)]
    pub quick_actions: Vec<crate::domain::quick::QuickAction>,
    /// Kept pages pinned to the status bar, drawn from their condensed views,
    /// in the order they are drawn. A setting for the quick actions' reason.
    #[serde(default)]
    pub widgets: Vec<crate::domain::widget::Widget>,
    /// What may interrupt the operator. On the Host rather than in each
    /// browser, because the Host now decides it, once, for every window.
    /// Only the operator sets it: it is not in [`AGENT_WRITABLE`], and an
    /// agent's own settings read does not show it.
    #[serde(default)]
    pub attention: crate::domain::outbox::Attention,
    /// Where to reach the operator with every window closed. Only the operator
    /// sets it, for the reason [`attention`] is theirs, and more: the address
    /// and the token are credentials, so an agent's settings read never shows
    /// them and the webview only ever gets a hint.
    #[serde(default)]
    pub delivery: DeliveryConfig,
}

/// An address the Host sends a push to while the operator is away: an ntfy
/// topic or any receiver of a JSON POST. See `push.rs`.
///
/// `url` and `token` are secrets. A topic on a public ntfy server is readable
/// by anyone who knows it, so it is redacted like a key, and `Debug` is
/// written out by hand so that printing a config cannot print either.
#[derive(Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct DeliveryConfig {
    /// Empty means nothing is sent anywhere, which is the default.
    pub url: String,
    /// Sent as `Authorization: Bearer`, unless it already names a scheme.
    pub token: String,
    pub format: crate::push::Format,
    pub content: crate::push::Content,
    /// Where a tap on a push opens: the address of this Host as a phone can
    /// reach it. Empty means a push opens nothing.
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

/// Where an event is posted to fire a routine, and what the post has to carry.
///
/// Both are written by the app rather than typed by the operator, and both are
/// here rather than derived so that they survive a restart: whatever the
/// operator wired to post here was given one address and one secret, and an
/// app that picked a fresh port or a fresh secret on every launch would break
/// that wiring silently on every launch.
///
/// `port` is zero until the receiver has bound once, which tells it to take
/// whatever the OS offers and write that down. `secret` is empty until the
/// first launch generates one. `webhook.rs` is the reader of both.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct WebhookConfig {
    pub port: u16,
    pub secret: String,
}

/// Credentials for the sandboxes agents run their computers in.
///
/// App-wide rather than per group: it is one E2B account, and a sandbox is
/// billed to it no matter which crew asked for one.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct E2bConfig {
    pub api_key: String,
    /// Minutes of inactivity before a machine puts itself to sleep.
    ///
    /// Sleeping keeps the disk, so a browser stays signed in; it is the bill
    /// that stops, not the work. Refreshed on every use, so this is idle time
    /// rather than a lifetime.
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

/// Credentials for the hosted browsers agents use.
///
/// A second provider rather than a second setting on the first, because a
/// computer and a browser are different products bought from different people.
/// Either can be configured without the other: an operator who only wants
/// agents that use the web needs no machines, and one who only wants shells
/// needs no browsers. App-wide for the same reason as E2B's: it is one account,
/// and a browser is billed to it whichever crew asked for one.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct KernelConfig {
    pub api_key: String,
    /// Minutes of inactivity before a browser is deleted.
    ///
    /// Not the same thing as a machine's sleep. A browser goes to standby on its
    /// own after a few seconds, which keeps its state and stops the bill; this
    /// is how long after that it is thrown away. Nothing is lost when it is:
    /// the cookies were written back to the agent's profile, so the next browser
    /// opens signed in to the same accounts. Longer therefore costs nothing but
    /// saves the seconds a fresh one takes to boot.
    #[serde(default = "default_browser_idle_minutes")]
    pub idle_minutes: u32,
    /// Whether browsers are created in stealth mode.
    ///
    /// Off by default, and the operator's call rather than Guaca's. On, sites
    /// that block automation are far more likely to let an agent through, and
    /// the provider solves the captchas. It also costs more and needs a plan
    /// that includes it, so switching it on for everyone would make the first
    /// browser fail to start on accounts that do not have it.
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

/// Brings stored settings up to date, returning true if anything changed.
///
/// Defaults otherwise only ever reach a fresh install: anyone who had opened
/// Settings once was pinned to whatever the defaults were that day, which is
/// how a retuned limit failed to reach the person who needed it. Only values
/// that still match a superseded default are touched, so a number someone
/// actually chose is left alone.
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
    // Four was the hardcoded value before this was settable, and it is far too
    // few for an agent working a browser.
    if config.limits.max_tool_rounds == V0_LIMITS.max_tool_rounds {
        config.limits.max_tool_rounds = current.max_tool_rounds;
    }

    // Always worth persisting even when no limit moved: recording the version
    // is what stops this running again on every launch.
    config.version = CURRENT_VERSION;
    true
}

/// What the webview is allowed to see. The key itself never crosses the IPC
/// boundary, only whether one is set and its last four characters, which is
/// enough for an operator to tell two keys apart.
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
    /// A safe initial choice while the account's live catalog is loading.
    pub subscription_models: Vec<String>,
    /// Nothing secret in one, so it crosses whole.
    #[serde(default)]
    pub quick_actions: Vec<crate::domain::quick::QuickAction>,
    /// An artifact id and a width. The page itself is read when it is drawn.
    #[serde(default)]
    pub widgets: Vec<crate::domain::widget::Widget>,
    /// What may interrupt the operator, for every window on this Host.
    #[serde(default)]
    pub attention: crate::domain::outbox::Attention,
    /// The address pushes go to, as hints only.
    #[serde(default)]
    pub delivery: RedactedDelivery,
}

/// What the webview may know about the push address: whether there is one and
/// enough of it to recognize, never the topic or the token.
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

/// Last four characters of a key, for showing that one is set without showing
/// what it is. Shared with groups, which redact theirs the same way.
pub fn hint_for(key: &str) -> String {
    let key = key.trim();
    if key.is_empty() {
        return String::new();
    }
    let tail: String = key.chars().rev().take(4).collect::<Vec<_>>().into_iter().rev().collect();
    format!("...{tail}")
}

/// An address shown the way a key is: its scheme and host, which say which
/// server it is, and the last four characters of the rest, which tell two
/// topics apart. Never the path, which on ntfy is the topic and so the secret.
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

/// Checks an address the Host is to send to or open: http or https, with a
/// host. Refused without repeating what was typed, since what was typed may be
/// a topic nobody else should read in an error.
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

/// Normalizes and validates an operator-supplied base URL.
pub fn normalize_base_url(input: &str) -> Result<String, ConfigError> {
    let trimmed = input.trim().trim_end_matches('/');
    if !(trimmed.starts_with("http://") || trimmed.starts_with("https://")) {
        return Err(ConfigError::BadBaseUrl { got: input.to_string() });
    }
    // A base that already names the endpoint is a common paste error; accepting
    // it silently produces a 404 that looks like an auth problem.
    let cleaned = trimmed.trim_end_matches("/chat/completions").trim_end_matches('/');
    if cleaned.is_empty() {
        return Err(ConfigError::BadBaseUrl { got: input.to_string() });
    }
    Ok(cleaned.to_string())
}

// ---- changing settings ---------------------------------------------------

/// Absent fields are left alone. `apiKey: ""` clears the key; omitting it
/// keeps the existing one, which is what lets the UI show a redacted value
/// without ever round-tripping the secret.
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

/// The parts of the push address the operator changed. A blank `url` or
/// `token` removes it, and an absent one keeps it, as with the API key, which
/// is what lets the pane show a hint without ever holding the secret.
#[derive(Debug, Default, Clone, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct DeliveryPatch {
    pub url: Option<String>,
    pub token: Option<String>,
    pub format: Option<crate::push::Format>,
    pub content: Option<crate::push::Content>,
    pub link_base: Option<String>,
}

/// The switches the operator changed. Absent ones keep their value, so a
/// window that knows two of them cannot turn off a third it has never drawn.
#[derive(Debug, Default, Clone, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct AttentionPatch {
    pub needs_you: Option<bool>,
    pub failures: Option<bool>,
    pub finished: Option<bool>,
}

impl AppConfig {
    /// Applies a patch in memory: the one set of rules every writer shares, the
    /// settings pane in any client and an agent alike.
    ///
    /// Clamped rather than refused where a number is out of range, because the
    /// pane reads the stored value back and shows what was kept.
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
            // A machine that sleeps after zero minutes can never be used, and one
            // that never sleeps is a bill nobody chose.
            self.e2b.idle_minutes = minutes.clamp(1, 24 * 60);
        }
        if let Some(key) = patch.kernel_api_key {
            self.kernel.api_key = key.trim().to_string();
        }
        if let Some(minutes) = patch.browser_idle_minutes {
            // Wider at the top than the machine's, because a browser on standby
            // costs nothing and the provider allows three days. Wider at the bottom
            // is not possible: ten seconds is its floor, which is a fifth of a
            // minute, so one minute is as short as this can offer.
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

/// The settings an agent may ask to change, as `settings` names them.
///
/// Everything here changes what agents run on and nothing here reaches outside
/// the workspace. The operator approves each change, with its before and after.
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

/// What an agent may never change, even with approval. The endpoint decides
/// where the API key is sent, so an agent that could move it could send the
/// key anywhere; the keys themselves are never shown to it at all.
pub const OPERATOR_ONLY: &[&str] =
    &["provider", "baseUrl", "apiKey", "e2bApiKey", "kernelApiKey", "permissionRules"];

/// The five limits, as the settings pane labels them. `limits.ts` holds the same
/// words for the pane, and `manual.test.ts` checks the two agree.
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

/// The limits an agent names, each one read as its own type. Absent is left
/// alone; a null is refused, as it was when an agent's limits were read whole.
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

/// Reads the changes an agent asked for into a patch, refusing what it may
/// not touch with a sentence that says what it may.
///
/// The patch carries what the agent named and nothing else, a limit included.
/// It is applied when the operator allows it, which can be ten minutes after
/// it was asked for, and a limit carried at the value it had then would undo
/// whatever the operator saved in between.
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

/// One setting, before and after, in the words the settings pane uses.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Change {
    pub label: String,
    pub from: String,
    pub to: String,
}

/// What differs between two settings, among the ones an agent can change.
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

/// An address with any `user:password@` taken out, for showing to a model.
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

/// Reads stored settings, migrating and rewriting them if they are outdated.
pub fn load(path: &Path) -> Result<AppConfig, ConfigError> {
    let mut config = match fs::read_to_string(path) {
        Ok(raw) => serde_json::from_str::<AppConfig>(&raw)
            .map_err(|source| ConfigError::Parse { path: path.to_path_buf(), source })?,
        // A missing config is the first-run case, not an error.
        Err(e) if e.kind() == io::ErrorKind::NotFound => {
            return Ok(AppConfig { version: CURRENT_VERSION, ..Default::default() });
        }
        Err(source) => return Err(ConfigError::Io { path: path.to_path_buf(), source }),
    };

    if migrate(&mut config) {
        // Best effort: a read-only config directory should not stop startup,
        // and the migrated values are already correct in memory.
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

    // Write to a sibling temp file and rename, so a crash mid-write cannot
    // leave a truncated config that fails to parse on next launch.
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
    // Windows ACL defaults already scope a per-user config directory to that
    // user. Nothing useful to tighten without pulling in a platform crate.
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

        // One limit, and nothing about the four it did not name, which keep
        // whatever they are when the patch is applied rather than resetting.
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
        // Not writable by an agent, even asking, even with approval: an agent
        // that could turn off the operator's interruptions could do anything
        // else unannounced.
        for asked in [
            serde_json::json!({ "attention": { "needsYou": false } }),
            serde_json::json!({ "attention": { "failures": false, "finished": false } }),
            serde_json::json!({ "limits": { "maxHops": 12 }, "attention": { "needsYou": false } }),
        ] {
            let refused = agent_patch(&asked).unwrap_err();
            assert!(refused.contains("attention"), "{refused}");
        }
        assert!(!AGENT_WRITABLE.contains(&"attention"));

        // The operator's own patch sets only what it names.
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
        // Two spellings of one value, and the second is a `serde` attribute
        // nothing else would notice changing. If they ever disagree, a group's
        // stored provider reads as inherit and a whole crew quietly moves onto
        // the app's settings.
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
        // The case that bit a real user: they had opened Settings once, so the
        // old defaults were written to disk and retuning them changed nothing.
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
        // Someone who raised the hop limit by hand keeps their number, even
        // while the stale ones beside it are corrected.
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

        // And the fix is durable, not re-applied on every launch.
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
        // Resetting to defaults here would silently discard the operator's key.
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
