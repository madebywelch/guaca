//! Public release information. What the checker reads is news and nothing
//! more; [`signed`] is the only read anything installs from, and it refuses a
//! manifest that none of this build's keys signed.
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};
use tokio::sync::Mutex;

pub const SOURCE: &str =
    "https://github.com/madebywelch/guaca/releases/latest/download/guaca-release.json";

/// The keys a manifest may be signed with. A GitHub account that can publish
/// a release is not, by itself, able to make every box install it.
pub const KEYS: &str = include_str!("../../release-keys.pub");

/// Where each release's files are, by version. The signature is read from
/// here rather than from `latest`, so a release published between the two
/// reads is a mismatch rather than a pair of files from two releases.
pub const DOWNLOADS: &str = "https://github.com/madebywelch/guaca/releases/download";
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

#[derive(Clone, Debug, Deserialize, Serialize)]
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
        parse(&download(&client()?, &self.source).await?)
    }
}

/// The release at `source`, if one of `keys` signed exactly the bytes read.
///
/// The signature is checked before anything in the manifest is acted on; the
/// version is read first only to find the signature, and only after
/// `validate` has held it to a plain release number.
pub async fn signed(source: &str, downloads: &str, keys: &str) -> Result<Release, String> {
    let client = client()?;
    let bytes = download(&client, source).await?;
    let release = parse(&bytes)?;
    let signature = format!("{downloads}/v{}/guaca-release.json.sig", release.version);
    let said = download(&client, &signature).await.map_err(|error| {
        if error.contains("HTTP 404") {
            format!(
                "Guaca {} was published without a signature, so nothing was installed. This host updates to the next signed release.",
                release.version
            )
        } else {
            format!("The signature of Guaca {} could not be read, so nothing was installed. {error}", release.version)
        }
    })?;
    verify(&bytes, &said, keys)?;
    Ok(release)
}

/// Whether one of `keys` made `signature` over `bytes`.
fn verify(bytes: &[u8], signature: &[u8], keys: &str) -> Result<(), String> {
    use base64::Engine;
    let refused = || {
        "This release is not signed with Guaca's release key, so nothing was installed. Try again later; if it persists, the release was not published by Guaca.".to_string()
    };
    let signature = base64::engine::general_purpose::STANDARD
        .decode(signature.trim_ascii())
        .map_err(|_| refused())?;
    let trusted = keys
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty() && !line.starts_with('#'))
        .filter_map(|line| base64::engine::general_purpose::STANDARD.decode(line).ok());
    for key in trusted {
        let key = ring::signature::UnparsedPublicKey::new(&ring::signature::ED25519, key);
        if key.verify(bytes, &signature).is_ok() {
            return Ok(());
        }
    }
    Err(refused())
}

fn client() -> Result<reqwest::Client, String> {
    reqwest::Client::builder()
        .timeout(Duration::from_secs(10))
        .user_agent("Guaca release check")
        .build()
        .map_err(|_| "Could not start the release check. Try again.".to_string())
}

async fn download(client: &reqwest::Client, source: &str) -> Result<Vec<u8>, String> {
    let mut response = client.get(source).send().await.map_err(|_| {
        "Could not reach the release service. Check your connection and try again.".to_string()
    })?;
    if !response.status().is_success() {
        return Err(format!(
            "Release metadata is unavailable (HTTP {}). Try again later.",
            response.status().as_u16()
        ));
    }
    let mut bytes = Vec::new();
    while let Some(chunk) =
        response.chunk().await.map_err(|_| "The release download was interrupted. Try again.")?
    {
        if bytes.len() + chunk.len() > MAX_BYTES {
            return Err("The release metadata is too large. Contact the publisher.".into());
        }
        bytes.extend_from_slice(&chunk);
    }
    Ok(bytes)
}

