//! Public release information. What the checker reads is news and nothing
//! more; [`signed`] and [`feed`] are the only reads anything installs from,
//! and each refuses a manifest that none of its channel's keys signed.
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};
use tokio::sync::Mutex;

pub const SOURCE: &str =
    "https://github.com/madebywelch/guaca/releases/latest/download/guaca-release.json";

/// The keys a manifest may be signed with. A GitHub account that can publish
/// a release is not, by itself, able to make every box install it.
pub const KEYS: &str = include_str!("../../release-keys.pub");

/// The newest build of `main`, as CI published it: one file carrying the
/// manifest and its signature, because the file is replaced on every push
/// and two files replaced one after the other can be read one from each. On
/// a branch, because this repository's releases are immutable.
pub const MAIN_FEED: &str =
    "https://raw.githubusercontent.com/madebywelch/guaca/refs/heads/main-feed/guaca-main.json";

/// The keys a build of `main` may be signed with. Not the release keys: CI
/// holds this one, and a leak of it reaches only the boxes that chose `main`.
pub const MAIN_KEYS: &str = include_str!("../../main-keys.pub");

/// How long the checker keeps an answer about `main`. A push is news within
/// minutes on that channel, which is what choosing it asked for.
const MAIN_CACHE_FOR: Duration = Duration::from_secs(5 * 60);

/// Which builds a box installs: published releases, or every build of `main`.
///
/// Set where the box is installed, in `GUACA_CHANNEL`, and read from the
/// updater's own environment. Nothing that can reach the updater's socket can
/// change it, so an agent in the host cannot move a release box onto `main`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Channel {
    #[default]
    Release,
    Main,
}

impl Channel {
    pub fn from_env() -> Result<Channel, String> {
        Channel::named(std::env::var("GUACA_CHANNEL").ok().as_deref())
    }

    fn named(said: Option<&str>) -> Result<Channel, String> {
        match said.map(str::trim) {
            None | Some("") | Some("release") => Ok(Channel::Release),
            Some("main") => Ok(Channel::Main),
            Some(other) => Err(format!(
                "GUACA_CHANNEL is {other:?}. Set it to release or main, or leave it out for release."
            )),
        }
    }

    fn is_release(&self) -> bool {
        *self == Channel::Release
    }
}

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

/// The commit this daemon was built from, told to the build rather than read
/// from a repository it does not ship with. Empty for a build made without
/// one, which `/health` says rather than hides: a box and a laptop that
/// disagree about this string are running different code, and that is the
/// first thing worth knowing about a bug that reproduces on one of them.
pub const BUILD: &str = match option_env!("GUACA_COMMIT") {
    Some(commit) => commit,
    None => "",
};

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
    /// Absent from a release manifest, which predates channels and is read by
    /// updaters that refuse a field they do not know.
    #[serde(default, skip_serializing_if = "Channel::is_release")]
    pub channel: Channel,
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
            || self.notes != self.notes_for()
        {
            return Err("The release metadata could not be verified. Try again after the publisher fixes the release.".into());
        }
        Ok(())
    }

    /// The one page a manifest may link: a release's notes, or the commit a
    /// build of `main` was made from.
    fn notes_for(&self) -> String {
        match self.channel {
            Channel::Release => {
                format!("https://github.com/madebywelch/guaca/releases/tag/v{}", self.version)
            }
            Channel::Main => format!("https://github.com/madebywelch/guaca/commit/{}", self.commit),
        }
    }
}

fn hex(value: &str, length: usize) -> bool {
    value.len() == length && value.bytes().all(|c| c.is_ascii_digit() || (b'a'..=b'f').contains(&c))
}

#[derive(Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Status {
    pub channel: Channel,
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

/// News about what an update would install. Which channel's news is not the
/// checker's to decide: a box's updater follows one, and a host without an
/// updater follows releases, so the caller says which on every check.
pub struct Checker {
    release: String,
    main: String,
    automatic: bool,
    cached: Mutex<Cached>,
}

