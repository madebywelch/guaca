//! `guaca-updater`: the process beside a box's host that can replace it.
//!
//! A host cannot update itself. A container cannot swap its own image; what
//! can is the Docker socket, which is root on the box, and guacad runs the
//! shell commands agents ask for, so it must never hold one. The socket goes
//! to a second container that runs nothing else, and guacad reaches it through
//! a Unix socket on a volume the two share.
//!
//! It can be asked two things: how the host is, and to install the latest
//! build on the box's channel whose manifest one of that channel's keys
//! signed. Not an image, not a command, not a restore, and not a channel: the
//! channel is the updater's own setting, fixed where the box was installed.
//! That list is what makes it safe to leave within reach of an agent's shell:
//! the most anything in the host can make it do is update the box to the
//! newest build Guaca published on the channel the box already follows, which
//! interrupts work and loses none.
//!
//! The sequence is the desktop's, in `host.rs`, run with a box's [`Spec`].
//! After it has updated the host it replaces itself from the same release, so
//! a fix to this file reaches a box without anybody logging in to it.

use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader};

use crate::host::{self, LocalHost, Operation, Reach, Running, Spec, Target};
use crate::updates::Channel;

/// The host's container.
pub const HOST: &str = "guacad";
/// The updater's container, while it is the one serving.
pub const UPDATER: &str = "guaca-updater";
/// An updater that has been replaced, until its replacement is serving and
/// removes it.
const RETIRING: &str = "guaca-updater-retiring";
const HOST_OWNER: &str = "bot.guaca.box";
const UPDATER_OWNER: &str = "bot.guaca.updater";
/// Where the journal lives: a volume only the updater mounts, so nothing in
/// the host can rewrite what an interrupted update left.
const STATE: &str = "guaca-updater";
const STATE_DIR: &str = "/var/lib/guaca-updater";
/// The volume the socket is on, mounted into both containers.
const SOCKET: &str = "guaca-updater-socket";
const SOCKET_DIR: &str = "/run/guaca-updater";

/// What a box is configured with, handed to the host by name.
pub const HOST_ENV: &[&str] = &[
    "GUACA_TOKEN",
    "GUACA_ORIGIN",
    "GUACA_UPDATE_CHECKS",
    "GUAC_LOG",
    "ANTHROPIC_API_KEY",
    "CODEX_API_KEY",
    "OPENAI_API_KEY",
    "OPENROUTER_API_KEY",
    "GH_TOKEN",
];
/// The updater's own settings. It carries these and the host's to the
/// updater that replaces it, which is the only way they survive one.
///
/// The channel is here and not in `HOST_ENV`: a host learns its environment
/// when it is made, and reinstalling replaces the updater and not the host,
/// so the host asks the updater which channel it follows instead.
const OWN_ENV: &[&str] = &["GUACA_PORT", "GUACA_VOLUME", "GUACA_CHANNEL"];

/// The one line guacad sends.
#[derive(Debug, Serialize, Deserialize)]
#[serde(tag = "ask", rename_all = "camelCase")]
pub enum Request {
    Status,
    /// Install the latest signed build on the box's channel, which has to be
    /// `version` from `commit`: the one the operator was shown and agreed to.
    /// Every build of `main` carries the same version, so on that channel the
    /// commit is the part that says which one, and it is required.
    Update {
        version: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        commit: Option<String>,
    },
}

/// How the box is, as the operator is shown it.
#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Report {
    /// What this box installs. Absent from an older updater's report, which
    /// only ever installed releases.
    #[serde(default)]
    pub channel: Channel,
    pub updating: bool,
    pub running: Option<Running>,
    pub operation: Option<Operation>,
    /// Why the updater could not start the host, or refused the last update
    /// before it began. A failure after that is in `operation`.
    pub error: Option<String>,
}

/// The host a box runs, as the updater makes it.
pub fn host_spec() -> Result<Spec, String> {
    let port = match std::env::var("GUACA_PORT") {
        Ok(port) if !port.trim().is_empty() => {
            port.trim().parse::<u16>().ok().filter(|p| *p > 0).ok_or_else(|| {
                format!("GUACA_PORT is {port:?}, which is not a port. Set it to the port this box's tunnel points at, or leave it out for 8787.")
            })?
        }
        _ => 8787,
    };
    let volume = std::env::var("GUACA_VOLUME")
        .ok()
        .map(|v| v.trim().to_string())
        .filter(|v| !v.is_empty())
        .unwrap_or_else(|| format!("{HOST}-data"));
    let nameable = volume.chars().all(|c| c.is_ascii_alphanumeric() || "_.-".contains(c))
        && volume.starts_with(|c: char| c.is_ascii_alphanumeric());
    if !nameable {
        return Err(format!("GUACA_VOLUME is {volume:?}, which Docker cannot name a volume."));
    }
    Ok(Spec {
        name: HOST.into(),
        owner: HOST_OWNER,
        volume,
        port: Some(port),
        env: HOST_ENV,
        socket: Some(SOCKET.into()),
        reach: Reach::Bridge,
    })
}

/// Where builds are read from, and the keys that have to have signed one.
#[derive(Clone)]
pub struct Source {
    pub channel: Channel,
    pub manifest: String,
    /// Where a release's signature is. A build of `main` carries its own.
    pub downloads: String,
    pub keys: String,
}