fn parse(bytes: &[u8]) -> Result<Release, String> {
    let release: Release = serde_json::from_slice(bytes)
        .map_err(|_| "The release metadata is unreadable. Try again later.".to_string())?;
    release.validate()?;
    Ok(release)
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
    /// A key pair nobody else holds, and its public half as `KEYS` spells one.
    fn keypair() -> (ring::signature::Ed25519KeyPair, String) {
        use base64::Engine;
        use ring::signature::KeyPair;
        let document =
            ring::signature::Ed25519KeyPair::generate_pkcs8(&ring::rand::SystemRandom::new())
                .unwrap();
        let pair = ring::signature::Ed25519KeyPair::from_pkcs8(document.as_ref()).unwrap();
        let public = base64::engine::general_purpose::STANDARD.encode(pair.public_key().as_ref());
        (pair, public)
    }

    fn sign(pair: &ring::signature::Ed25519KeyPair, bytes: &[u8]) -> String {
        use base64::Engine;
        base64::engine::general_purpose::STANDARD.encode(pair.sign(bytes).as_ref())
    }

    /// Serves one manifest and, beside it, whatever signature it is given.
    async fn publish(
        manifest: Vec<u8>,
        signature: Option<String>,
    ) -> (String, tokio::task::JoinHandle<()>) {
        let app = axum::Router::new()
            .route("/guaca-release.json", axum::routing::get(move || async move { manifest }))
            .route(
                "/v0.2.0/guaca-release.json.sig",
                axum::routing::get(move || async move {
                    match signature {
                        Some(said) => (axum::http::StatusCode::OK, said),
                        None => (axum::http::StatusCode::NOT_FOUND, String::new()),
                    }
                }),
            );
        let socket = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let base = format!("http://{}", socket.local_addr().unwrap());
        let task = tokio::spawn(async move {
            axum::serve(socket, app).await.unwrap();
        });
        (base, task)
    }

    #[tokio::test]
    async fn only_a_manifest_one_of_the_keys_signed_is_installable() {
        let (pair, public) = keypair();
        let (stranger, _) = keypair();
        let bytes = serde_json::to_vec_pretty(&release()).unwrap();
        let mut tampered = release();
        tampered.image = format!("ghcr.io/madebywelch/guaca/guacad@sha256:{}", "d".repeat(64));
        let tampered = serde_json::to_vec_pretty(&tampered).unwrap();
        let rotated = format!("# rotation\n{}\n{public}\n", keypair().1);
        for (manifest, signature, keys, accepted) in [
            (bytes.clone(), Some(sign(&pair, &bytes)), public.clone(), true),
            (bytes.clone(), Some(sign(&pair, &bytes)), rotated, true),
            (tampered.clone(), Some(sign(&pair, &bytes)), public.clone(), false),
            (bytes.clone(), Some(sign(&stranger, &bytes)), public.clone(), false),
            (bytes.clone(), Some("not base64".into()), public.clone(), false),
            (bytes.clone(), None, public.clone(), false),
        ] {
            let (base, task) = publish(manifest, signature).await;
            let result = signed(&format!("{base}/guaca-release.json"), &base, &keys).await;
            assert_eq!(result.is_ok(), accepted, "{result:?}");
            if let Err(error) = result {
                assert!(error.contains("nothing was installed"), "{error}");
                assert!(!error.contains("HTTP"), "said to an operator, not a developer: {error}");
            }
            task.abort();
        }
    }

    #[test]
    fn this_build_trusts_at_least_one_well_formed_key() {
        use base64::Engine;
        let keys: Vec<Vec<u8>> = KEYS
            .lines()
            .map(str::trim)
            .filter(|line| !line.is_empty() && !line.starts_with('#'))
            .map(|line| base64::engine::general_purpose::STANDARD.decode(line).unwrap())
            .collect();
        assert!(!keys.is_empty());
        assert!(keys.iter().all(|key| key.len() == 32), "Ed25519 public keys are 32 bytes");
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
