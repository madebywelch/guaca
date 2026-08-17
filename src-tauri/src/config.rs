//! Operator configuration.
//!
//! Stored as JSON next to the database in the platform config directory. The
//! API key lives here in plaintext, which is worth being blunt about: Guac is
//! a local, no-auth app, and a key encrypted with a key stored beside it is
//! theatre. The file is written 0600 and the key is never sent to the webview.
//! If you want real secret storage, the honest answer is the OS keychain, and
//! that is a deliberate follow-up rather than something faked here.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::domain::computer::{Provider, ProviderChoice};
use crate::runtime::guard::GuardLimits;

pub const DEFAULT_BASE_URL: &str = "https://openrouter.ai/api/v1";
pub const DEFAULT_MODEL: &str = "anthropic/claude-sonnet-4.5";

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct InferenceConfig {
    /// Any OpenAI-compatible base. Swappable so a local llama.cpp or LM Studio
    /// endpoint works without a code change.
    pub base_url: String,
    #[serde(default)]
    pub api_key: String,
    pub default_model: String,
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

impl Default for InferenceConfig {
    fn default() -> Self {
        Self {
            base_url: DEFAULT_BASE_URL.to_string(),
            api_key: String::new(),
            default_model: DEFAULT_MODEL.to_string(),
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

    pub fn is_ready(&self) -> bool {
        !self.api_key.trim().is_empty() && !self.base_url.trim().is_empty()
    }
}

/// Bumped whenever stored settings need adjusting on load.
pub const CURRENT_VERSION: u32 = 2;

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
    pub computer: ComputerConfig,
    #[serde(default)]
    pub e2b: E2bConfig,
}

/// How agents get a machine, independent of who ends up running it.
///
/// Provider-neutral because the setting outlives the provider: an operator who
/// moves from a hosted sandbox to a local one keeps the same idle time and the
/// same install, and a setting that lived under `e2b` would have quietly meant
/// nothing the moment they moved.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct ComputerConfig {
    /// Who to ask for a new machine. `Automatic` is resolved once per computer
    /// and written to its row; existing computers keep whoever made them.
    pub provider: ProviderChoice,
    /// Minutes of inactivity before a machine puts itself to sleep.
    ///
    /// Sleeping keeps the disk, so a browser stays signed in; it is the bill
    /// that stops, not the work. Refreshed on every use, so this is idle time
    /// rather than a lifetime.
    pub idle_minutes: u32,
    /// Labels the resources this install makes, so a machine belonging to
    /// another copy of Guac on the same Mac is never swept up as an orphan.
    /// Generated once and then never changed.
    pub installation_id: String,
}

impl Default for ComputerConfig {
    fn default() -> Self {
        Self {
            provider: ProviderChoice::Automatic,
            idle_minutes: default_idle_minutes(),
            // Empty until `migrate` mints one, which is also how a fresh
            // install gets one: there is exactly one place that generates it.
            installation_id: String::new(),
        }
    }
}

/// Credentials for the sandboxes agents run their computers in.
///
/// App-wide rather than per group: it is one E2B account, and a sandbox is
/// billed to it no matter which crew asked for one.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct E2bConfig {
    pub api_key: String,
    /// Where the idle setting used to live, kept only so `migrate` can move a
    /// stored one into `computer`. Read there and nowhere else, and never
    /// written back: two places to read one setting is how they drift.
    #[serde(default, skip_serializing)]
    pub idle_minutes: Option<u32>,
}

pub fn default_idle_minutes() -> u32 {
    15
}