impl Source {
    pub fn of(channel: Channel) -> Self {
        match channel {
            Channel::Release => Self {
                channel,
                manifest: crate::updates::SOURCE.into(),
                downloads: crate::updates::DOWNLOADS.into(),
                keys: crate::updates::KEYS.into(),
            },
            Channel::Main => Self {
                channel,
                manifest: crate::updates::MAIN_FEED.into(),
                downloads: String::new(),
                keys: crate::updates::MAIN_KEYS.into(),
            },
        }
    }
}

pub struct Updater {
    host: LocalHost,
    source: Source,
    /// Held from the moment an update is accepted until it has finished,
    /// including the replacement of this updater.
    busy: AtomicBool,
    /// Why the host could not be started, or the last update never began.
    problem: parking_lot::Mutex<Option<String>>,
}

impl Updater {
    pub fn new(host: LocalHost, source: Source) -> Self {
        Self { host, source, busy: AtomicBool::new(false), problem: Default::default() }
    }

    pub async fn report(&self) -> Report {
        let (operation, unreadable) = match self.host.operation() {
            Ok(operation) => (operation, None),
            Err(error) => (None, Some(error)),
        };
        // An update, not the manager's lock: the lock is also held while the
        // host starts, and a box reported as updating then hides its button
        // and draws a restart that is not happening.
        let unfinished = operation.as_ref().is_some_and(|op| !op.stage.finished());
        Report {
            channel: self.source.channel,
            updating: self.busy.load(Ordering::SeqCst) || unfinished,
            running: self.host.running().await.ok().flatten(),
            operation,
            error: unreadable.or_else(|| self.problem.lock().clone()),
        }
    }

    /// Accepts an update and returns before it runs: the host this request
    /// came through is about to be stopped.
    pub async fn update(
        self: &Arc<Self>,
        version: String,
        commit: Option<String>,
    ) -> Result<Report, String> {
        if self.busy.swap(true, Ordering::SeqCst) {
            return Err("An update is already running. Wait for it to finish.".into());
        }
        self.problem.lock().take();
        let release = match self.release(&version, commit.as_deref()).await {
            Ok(release) => release,
            Err(error) => {
                self.busy.store(false, Ordering::SeqCst);
                return Err(error);
            }
        };
        let this = self.clone();
        tokio::spawn(async move {
            this.install(release).await;
            this.busy.store(false, Ordering::SeqCst);
        });
        Ok(self.report().await)
    }

    async fn release(
        &self,
        version: &str,
        commit: Option<&str>,
    ) -> Result<crate::updates::Release, String> {
        let Source { channel, manifest, downloads, keys } = &self.source;
        let release = match channel {
            Channel::Release => crate::updates::signed(manifest, downloads, keys).await?,
            Channel::Main => {
                if commit.is_none() {
                    return Err("This box follows main, so an update names the build to install. Update the Guaca app, then check for updates and try again.".into());
                }
                crate::updates::feed(manifest, keys).await?
            }
        };
        let latest = short(&release.commit);
        if release.version != version {
            return Err(match channel {
                Channel::Release => format!("The latest release is Guaca {}, not {version}. Check for updates and review it again.", release.version),
                Channel::Main => format!("Main is now at {latest}, which is Guaca {}, not {version}. Check for updates and review it again.", release.version),
            });
        }
        // Seven characters is the shortest spelling a person is shown, and a
        // prefix any shorter would match builds nobody reviewed.
        if let Some(commit) = commit {
            if commit.len() < 7 || !release.commit.starts_with(commit) {
                let reviewed = short(commit);
                return Err(match channel {
                    Channel::Release => format!("The latest release is Guaca {} from {latest}, not {reviewed}. Check for updates and review it again.", release.version),
                    Channel::Main => format!("Main is now at {latest}, not {reviewed}. Check for updates and review it again."),
                });
            }
        }
        Ok(release)
    }

    async fn install(&self, release: crate::updates::Release) {
        let commit = release.commit;
        let target = Target {
            image: release.image,
            version: release.version,
            api_generation: release.api_generation,
        };
        let before = self.journaled();
        match self.host.update(&target, None).await {
            Ok(_) => {
                tracing::info!(version = %target.version, %commit, "host updated");
                // Before this updater replaces itself, because the one that
                // replaces it removes this one, and whatever is after that
                // line may never run.
                let previous = self.host.operation().ok().flatten().map(|op| op.previous_image);
                let docker = self.host.docker_path();
                let own = own_image(docker).await.ok();
                let keep: Vec<&str> =
                    [Some(target.image.as_str()), previous.as_deref(), own.as_deref()]
                        .into_iter()
                        .flatten()
                        .collect();
                prune_images(docker, &keep).await;
                if let Err(error) = self.succeed(&target.image).await {
                    tracing::warn!(%error, "the host is updated; this updater could not replace itself");
                }
            }
            Err(error) => {
                tracing::warn!(%error, "host update did not finish");
                // Refused before the sequence began, the journal still holds
                // the last update, whose outcome is not this one's.
                if self.journaled() == before {
                    *self.problem.lock() = Some(error);
                }
            }
        }
    }

    fn journaled(&self) -> Option<String> {
        self.host.operation().ok().flatten().and_then(|op| serde_json::to_string(&op).ok())
    }

    /// Replaces this updater from the release the host now runs.
    async fn succeed(&self, image: &str) -> Result<(), String> {
        let docker = self.host.docker_path();
        if own_image(docker).await? == image {
            return Ok(());
        }
        install(docker, image).await
    }