impl Default for Checker {
    fn default() -> Self {
        Self::new(
            SOURCE.into(),
            MAIN_FEED.into(),
            std::env::var("GUACA_UPDATE_CHECKS").as_deref() != Ok("off"),
        )
    }
}

impl Checker {
    fn new(release: String, main: String, automatic: bool) -> Self {
        Self { release, main, automatic, cached: Mutex::new(Cached::default()) }
    }

    /// The news for `channel`, or for whichever channel was asked about last
    /// when the caller cannot say, which is an updater replacing itself.
    pub async fn check(&self, channel: Option<Channel>, refresh: bool) -> Status {
        // Serializing the read also coalesces checks from several clients.
        let mut cached = self.cached.lock().await;
        let channel = channel.unwrap_or(cached.status.channel);
        if channel != cached.status.channel {
            // A release is not news on main, nor the other way round.
            *cached = Cached::default();
            cached.status.channel = channel;
        }
        cached.status.automatic = self.automatic;
        let settled = match channel {
            Channel::Release => CACHE_FOR,
            Channel::Main => MAIN_CACHE_FOR,
        };
        let lifetime = if refresh || cached.status.error.is_some() { RETRY_AFTER } else { settled };
        if (!self.automatic && !refresh)
            || cached.attempted.is_some_and(|time| time.elapsed() < lifetime)
        {
            return cached.status.clone();
        }
        cached.attempted = Some(Instant::now());
        match self.fetch(channel).await {
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

    async fn fetch(&self, channel: Channel) -> Result<Release, String> {
        match channel {
            Channel::Release => {
                parse(&download(&client()?, &self.release).await?, Channel::Release)
            }
            // One read either way, so the news is checked as an install would be.
            Channel::Main => feed(&self.main, MAIN_KEYS).await,
        }
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
    let release = parse(&bytes, Channel::Release)?;
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
    verify(&bytes, &said, keys, Channel::Release)?;
    Ok(release)
}

/// What `MAIN_FEED` is: a manifest's exact bytes and the signature over them,
/// both base64.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Envelope {
    manifest: String,
    signature: String,
}

/// The build `main` is at, if one of `keys` signed exactly the manifest read.
pub async fn feed(source: &str, keys: &str) -> Result<Release, String> {
    use base64::Engine;
    let bytes = download(&client()?, source).await?;
    let unreadable = || {
        "The latest build of main is unreadable, so nothing was installed. Try again after the next push.".to_string()
    };
    let envelope: Envelope = serde_json::from_slice(&bytes).map_err(|_| unreadable())?;
    let manifest = base64::engine::general_purpose::STANDARD
        .decode(envelope.manifest.trim())
        .map_err(|_| unreadable())?;
    verify(&manifest, envelope.signature.as_bytes(), keys, Channel::Main)?;
    parse(&manifest, Channel::Main)
}

const UNSIGNED_RELEASE: &str = "This release is not signed with Guaca's release key, so nothing was installed. Try again later; if it persists, the release was not published by Guaca.";
const UNSIGNED_MAIN: &str = "This build of main is not signed with Guaca's key for main, so nothing was installed. Try again after the next push; if it persists, the build was not published by Guaca's CI.";

/// Whether one of `keys` made `signature` over `bytes`.
fn verify(bytes: &[u8], signature: &[u8], keys: &str, channel: Channel) -> Result<(), String> {
    use base64::Engine;
    let said = match channel {
        Channel::Release => UNSIGNED_RELEASE,
        Channel::Main => UNSIGNED_MAIN,
    };
    let refused = || said.to_string();
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

/// A manifest, held to the channel it was read for: a build of `main` read
/// where a release belongs is refused, and the other way round.
fn parse(bytes: &[u8], channel: Channel) -> Result<Release, String> {
    let release: Release = serde_json::from_slice(bytes)
        .map_err(|_| "The release metadata is unreadable. Try again later.".to_string())?;
    if release.channel != channel {
        return Err("The release metadata could not be verified. Try again after the publisher fixes the release.".into());
    }
    release.validate()?;
    Ok(release)
}

#[cfg(test)]
mod tests {
    use super::*;
    fn release() -> Release {
        Release {
            schema: 1,
            channel: Channel::Release,
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
        let checker = Checker::new("http://127.0.0.1:1".into(), String::new(), true);
        {
            let mut cached = checker.cached.lock().await;
            cached.status.latest = Some(release());
            cached.status.checked_at = Some("previous check".into());
        }
        let result = checker.check(Some(Channel::Release), true).await;
        assert!(result.error.is_some());
        assert_eq!(result.latest.unwrap().version, "0.2.0");
        assert_eq!(result.checked_at.as_deref(), Some("previous check"));
    }
    #[tokio::test]
    async fn disabled_checks_do_not_contact_the_source() {
        let checker = Checker::new("not a URL".into(), "not a URL".into(), false);
        let result = checker.check(None, false).await;
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
        let checker =
            Checker::new(format!("http://{}/", socket.local_addr().unwrap()), String::new(), true);
        let task = tokio::spawn(async move {
            axum::serve(socket, app).await.unwrap();
        });
        let (a, b) = tokio::join!(
            checker.check(Some(Channel::Release), true),
            checker.check(Some(Channel::Release), true)
        );
        assert!(a.error.is_none());
        assert!(b.latest.is_some());
        assert_eq!(count.load(Ordering::SeqCst), 1);
        task.abort();
    }

    /// A build of `main`, as CI describes one.
    fn build() -> Release {
        let commit = "e".repeat(40);
        Release {
            channel: Channel::Main,
            notes: format!("https://github.com/madebywelch/guaca/commit/{commit}"),
            commit,
            ..release()
        }
    }

    /// What CI publishes: the manifest's exact bytes and a signature over them.
    fn envelope(manifest: &[u8], signature: &str) -> Vec<u8> {
        use base64::Engine;
        serde_json::to_vec(&serde_json::json!({
            "manifest": base64::engine::general_purpose::STANDARD.encode(manifest),
            "signature": signature,
        }))
        .unwrap()
    }

    async fn serve(body: Vec<u8>) -> (String, tokio::task::JoinHandle<()>) {
        let app = axum::Router::new()
            .route("/guaca-main.json", axum::routing::get(move || async move { body }));
        let socket = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("http://{}/guaca-main.json", socket.local_addr().unwrap());
        let task = tokio::spawn(async move {
            axum::serve(socket, app).await.unwrap();
        });
        (url, task)
    }

    #[test]
    fn a_build_of_main_links_its_commit_and_nothing_else() {
        assert!(build().validate().is_ok());
        let mut value = build();
        value.notes = release().notes;
        assert!(value.validate().is_err(), "a build of main has no release notes to link");
        let mut value = release();
        value.notes = build().notes;
        assert!(value.validate().is_err());
        let mut value = build();
        value.version = "0.2.0-main".into();
        assert!(value.validate().is_err(), "main carries the version it will be released as");
    }

    #[test]
    fn a_manifest_is_refused_on_the_other_channel() {
        let main = serde_json::to_vec(&build()).unwrap();
        let published = serde_json::to_vec(&release()).unwrap();
        assert!(parse(&main, Channel::Main).is_ok());
        assert!(parse(&published, Channel::Release).is_ok());
        assert!(parse(&main, Channel::Release).is_err(), "main at the release address");
        assert!(parse(&published, Channel::Main).is_err(), "a release in main's feed");
        assert!(
            !String::from_utf8(published).unwrap().contains("channel"),
            "a release manifest stays readable by updaters that refuse unknown fields"
        );
    }

    #[tokio::test]
    async fn only_a_build_the_main_key_signed_is_installable() {
        let (pair, public) = keypair();
        let (stranger, _) = keypair();
        let bytes = serde_json::to_vec_pretty(&build()).unwrap();
        let mut tampered = build();
        tampered.image = format!("ghcr.io/madebywelch/guaca/guacad@sha256:{}", "d".repeat(64));
        let tampered = serde_json::to_vec_pretty(&tampered).unwrap();
        let published = serde_json::to_vec_pretty(&release()).unwrap();
        for (body, accepted) in [
            (envelope(&bytes, &sign(&pair, &bytes)), true),
            (envelope(&tampered, &sign(&pair, &bytes)), false),
            (envelope(&bytes, &sign(&stranger, &bytes)), false),
            (envelope(&published, &sign(&pair, &published)), false),
            (envelope(&bytes, "not base64"), false),
            (bytes.clone(), false),
            (b"<html>".to_vec(), false),
        ] {
            let (url, task) = serve(body).await;
            let result = feed(&url, &public).await;
            assert_eq!(result.is_ok(), accepted, "{result:?}");
            if let Err(error) = result {
                assert!(
                    error.contains("nothing was installed")
                        || error.contains("could not be verified"),
                    "{error}"
                );
            }
            task.abort();
        }
    }

    #[tokio::test]
    async fn a_checker_on_main_reads_the_feed_and_says_which_channel_it_follows() {
        let (pair, public) = keypair();
        let bytes = serde_json::to_vec_pretty(&build()).unwrap();
        let (url, task) = serve(envelope(&bytes, &sign(&pair, &bytes))).await;
        // The checker trusts the compiled-in key, which this test's pair is
        // not, so it reports the refusal: the news is checked as an install
        // would check it.
        let checker = Checker::new(String::new(), url.clone(), true);
        let status = checker.check(Some(Channel::Main), true).await;
        assert_eq!(status.channel, Channel::Main);
        assert!(status.latest.is_none());
        assert!(status.error.unwrap().contains("not signed"));
        assert_eq!(feed(&url, &public).await.unwrap().commit, "e".repeat(40));
        task.abort();
    }

    #[tokio::test]
    async fn news_about_one_channel_is_never_reported_as_the_other() {
        let checker = Checker::new("http://127.0.0.1:1".into(), "http://127.0.0.1:1".into(), true);
        {
            let mut cached = checker.cached.lock().await;
            cached.status.latest = Some(release());
            cached.status.checked_at = Some("a release check".into());
            cached.attempted = Some(Instant::now());
        }
        // Not refreshed and within the cache's life: only the channel moved.
        let moved = checker.check(Some(Channel::Main), false).await;
        assert_eq!(moved.channel, Channel::Main);
        assert!(moved.latest.is_none(), "a release is not a build of main");
        assert!(moved.checked_at.is_none());
        // An updater that cannot answer keeps the channel the box was on.
        assert_eq!(checker.check(None, false).await.channel, Channel::Main);
    }

    #[test]
    fn a_channel_is_one_of_two_words() {
        for (said, channel) in [
            (Some("mian"), None),
            (Some("Main"), None),
            (None, Some(Channel::Release)),
            (Some(""), Some(Channel::Release)),
            (Some("release"), Some(Channel::Release)),
            (Some(" main "), Some(Channel::Main)),
        ] {
            let result = Channel::named(said);
            assert_eq!(result.as_ref().ok().copied(), channel, "{said:?}");
            if let Err(error) = result {
                assert!(error.contains("release or main"), "{error}");
            }
        }
    }

    #[test]
    fn this_build_trusts_at_least_one_well_formed_main_key_and_it_is_not_a_release_key() {
        use base64::Engine;
        let parse = |file: &str| -> Vec<Vec<u8>> {
            file.lines()
                .map(str::trim)
                .filter(|line| !line.is_empty() && !line.starts_with('#'))
                .map(|line| base64::engine::general_purpose::STANDARD.decode(line).unwrap())
                .collect()
        };
        let main = parse(MAIN_KEYS);
        assert!(!main.is_empty());
        assert!(main.iter().all(|key| key.len() == 32));
        assert!(
            parse(KEYS).iter().all(|key| !main.contains(key)),
            "a key CI holds must not also sign releases"
        );
    }
}