/// Brings stored settings up to date, returning true if anything changed.
///
/// Defaults otherwise only ever reach a fresh install: anyone who had opened
/// Settings once was pinned to whatever the defaults were that day, which is
/// how a retuned limit failed to reach the person who needed it. Only values
/// that still match a superseded default are touched, so a number someone
/// actually chose is left alone.
///
/// Each step is gated on the version it was written for, so a config two
/// versions behind gets both in order and one a single version behind is not
/// put through a step that already ran against it.
pub fn migrate(config: &mut AppConfig) -> bool {
    if config.version >= CURRENT_VERSION {
        return false;
    }

    if config.version < 1 {
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
        // Four was the hardcoded value before this was settable, and it is far
        // too few for an agent working a browser.
        if config.limits.max_tool_rounds == V0_LIMITS.max_tool_rounds {
            config.limits.max_tool_rounds = current.max_tool_rounds;
        }
    }

    if config.version < 2 {
        if config.computer.installation_id.is_empty() {
            config.computer.installation_id = uuid::Uuid::new_v4().to_string();
        }
        // An install that has an E2B key is pinned to E2B rather than left to
        // choose: automatic would pick a local provider on a Mac that supports
        // one, and an operator who paid for a hosted sandbox did not ask to
        // move. Only when nothing has been chosen, so this can never overwrite
        // a decision made later.
        if config.computer.provider == ProviderChoice::Automatic
            && !config.e2b.api_key.trim().is_empty()
        {
            config.computer.provider = ProviderChoice::Provider(Provider::E2b);
        }
        if let Some(minutes) = config.e2b.idle_minutes.take() {
            config.computer.idle_minutes = minutes;
        }
    }

    // Always worth persisting even when nothing else moved: recording the
    // version is what stops this running again on every launch.
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
    pub computer_provider: ProviderChoice,
    pub computer_idle_minutes: u32,
    pub base_url: String,
    pub default_model: String,
    pub api_key_set: bool,
    pub api_key_hint: String,
    pub request_timeout_secs: u64,
    pub limits: GuardLimits,
}

