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
    /// own notes: an operator had to tell one agent their name, that agent
    /// stored it privately, and every other agent still had no idea who it was
    /// working for. Empty falls back to "the operator", which is what agents
    /// said before this existed.
    #[serde(default)]
    pub operator_name: String,
    pub inference: InferenceConfig,
    pub limits: GuardLimits,
    #[serde(default)]
    pub e2b: E2bConfig,
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
            computer_idle_minutes: self.e2b.idle_minutes,
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