    /// Starts the host, and keeps trying while it will not start. The reason
    /// is in every report until it does.
    async fn keep_started(&self) {
        loop {
            match self.host.start().await {
                Ok(_) => {
                    self.problem.lock().take();
                    return;
                }
                Err(error) => {
                    tracing::warn!(%error, "the host did not start; trying again in a minute");
                    *self.problem.lock() = Some(error);
                }
            }
            tokio::time::sleep(Duration::from_secs(60)).await;
        }
    }
}

/// A commit as the operator is shown one.
fn short(commit: &str) -> &str {
    commit.get(..7).unwrap_or(commit)
}

/// Where every host image a box installs comes from.
const REPOSITORY: &str = "ghcr.io/madebywelch/guaca/guacad";

/// Removes host images this box can no longer use: all but the one it runs
/// now and the one before, which is what the one kept backup restores with.
/// Docker refuses to remove an image a container still uses, and that one is
/// left. Only this repository's images are looked at; anything the operator
/// built or pulled under another name is theirs.
async fn prune_images(docker: &Path, keep: &[&str]) {
    let listed = match host::run_docker(
        docker,
        &[
            "image",
            "ls",
            "--no-trunc",
            "--digests",
            "--format",
            "{{.ID}} {{.Repository}}@{{.Digest}}",
            REPOSITORY,
        ],
        30,
    )
    .await
    {
        Ok(listed) => listed,
        Err(error) => {
            tracing::warn!(%error, "could not list host images; they were left");
            return;
        }
    };
    let images: Vec<(&str, &str)> =
        listed.lines().filter_map(|line| line.trim().split_once(' ')).collect();
    let kept: Vec<&str> =
        images.iter().filter(|(_, image)| keep.contains(image)).map(|(id, _)| *id).collect();
    let mut gone: Vec<&str> = Vec::new();
    for (id, _) in &images {
        if kept.contains(id) || gone.contains(id) {
            continue;
        }
        gone.push(id);
        match host::run_docker(docker, &["image", "rm", id], 60).await {
            Ok(_) => tracing::info!(image = id, "removed a host image this box no longer uses"),
            Err(error) => tracing::warn!(image = id, %error, "could not remove a host image"),
        }
    }
}

/// How every updater is labeled, whatever it is named now.
const MADE: (&str, &str) = (UPDATER_OWNER, UPDATER);

/// Removes an updater that was renamed aside for a replacement.
async fn retire(docker: &Path) -> Result<(), String> {
    if host::owned(docker, RETIRING, MADE).await?.is_some() {
        // With the anonymous volume the image's VOLUME line gave it, which
        // nothing mounts again and which would otherwise stay for good.
        host::run_docker(docker, &["rm", "--force", "--volumes", RETIRING], 60).await?;
        tracing::info!("removed the updater this one replaced");
    }
    Ok(())
}

/// The image the serving updater was made from, which is also what a box's
/// first host is made from.
async fn own_image(docker: &Path) -> Result<String, String> {
    let value = host::owned(docker, UPDATER, MADE).await?.ok_or(
        "This updater is not the one guaca-updater install made. Install it with deploy/box/install.sh.",
    )?;
    Ok(value["Config"]["Image"].as_str().unwrap_or_default().into())
}

/// Makes the updater container from `image`. One already there is renamed
/// rather than removed, and named back if its replacement does not start;
/// the replacement removes it once it is serving ([`serve`]).
pub async fn install(docker: &Path, image: &str) -> Result<(), String> {
    // Refused before the old updater is touched: the new one would refuse to
    // serve, and the box would keep an updater nobody can replace from here.
    Channel::from_env()?;
    retire(docker).await?;
    let existing = host::owned(docker, UPDATER, MADE).await?.is_some();
    if existing {
        host::run_docker(docker, &["rename", UPDATER, RETIRING], 30).await?;
    }
    let label = format!("{UPDATER_OWNER}={UPDATER}");
    let state = format!("type=volume,src={STATE},dst={STATE_DIR}");
    let socket = format!("type=volume,src={SOCKET},dst={SOCKET_DIR}");
    let mut args = vec![
        "run",
        "--detach",
        "--name",
        UPDATER,
        "--label",
        &label,
        "--init",
        "--restart",
        "unless-stopped",
        // The image's check asks the host's port, which this is not.
        "--no-healthcheck",
        "--user",
        "0",
        "--mount",
        "type=bind,src=/var/run/docker.sock,dst=/var/run/docker.sock",
        "--mount",
        &state,
        "--mount",
        &socket,
        "--entrypoint",
        "/usr/local/bin/guaca-updater",
    ];
    for name in HOST_ENV.iter().chain(OWN_ENV) {
        if std::env::var_os(name).is_some() {
            args.extend(["--env", *name]);
        }
    }
    args.push(image);
    let made = host::run_docker(docker, &args, 60).await;
    if made.is_err() && existing {
        if let Err(error) = host::run_docker(docker, &["rename", RETIRING, UPDATER], 30).await {
            tracing::error!(%error, "could not put the previous updater's name back");
        }
    }
    made.map(drop)
}

