//! Public release information is advisory. Only the native app's embedded
//! image reference may be installed by its local host manager.
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};
use tokio::sync::Mutex;

pub const SOURCE: &str =
    "https://github.com/madebywelch/guaca/releases/latest/download/guaca-release.json";
const MAX_BYTES: usize = 16 * 1024;
const CACHE_FOR: Duration = Duration::from_secs(6 * 60 * 60);
const RETRY_AFTER: Duration = Duration::from_secs(60);

#[derive(Clone, Deserialize, Serialize)]
pub struct Protocol {
    pub generation: u32,
    pub minimum: u32,
    pub maximum: u32,
}

pub fn protocol() -> Protocol {
    serde_json::from_str(include_str!("../../release-protocol.json"))
        .expect("release-protocol.json is checked by the build gates")
}

pub fn metadata() -> serde_json::Value {
    serde_json::json!({
        "version": env!("CARGO_PKG_VERSION"),
        "release": option_env!("GUACA_RELEASE") == Some("1"),
        "apiGeneration": protocol().generation,
    })
}

#[derive(Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Release {
    pub schema: u32,
    pub version: String,
    pub commit: String,
    pub image: String,
    pub api_generation: u32,
    pub client_minimum: u32,
    pub client_maximum: u32,
    pub notes: String,
}

impl Release {
    pub fn validate(&self) -> Result<(), String> {
        let version = semver::Version::parse(&self.version)
            .map_err(|_| "The release has an invalid version.")?;
        let digest = self.image.strip_prefix("ghcr.io/madebywelch/guaca/guacad@sha256:");
        if self.schema != 1
            || !version.pre.is_empty()
            || !version.build.is_empty()
            || !hex(&self.commit, 40)
            || !digest.is_some_and(|value| hex(value, 64))
            || self.api_generation == 0
            || self.client_minimum == 0
            || self.client_minimum > self.client_maximum
            || self.api_generation < self.client_minimum
            || self.api_generation > self.client_maximum
            || self.notes
                != format!("https://github.com/madebywelch/guaca/releases/tag/v{}", self.version)
        {
            return Err("The release metadata could not be verified. Try again after the publisher fixes the release.".into());
        }
        Ok(())
    }
}

fn hex(value: &str, length: usize) -> bool {
    value.len() == length && value.bytes().all(|c| c.is_ascii_digit() || (b'a'..=b'f').contains(&c))
}

#[derive(Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Status {
    pub automatic: bool,
    pub checked_at: Option<String>,
    pub latest: Option<Release>,
    pub error: Option<String>,
}

#[derive(Default)]
struct Cached {
    status: Status,
    attempted: Option<Instant>,
}

pub struct Checker {
    source: String,
    automatic: bool,
    cached: Mutex<Cached>,
}

impl Default for Checker {
    fn default() -> Self {
        Self::new(SOURCE.into(), std::env::var("GUACA_UPDATE_CHECKS").as_deref() != Ok("off"))
    }
}

impl Checker {
    fn new(source: String, automatic: bool) -> Self {
        Self { source, automatic, cached: Mutex::new(Cached::default()) }
    }

    pub async fn check(&self, refresh: bool) -> Status {
        // Serializing the read also coalesces checks from several clients.
        let mut cached = self.cached.lock().await;
        cached.status.automatic = self.automatic;
        let lifetime =
            if refresh || cached.status.error.is_some() { RETRY_AFTER } else { CACHE_FOR };
        if (!self.automatic && !refresh)
            || cached.attempted.is_some_and(|time| time.elapsed() < lifetime)
        {
            return cached.status.clone();
        }
        cached.attempted = Some(Instant::now());
        match self.fetch().await {
            Ok(release) => {
                cached.status.latest = Some(release);
                cached.status.checked_at = Some(chrono::Utc::now().to_rfc3339());
                cached.status.error = None;
            }
            Err(error) => {
                tracing::warn!(%error, "could not check host releases");
                // Retain the last successful answer, with its original timestamp.
                cached.status.error = Some(error);
            }
        }
        cached.status.clone()
    }