impl AppConfig {
    pub fn redacted(&self) -> RedactedConfig {
        RedactedConfig {
            operator_name: self.operator_name.clone(),
            e2b_key_set: !self.e2b.api_key.trim().is_empty(),
            e2b_key_hint: hint_for(&self.e2b.api_key),
            computer_provider: self.computer.provider,
            computer_idle_minutes: self.computer.idle_minutes,
            base_url: self.inference.base_url.clone(),
            default_model: self.inference.default_model.clone(),
            api_key_set: !self.inference.api_key.trim().is_empty(),
            api_key_hint: hint_for(&self.inference.api_key),
            request_timeout_secs: self.inference.request_timeout_secs,
            limits: self.limits,
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
    #[error("model must not be blank")]
    BlankModel,
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

/// Reads stored settings, migrating and rewriting them if they are outdated.
pub fn load(path: &Path) -> Result<AppConfig, ConfigError> {
    let mut config = match fs::read_to_string(path) {
        Ok(raw) => serde_json::from_str::<AppConfig>(&raw)
            .map_err(|source| ConfigError::Parse { path: path.to_path_buf(), source })?,
        // A missing config is the first-run case, not an error. Migrated from
        // version 0 like any other, and then written: a fresh install needs an
        // installation id as much as an old one, and one minted per launch
        // would orphan every resource the launch before it made.
        Err(e) if e.kind() == io::ErrorKind::NotFound => AppConfig::default(),
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
    fn a_v1_install_with_an_e2b_key_keeps_using_e2b_explicitly() {
        // Automatic would pick a local provider on a supported Mac, and an
        // operator who paid for E2B did not ask to move.
        let mut config = AppConfig { version: 1, ..Default::default() };
        config.e2b.api_key = "e2b_key".into();
        config.e2b.idle_minutes = Some(42);

        assert!(migrate(&mut config));

        assert_eq!(config.computer.provider, ProviderChoice::Provider(Provider::E2b));
        assert_eq!(config.computer.idle_minutes, 42, "the idle setting moved with its meaning");
        assert_eq!(config.e2b.idle_minutes, None, "and left nothing behind to drift from");
        assert!(uuid::Uuid::parse_str(&config.computer.installation_id).is_ok());
        assert_eq!(config.version, CURRENT_VERSION);
    }

    #[test]
    fn a_v1_install_without_a_key_becomes_automatic_and_a_fresh_one_starts_there() {
        let mut config = AppConfig { version: 1, ..Default::default() };
        migrate(&mut config);
        assert_eq!(config.computer.provider, ProviderChoice::Automatic);
        assert_eq!(config.computer.idle_minutes, default_idle_minutes());

        let dir = tempfile::tempdir().unwrap();
        let fresh = load(&dir.path().join("config.json")).unwrap();
        assert_eq!(fresh.computer.provider, ProviderChoice::Automatic);
        assert!(
            !fresh.computer.installation_id.is_empty(),
            "a fresh install labels its resources from day one"
        );
    }

    #[test]
    fn a_fresh_install_keeps_the_same_installation_id_on_the_next_launch() {
        // The label on every resource this app makes. A new one each launch
        // would orphan everything the last one created.
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.json");
        let first = load(&path).unwrap();
        assert_eq!(load(&path).unwrap().computer.installation_id, first.computer.installation_id);
    }

    #[test]
    fn migration_v1_to_v2_runs_once_and_keeps_the_installation_id() {
        let mut config = AppConfig { version: 1, ..Default::default() };
        migrate(&mut config);
        let id = config.computer.installation_id.clone();
        assert!(!migrate(&mut config));
        assert_eq!(config.computer.installation_id, id);
    }

    #[test]
    fn a_v2_file_naming_a_provider_is_never_re_migrated_to_e2b() {
        // The operator moved a keyed install to a local provider on purpose.
        let mut config = AppConfig {
            version: CURRENT_VERSION,
            computer: ComputerConfig {
                provider: ProviderChoice::Provider(Provider::AppleContainer),
                ..Default::default()
            },
            ..Default::default()
        };
        config.e2b.api_key = "e2b_key".into();

        assert!(!migrate(&mut config));
        assert_eq!(config.computer.provider, ProviderChoice::Provider(Provider::AppleContainer));
    }

    #[test]
    fn a_v1_file_on_disk_round_trips_through_v2() {
        // What is actually in ~/Library/Application Support: e2b.idleMinutes.
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.json");
        fs::write(&path, r#"{"version":1,"e2b":{"apiKey":"k","idleMinutes":7}}"#).unwrap();

        let cfg = load(&path).unwrap();
        assert_eq!(cfg.computer.idle_minutes, 7);
        assert_eq!(cfg.computer.provider, ProviderChoice::Provider(Provider::E2b));

        // load() rewrites a migrated config in place, and the legacy field is
        // not written back: two places to read one setting is how they drift.
        let raw: serde_json::Value =
            serde_json::from_str(&fs::read_to_string(&path).unwrap()).unwrap();
        assert_eq!(raw["computer"]["idleMinutes"], 7);
        assert_eq!(raw["computer"]["provider"], "e2b");
        assert!(raw["e2b"]["idleMinutes"].is_null(), "{raw}");
        assert_eq!(raw["e2b"]["apiKey"], "k", "the key it was migrated for is still there");
        assert_eq!(load(&path).unwrap(), cfg, "and the rewrite reloads unchanged");
    }

    #[test]
    fn a_computer_section_that_only_names_a_provider_keeps_the_default_idle_time() {
        // Zero would be a machine that falls asleep between two commands.
        let cfg: AppConfig =
            serde_json::from_str(r#"{"version":2,"computer":{"provider":"e2b"}}"#).unwrap();
        assert_eq!(cfg.computer.idle_minutes, default_idle_minutes());
        assert_eq!(cfg.computer.provider, ProviderChoice::Provider(Provider::E2b));
    }

    #[test]
    fn a_config_naming_a_provider_this_build_cannot_drive_is_refused() {
        // Silently reading it as automatic would run an agent's machine
        // somewhere the operator did not choose.
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.json");
        fs::write(&path, r#"{"version":2,"computer":{"provider":"docker"}}"#).unwrap();

        let err = load(&path).unwrap_err();
        assert!(matches!(err, ConfigError::Parse { .. }), "{err}");
        assert!(err.to_string().contains("docker"), "{err}");
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