/// Serves until the process is stopped.
///
/// In this order: bind, retire the updater this one replaced, start the host,
/// answer. Binding first means an updater that cannot serve never removes the
/// one that can.
#[cfg(unix)]
pub async fn serve() -> Result<(), String> {
    let docker = host::docker_binary();
    let image = own_image(&docker).await?;
    let spec = host_spec()?;
    // Refused here, before the bind, so a box installed with a channel nobody
    // can read keeps the updater it had rather than one that guesses.
    let channel = Channel::from_env()?;
    let listener = bind(Path::new(host::UPDATER_SOCKET))?;
    // Past the bind this updater is the one the host reaches, so nothing
    // after it may stop it: an updater that exits here strands the box.
    if let Err(error) = retire(&docker).await {
        tracing::warn!(%error, "the updater this one replaced is still there");
    }
    let host = LocalHost::with_spec(spec, &image)
        .with_journal(Path::new(STATE_DIR).join("host-update.json"));
    let updater = Arc::new(Updater::new(host, Source::of(channel)));
    let starting = updater.clone();
    tokio::spawn(async move { starting.keep_started().await });
    tracing::info!(%image, ?channel, socket = host::UPDATER_SOCKET, "guaca-updater ready");
    listen(listener, updater).await
}

#[cfg(unix)]
fn bind(path: &Path) -> Result<tokio::net::UnixListener, String> {
    use std::os::unix::fs::PermissionsExt;
    // A socket left by the updater this one replaces, or by a crash.
    let _ = std::fs::remove_file(path);
    let listener = tokio::net::UnixListener::bind(path)
        .map_err(|e| format!("Could not listen on {}: {e}", path.display()))?;
    // The host connects as its own unprivileged user. The volume is mounted
    // into nothing else.
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o666))
        .map_err(|e| format!("Could not open {} to the host: {e}", path.display()))?;
    Ok(listener)
}

#[cfg(unix)]
pub async fn listen(
    listener: tokio::net::UnixListener,
    updater: Arc<Updater>,
) -> Result<(), String> {
    loop {
        let (stream, _) = listener.accept().await.map_err(|e| format!("The socket failed: {e}"))?;
        let updater = updater.clone();
        tokio::spawn(async move {
            if let Err(error) = answer(stream, &updater).await {
                tracing::debug!(%error, "a request to the updater went unanswered");
            }
        });
    }
}

/// The longest request line read. Both requests fit in a hundred bytes.
const MOST_REQUEST: u64 = 4096;
/// The longest answer read. A report is a few hundred bytes.
const MOST_ANSWER: u64 = 64 * 1024;

#[cfg(unix)]
async fn answer(stream: tokio::net::UnixStream, updater: &Arc<Updater>) -> std::io::Result<()> {
    let (read, mut write) = stream.into_split();
    let mut line = String::new();
    // Anything in the host can connect, so a connection that never finishes
    // its line is closed rather than held.
    tokio::time::timeout(
        Duration::from_secs(10),
        BufReader::new(read.take(MOST_REQUEST)).read_line(&mut line),
    )
    .await
    .map_err(|_| std::io::Error::from(std::io::ErrorKind::TimedOut))??;
    let reply = match serde_json::from_str::<Request>(&line) {
        Ok(Request::Status) => Ok(updater.report().await),
        Ok(Request::Update { version, commit }) => updater.update(version, commit).await,
        Err(_) => Err("The updater did not understand the request. Update the host.".into()),
    };
    let value = match reply {
        Ok(report) => json!({ "ok": report }),
        Err(message) => json!({ "err": message }),
    };
    write.write_all(format!("{value}\n").as_bytes()).await?;
    write.shutdown().await
}