    async fn fetch(&self) -> Result<Release, String> {
        let client = reqwest::Client::builder()
            .timeout(Duration::from_secs(10))
            .user_agent("Guaca release check")
            .build()
            .map_err(|_| "Could not start the release check. Try again.".to_string())?;
        let mut response = client.get(&self.source).send().await.map_err(|_| {
            "Could not reach the release service. Check your connection and try again.".to_string()
        })?;
        if !response.status().is_success() {
            return Err(format!(
                "Release metadata is unavailable (HTTP {}). Try again later.",
                response.status().as_u16()
            ));
        }
        let mut bytes = Vec::new();
        while let Some(chunk) = response
            .chunk()
            .await
            .map_err(|_| "The release download was interrupted. Try again.")?
        {
            if bytes.len() + chunk.len() > MAX_BYTES {
                return Err("The release metadata is too large. Contact the publisher.".into());
            }
            bytes.extend_from_slice(&chunk);
        }
        let release: Release = serde_json::from_slice(&bytes)
            .map_err(|_| "The release metadata is unreadable. Try again later.".to_string())?;
        release.validate()?;
        Ok(release)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn release() -> Release {
        Release {
            schema: 1,
            version: "0.2.0".into(),
            commit: "a".repeat(40),
            image: format!("ghcr.io/madebywelch/guaca/guacad@sha256:{}", "b".repeat(64)),
            api_generation: 1,
            client_minimum: 1,
            client_maximum: 1,
            notes: "https://github.com/madebywelch/guaca/releases/tag/v0.2.0".into(),
        }
    }
    #[test]
    fn refuses_untrusted_or_unordered_release_metadata() {
        for bad in ["0.2.0-beta.1", "0.2.0+custom", "latest", "01.2.0"] {
            let mut value = release();
            value.version = bad.into();
            assert!(value.validate().is_err());
        }
        let mut value = release();
        value.image = "attacker/image:latest".into();
        assert!(value.validate().is_err());
        let mut value = release();
        value.notes = "javascript:alert(1)".into();
        assert!(value.validate().is_err());
        let mut value = release();
        value.client_maximum = 0;
        assert!(value.validate().is_err());
        assert!(release().validate().is_ok());
    }
    #[tokio::test]
    async fn offline_checks_keep_the_previous_release_and_timestamp() {
        let checker = Checker::new("http://127.0.0.1:1".into(), true);
        {
            let mut cached = checker.cached.lock().await;
            cached.status.latest = Some(release());
            cached.status.checked_at = Some("previous check".into());
        }
        let result = checker.check(true).await;
        assert!(result.error.is_some());
        assert_eq!(result.latest.unwrap().version, "0.2.0");
        assert_eq!(result.checked_at.as_deref(), Some("previous check"));
    }
    #[tokio::test]
    async fn disabled_checks_do_not_contact_the_source() {
        let checker = Checker::new("not a URL".into(), false);
        let result = checker.check(false).await;
        assert!(!result.automatic);
        assert!(result.error.is_none());
        assert!(result.checked_at.is_none());
    }
    #[tokio::test]
    async fn concurrent_and_manual_checks_share_a_rate_limit() {
        use std::sync::{
            atomic::{AtomicUsize, Ordering},
            Arc,
        };
        let count = Arc::new(AtomicUsize::new(0));
        let calls = count.clone();
        let app = axum::Router::new().route(
            "/",
            axum::routing::get(move || {
                let calls = calls.clone();
                async move {
                    calls.fetch_add(1, Ordering::SeqCst);
                    axum::Json(release())
                }
            }),
        );
        let socket = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let checker = Checker::new(format!("http://{}/", socket.local_addr().unwrap()), true);
        let task = tokio::spawn(async move {
            axum::serve(socket, app).await.unwrap();
        });
        let (a, b) = tokio::join!(checker.check(true), checker.check(true));
        assert!(a.error.is_none());
        assert!(b.latest.is_some());
        assert_eq!(count.load(Ordering::SeqCst), 1);
        task.abort();
    }
}