/// Asks the box's updater one question, from the host.
#[cfg(unix)]
pub async fn ask(socket: &Path, request: &Request) -> Result<Value, String> {
    let unreachable = |e: std::io::Error| {
        format!("The updater on this box is not answering ({e}). Restart the guaca-updater container on the box, or reinstall it with deploy/box/install.sh.")
    };
    let exchange = async {
        let mut stream = tokio::net::UnixStream::connect(socket).await.map_err(unreachable)?;
        let line = serde_json::to_string(request).map_err(|e| e.to_string())?;
        stream.write_all(format!("{line}\n").as_bytes()).await.map_err(unreachable)?;
        let mut answer = String::new();
        BufReader::new(stream.take(MOST_ANSWER))
            .read_line(&mut answer)
            .await
            .map_err(unreachable)?;
        let value: Value = serde_json::from_str(&answer)
            .map_err(|_| "The updater gave an answer the host cannot read.".to_string())?;
        match (value.get("ok"), value["err"].as_str()) {
            (Some(ok), _) => Ok(ok.clone()),
            (None, Some(message)) => Err(message.to_string()),
            (None, None) => Err("The updater gave an answer the host cannot read.".into()),
        }
    };
    // Long enough for a release lookup, which is the slow half of an update
    // request; the update itself runs after the answer.
    tokio::time::timeout(Duration::from_secs(30), exchange)
        .await
        .map_err(|_| "The updater on this box took too long to answer. Try again.".to_string())?
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;

    fn calls(dir: &tempfile::TempDir) -> Vec<Vec<String>> {
        std::fs::read_to_string(dir.path().join("docker-calls.jsonl"))
            .unwrap_or_default()
            .lines()
            .map(|line| serde_json::from_str(line).unwrap())
            .collect()
    }

    fn state(dir: &tempfile::TempDir) -> Value {
        serde_json::from_slice(&std::fs::read(dir.path().join("docker-state.json")).unwrap())
            .unwrap()
    }

    fn write(dir: &tempfile::TempDir, value: Value) {
        std::fs::write(dir.path().join("docker-state.json"), value.to_string()).unwrap();
    }

    /// A box with a host, an updater, a release server and the key it signs with.
    struct Rig {
        dir: tempfile::TempDir,
        updater: Arc<Updater>,
        tasks: Vec<tokio::task::JoinHandle<()>>,
    }

    impl Drop for Rig {
        fn drop(&mut self) {
            for task in &self.tasks {
                task.abort();
            }
        }
    }

    async fn a_box(failure: &str, signed_by_us: bool) -> Rig {
        a_box_on(Channel::Release, failure, signed_by_us).await
    }

    /// The commit the rig's build of `main` was made from.
    const TIP: &str = "e0e0e0e0e0e0e0e0e0e0e0e0e0e0e0e0e0e0e0e0";

    async fn a_box_on(channel: Channel, failure: &str, signed_by_us: bool) -> Rig {
        use base64::Engine;
        use ring::signature::KeyPair;
        let version = env!("CARGO_PKG_VERSION");
        let health = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = health.local_addr().unwrap().port();
        let app = axum::Router::new()
            .route("/health", axum::routing::get(move || async move {
                axum::Json(json!({"service":"guacad","build":"abcdef1","version":version,"apiGeneration":1}))
            }))
            .route("/v1/call", axum::routing::post(|| async { axum::Json(json!({"ok":{}})) }));
        let mut tasks = vec![tokio::spawn(async move { axum::serve(health, app).await.unwrap() })];

        let image = format!("ghcr.io/madebywelch/guaca/guacad@sha256:{}", "c".repeat(64));
        let manifest = serde_json::to_vec_pretty(&match channel {
            Channel::Release => json!({
                "schema":1, "version":version, "commit":"a".repeat(40), "image":image,
                "apiGeneration":1, "clientMinimum":1, "clientMaximum":1,
                "notes":format!("https://github.com/madebywelch/guaca/releases/tag/v{version}"),
            }),
            Channel::Main => json!({
                "schema":1, "channel":"main", "version":version, "commit":TIP, "image":image,
                "apiGeneration":1, "clientMinimum":1, "clientMaximum":1,
                "notes":format!("https://github.com/madebywelch/guaca/commit/{TIP}"),
            }),
        })
        .unwrap();
        let random = ring::rand::SystemRandom::new();
        let ours = ring::signature::Ed25519KeyPair::from_pkcs8(
            ring::signature::Ed25519KeyPair::generate_pkcs8(&random).unwrap().as_ref(),
        )
        .unwrap();
        let theirs = ring::signature::Ed25519KeyPair::from_pkcs8(
            ring::signature::Ed25519KeyPair::generate_pkcs8(&random).unwrap().as_ref(),
        )
        .unwrap();
        let signer = if signed_by_us { &ours } else { &theirs };
        let signature =
            base64::engine::general_purpose::STANDARD.encode(signer.sign(&manifest).as_ref());
        let keys = base64::engine::general_purpose::STANDARD.encode(ours.public_key().as_ref());
        let envelope = serde_json::to_vec(&json!({
            "manifest": base64::engine::general_purpose::STANDARD.encode(&manifest),
            "signature": signature.clone(),
        }))
        .unwrap();
        let releases = axum::Router::new()
            .route("/latest", axum::routing::get(move || async move { manifest }))
            .route(
                &format!("/v{version}/guaca-release.json.sig"),
                axum::routing::get(move || async move { signature }),
            )
            .route("/main", axum::routing::get(move || async move { envelope }));
        let socket = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let base = format!("http://{}", socket.local_addr().unwrap());
        tasks.push(tokio::spawn(async move { axum::serve(socket, releases).await.unwrap() }));

        let dir = tempfile::tempdir().unwrap();
        let docker = crate::host::tests::fixture(&dir);
        let spec = Spec { port: Some(port), ..host_spec().unwrap() };
        write(
            &dir,
            json!({
                "failure": failure, "exists": true, "image_version": version, "name": HOST,
                "container": {
                    "Mounts":[{"Destination":"/var/lib/guaca","Type":"volume","Name":spec.volume.clone()}],
                    "Config":{"Image":"guacad:old","Labels":{HOST_OWNER:HOST}},
                    "State":{"Running":true},
                    "NetworkSettings":{
                        "Ports":{"8787/tcp":[{"HostIp":"127.0.0.1","HostPort":port.to_string()}]},
                        "Networks":{"bridge":{"IPAddress":"127.0.0.1"}}
                    }
                },
                "others": {UPDATER: {"Config":{"Image":"guacad:old","Labels":{UPDATER_OWNER:UPDATER}}}}
            }),
        );
        let host = LocalHost::with_spec(Spec { reach: Reach::Published, ..spec }, "guacad:old")
            .with_journal(dir.path().join("host-update.json"))
            .with_docker(docker);
        let source = match channel {
            Channel::Release => {
                Source { channel, manifest: format!("{base}/latest"), downloads: base, keys }
            }
            Channel::Main => {
                Source { channel, manifest: format!("{base}/main"), downloads: String::new(), keys }
            }
        };
        Rig { dir, updater: Arc::new(Updater::new(host, source)), tasks }
    }

    async fn settled(updater: &Updater) -> Report {
        for _ in 0..200 {
            let report = updater.report().await;
            if !report.updating {
                return report;
            }
            tokio::time::sleep(Duration::from_millis(25)).await;
        }
        panic!("the update did not finish");
    }

    #[test]
    fn a_box_is_published_to_loopback_and_its_settings_are_refused_when_wrong() {
        let spec = host_spec().unwrap();
        assert_eq!(
            (spec.name.as_str(), spec.port, spec.volume.as_str()),
            (HOST, Some(8787), "guacad-data")
        );
        assert_eq!(spec.socket.as_deref(), Some(SOCKET));
        for own in OWN_ENV {
            assert!(!HOST_ENV.contains(own), "{own} is the updater's setting, not the host's");
        }
    }

    #[tokio::test]
    async fn an_update_installs_the_signed_release_and_then_replaces_the_updater() {
        let bx = a_box("", true).await;
        let version = env!("CARGO_PKG_VERSION").to_string();
        let accepted = bx.updater.update(version, None).await.unwrap();
        assert!(accepted.updating);
        let report = settled(&bx.updater).await;
        let op = report.operation.unwrap();
        assert_eq!(op.stage, host::Stage::Updated, "{:?}", op.error);
        let now = state(&bx.dir);
        assert!(now["container"]["Config"]["Image"].as_str().unwrap().contains("@sha256:"));
        // The host is made with the socket and never with a value in its arguments.
        let made = now["created"].as_array().unwrap();
        assert!(made
            .iter()
            .any(|a| a.as_str().unwrap().contains("dst=/run/guaca-updater,readonly")));
        assert!(!made.iter().any(|a| a.as_str().unwrap().contains("GUACA_TOKEN=")));
        // Then the updater: renamed aside, made again from the release, never removed by itself.
        let log = calls(&bx.dir);
        let renamed = log
            .iter()
            .position(|c| c.iter().map(String::as_str).eq(["rename", UPDATER, RETIRING]))
            .unwrap();
        let remade =
            log.iter().position(|c| c[0] == "run" && c.contains(&UPDATER.to_string())).unwrap();
        assert!(renamed < remade);
        let updater = &log[remade];
        assert!(
            updater.contains(&"type=bind,src=/var/run/docker.sock,dst=/var/run/docker.sock".into())
        );
        assert_eq!(updater.last(), made.last().and_then(|a| a.as_str()).map(String::from).as_ref());
        assert!(!log.iter().any(|c| c[0] == "rm" && c.contains(&UPDATER.to_string())));
    }

    #[test]
    fn the_compose_host_and_a_box_read_the_same_settings() {
        let compose = include_str!("../../docker-compose.yml");
        let names: Vec<&str> = compose
            .lines()
            .skip_while(|line| line.trim() != "environment:")
            .skip(1)
            .take_while(|line| line.starts_with("      "))
            .map(str::trim)
            .filter(|line| !line.starts_with('#'))
            .filter_map(|line| line.split_once(':').map(|(name, _)| name))
            .collect();
        assert!(names.contains(&"GUACA_TOKEN"), "read the Compose file's settings: {names:?}");
        for name in names {
            assert!(HOST_ENV.contains(&name), "{name} reaches a Compose host and not a box's");
        }
    }

    #[tokio::test]
    async fn a_box_is_updating_while_an_update_is_unfinished_and_not_otherwise() {
        let bx = a_box("", true).await;
        assert!(!bx.updater.report().await.updating);
        let journal = bx.dir.path().join("host-update.json");
        for (stage, updating) in [
            ("Backing up workspace", true),
            ("Restoring previous version", true),
            ("Host updated", false),
            ("Recovery needed", false),
        ] {
            std::fs::write(
                &journal,
                json!({"stage":stage,"backup":null,"previousImage":"old","targetImage":"new","error":null}).to_string(),
            )
            .unwrap();
            assert_eq!(bx.updater.report().await.updating, updating, "{stage}");
        }
    }

    #[tokio::test]
    async fn a_release_nobody_trusted_signed_is_refused_before_anything_stops() {
        let bx = a_box("", false).await;
        let error = bx.updater.update(env!("CARGO_PKG_VERSION").into(), None).await.err().unwrap();
        assert!(error.contains("nothing was installed"), "{error}");
        assert!(!calls(&bx.dir).iter().any(|c| c[0] == "stop" || c[0] == "pull"));
        assert!(!bx.updater.report().await.updating, "a refusal frees the updater");
    }

    #[tokio::test]
    async fn an_update_is_to_the_release_the_operator_reviewed() {
        let bx = a_box("", true).await;
        let error = bx.updater.update("9.9.9".into(), None).await.err().unwrap();
        assert!(error.contains("review it again"), "{error}");
        assert!(!calls(&bx.dir).iter().any(|c| c[0] == "stop"));
    }

    #[tokio::test]
    async fn a_second_update_while_one_runs_is_refused() {
        let bx = a_box("", true).await;
        let version = env!("CARGO_PKG_VERSION").to_string();
        bx.updater.update(version.clone(), None).await.unwrap();
        let error = bx.updater.update(version, None).await.err().unwrap();
        assert!(error.contains("already running"), "{error}");
        settled(&bx.updater).await;
    }

    #[tokio::test]
    async fn a_failed_update_is_undone_and_the_updater_stays_as_it_was() {
        let bx = a_box("", true).await;
        let mut value = state(&bx.dir);
        value["refuse"] =
            json!(format!("ghcr.io/madebywelch/guaca/guacad@sha256:{}", "c".repeat(64)));
        write(&bx.dir, value);
        bx.updater.update(env!("CARGO_PKG_VERSION").into(), None).await.unwrap();
        let report = settled(&bx.updater).await;
        let op = report.operation.unwrap();
        assert_eq!(op.stage, host::Stage::Restored, "{:?}", op.error);
        assert_eq!(state(&bx.dir)["container"]["Config"]["Image"], "guacad:old");
        assert!(
            !calls(&bx.dir).iter().any(|c| c[0] == "rename"),
            "a failed update keeps its updater"
        );
    }

    #[tokio::test]
    async fn an_update_refused_before_it_began_is_not_reported_as_the_last_one() {
        let bx = a_box("", true).await;
        let earlier = json!({"stage":"Host updated","backup":"guacad-backup-0","previousImage":"older",
            "targetImage":"guacad:old","targetVersion":"0.1.0","error":null});
        std::fs::write(bx.dir.path().join("host-update.json"), earlier.to_string()).unwrap();
        // Another manager holds the host, so the sequence never begins.
        let held = std::fs::File::create(bx.dir.path().join("host-update.lock")).unwrap();
        held.try_lock().unwrap();
        bx.updater.update(env!("CARGO_PKG_VERSION").into(), None).await.unwrap();
        let report = settled(&bx.updater).await;
        let op = report.operation.unwrap();
        assert_eq!((op.stage, op.target_version.as_str()), (host::Stage::Updated, "0.1.0"));
        let said = report.error.unwrap();
        assert!(said.contains("Another Guaca process"), "{said}");
        drop(held);
        bx.updater.update(env!("CARGO_PKG_VERSION").into(), None).await.unwrap();
        let report = settled(&bx.updater).await;
        assert!(report.error.is_none(), "a new attempt clears the last refusal");
        assert_eq!(report.operation.unwrap().stage, host::Stage::Updated);
    }

    #[tokio::test]
    async fn the_replacement_removes_the_updater_it_replaced_and_nothing_else() {
        let bx = a_box("", true).await;
        let docker = bx.updater.host.docker_path().to_path_buf();
        install(&docker, "guacad:new").await.unwrap();
        assert!(state(&bx.dir)["others"][RETIRING].is_object(), "renamed aside, not removed");
        retire(&docker).await.unwrap();
        assert!(
            calls(&bx.dir).iter().any(|c| c.iter().map(String::as_str).eq([
                "rm",
                "--force",
                "--volumes",
                RETIRING
            ])),
            "its anonymous volume goes with it"
        );
        let now = state(&bx.dir);
        assert!(now["others"][RETIRING].is_null());
        assert_eq!(now["others"][UPDATER]["Config"]["Image"], "guacad:new");
        assert_eq!(now["exists"], true, "the host is not the updater's to remove");
        // Something else renamed to that name is somebody else's.
        let mut value = now;
        value["others"][RETIRING] = json!({"Config":{"Image":"x","Labels":{}}});
        write(&bx.dir, value);
        assert!(retire(&docker).await.is_err());
        assert!(state(&bx.dir)["others"][RETIRING].is_object());
    }

    #[tokio::test]
    async fn install_puts_the_previous_updater_back_when_its_replacement_will_not_start() {
        let bx = a_box("", true).await;
        let mut value = state(&bx.dir);
        value["refuse"] = json!("broken:image");
        write(&bx.dir, value);
        let docker = bx.updater.host.docker_path().to_path_buf();
        assert!(install(&docker, "broken:image").await.is_err());
        let log = calls(&bx.dir);
        let renames: Vec<_> = log.iter().filter(|c| c[0] == "rename").collect();
        assert_eq!(renames.len(), 2);
        assert_eq!(renames[1][1..], [RETIRING.to_string(), UPDATER.to_string()]);
        assert!(state(&bx.dir)["others"][UPDATER].is_object());
    }

    #[tokio::test]
    async fn the_host_asks_over_the_socket_and_reads_the_answer() {
        let bx = a_box("", true).await;
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("updater.sock");
        let listener = bind(&path).unwrap();
        use std::os::unix::fs::PermissionsExt;
        assert_eq!(std::fs::metadata(&path).unwrap().permissions().mode() & 0o777, 0o666);
        let serving = tokio::spawn(listen(listener, bx.updater.clone()));
        let report = ask(&path, &Request::Status).await.unwrap();
        assert_eq!(report["running"]["image"], "guacad:old");
        assert_eq!(report["updating"], false);
        let refused = ask(&path, &Request::Update { version: "9.9.9".into(), commit: None })
            .await
            .unwrap_err();
        assert!(refused.contains("review it again"), "{refused}");
        // A connection that never sends its line is let go, and the next is
        // answered. The clock is paused, so the runtime skips the wait.
        tokio::time::pause();
        let mut silent = tokio::net::UnixStream::connect(&path).await.unwrap();
        let mut nothing = Vec::new();
        assert_eq!(silent.read_to_end(&mut nothing).await.unwrap(), 0, "closed, unanswered");
        tokio::time::resume();
        assert!(ask(&path, &Request::Status).await.is_ok());
        serving.abort();
        let gone = ask(&dir.path().join("nobody.sock"), &Request::Status).await.unwrap_err();
        assert!(gone.contains("not answering"), "{gone}");
    }

    #[tokio::test]
    async fn a_box_on_main_refuses_any_build_but_the_one_reviewed_before_anything_stops() {
        let bx = a_box_on(Channel::Main, "", true).await;
        let version = env!("CARGO_PKG_VERSION").to_string();
        for (commit, said) in [
            (None, "names the build"),
            (Some("f".repeat(40)), "review it again"),
            (Some(TIP[..6].to_string()), "review it again"),
            (Some(String::new()), "review it again"),
        ] {
            let error = bx.updater.update(version.clone(), commit.clone()).await.err().unwrap();
            assert!(error.contains(said), "{commit:?}: {error}");
            assert!(!bx.updater.report().await.updating, "a refusal frees the updater");
        }
        let error = bx.updater.update("9.9.9".into(), Some(TIP.into())).await.err().unwrap();
        assert!(error.contains("review it again"), "{error}");
        assert!(!calls(&bx.dir).iter().any(|c| c[0] == "stop" || c[0] == "pull"));
    }

    #[tokio::test]
    async fn a_box_on_main_installs_the_reviewed_build_and_says_which_channel_it_follows() {
        let bx = a_box_on(Channel::Main, "", true).await;
        assert_eq!(bx.updater.report().await.channel, Channel::Main);
        let version = env!("CARGO_PKG_VERSION").to_string();
        bx.updater.update(version, Some(TIP[..7].into())).await.unwrap();
        let op = settled(&bx.updater).await.operation.unwrap();
        assert_eq!(op.stage, host::Stage::Updated, "{:?}", op.error);
        assert!(state(&bx.dir)["container"]["Config"]["Image"]
            .as_str()
            .unwrap()
            .contains("@sha256:"));
    }

    #[tokio::test]
    async fn a_box_on_main_refuses_a_build_its_key_did_not_sign() {
        let bx = a_box_on(Channel::Main, "", false).await;
        let error = bx
            .updater
            .update(env!("CARGO_PKG_VERSION").into(), Some(TIP.into()))
            .await
            .err()
            .unwrap();
        assert!(error.contains("nothing was installed"), "{error}");
        assert!(!calls(&bx.dir).iter().any(|c| c[0] == "stop" || c[0] == "pull"));
    }

    #[tokio::test]
    async fn a_release_box_holds_a_named_commit_to_the_release() {
        let bx = a_box("", true).await;
        let version = env!("CARGO_PKG_VERSION").to_string();
        let error = bx.updater.update(version.clone(), Some(TIP.into())).await.err().unwrap();
        assert!(error.contains("latest release"), "{error}");
        bx.updater.update(version, Some("a".repeat(40))).await.unwrap();
        settled(&bx.updater).await;
    }

    #[tokio::test]
    async fn a_finished_update_keeps_the_images_it_could_restore_to_and_no_others() {
        let bx = a_box("", true).await;
        let target = format!("ghcr.io/madebywelch/guaca/guacad@sha256:{}", "c".repeat(64));
        let mut value = state(&bx.dir);
        value["images"] = json!([
            {"id": "sha256:older", "ref": "ghcr.io/madebywelch/guaca/guacad@sha256:older"},
            {"id": "sha256:target", "ref": target},
            {"id": "sha256:busy", "ref": "ghcr.io/madebywelch/guaca/guacad@sha256:busy"},
        ]);
        value["in_use"] = json!(["sha256:busy"]);
        write(&bx.dir, value);
        bx.updater.update(env!("CARGO_PKG_VERSION").into(), None).await.unwrap();
        let op = settled(&bx.updater).await.operation.unwrap();
        assert_eq!(op.stage, host::Stage::Updated, "{:?}", op.error);
        let left: Vec<String> = state(&bx.dir)["images"]
            .as_array()
            .unwrap()
            .iter()
            .map(|image| image["id"].as_str().unwrap().to_string())
            .collect();
        assert_eq!(left, ["sha256:target", "sha256:busy"], "an image in use is refused and left");
        let log = calls(&bx.dir);
        let listed = log.iter().position(|c| c[..2] == ["image", "ls"]).unwrap();
        assert!(log[listed].contains(&REPOSITORY.to_string()), "only this repository is looked at");
        let replaced =
            log.iter().position(|c| c[0] == "run" && c.contains(&UPDATER.to_string())).unwrap();
        assert!(listed < replaced, "pruned before this updater is replaced");
    }

    #[tokio::test]
    async fn a_failed_update_removes_no_image() {
        let bx = a_box("", true).await;
        let mut value = state(&bx.dir);
        value["refuse"] =
            json!(format!("ghcr.io/madebywelch/guaca/guacad@sha256:{}", "c".repeat(64)));
        value["images"] =
            json!([{"id": "sha256:older", "ref": "ghcr.io/madebywelch/guaca/guacad@sha256:older"}]);
        write(&bx.dir, value);
        bx.updater.update(env!("CARGO_PKG_VERSION").into(), None).await.unwrap();
        settled(&bx.updater).await;
        assert!(!calls(&bx.dir).iter().any(|c| c[..2] == ["image", "rm"]));
    }

    #[test]
    fn a_host_from_before_channels_still_asks_in_words_the_updater_reads() {
        let old: Request = serde_json::from_str(r#"{"ask":"update","version":"0.2.0"}"#).unwrap();
        assert!(matches!(old, Request::Update { commit: None, .. }));
        let said =
            serde_json::to_string(&Request::Update { version: "0.2.0".into(), commit: None })
                .unwrap();
        assert_eq!(said, r#"{"ask":"update","version":"0.2.0"}"#, "an older updater reads this");
        let report: Report = serde_json::from_str(
            r#"{"updating":false,"running":null,"operation":null,"error":null}"#,
        )
        .unwrap();
        assert_eq!(report.channel, Channel::Release, "an older updater only installed releases");
    }
}
